use std::time::Duration;

use reqwest::redirect::Policy;

use crate::netio::HttpResponse;
use crate::PlatformError;

const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_PREFIX_LEN: usize = 16 * 1024;

fn io_error(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Io {
        message: e.to_string(),
    }
}

const MESSAGE_LIMIT: usize = 100;
const MAX_REDIRECTS: usize = 5;
// Some CDNs (www.microsoft.com's among them) answer requests without a
// User-Agent with 403 and then stall them until the timeout.
const USER_AGENT: &str = concat!("netlantern/", env!("CARGO_PKG_VERSION"));

/// The error's own message plus its innermost cause, cut to 100 characters,
/// so a failure says why it happened, not just which request failed.
fn with_root_cause(err: &(dyn std::error::Error + 'static)) -> PlatformError {
    let mut message = err.to_string();
    if let Some(mut root) = err.source() {
        while let Some(next) = root.source() {
            root = next;
        }
        message.push_str(": ");
        message.extend(root.to_string().chars().take(MESSAGE_LIMIT));
    }
    PlatformError::Io { message }
}

/// Walks the error chain looking for a rejected certificate and returns the
/// rustls message, cut to 100 characters.
fn is_certificate_error(err: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        let tls = e.downcast_ref::<rustls::Error>().or_else(|| {
            // io::Error::source() skips its payload, so unwrap nested io errors by hand.
            let mut inner = e.downcast_ref::<std::io::Error>()?.get_ref()?;
            loop {
                if let Some(tls) = inner.downcast_ref::<rustls::Error>() {
                    return Some(tls);
                }
                inner = inner.downcast_ref::<std::io::Error>()?.get_ref()?;
            }
        });
        if let Some(tls @ rustls::Error::InvalidCertificate(_)) = tls {
            return Some(tls.to_string().chars().take(MESSAGE_LIMIT).collect());
        }
        current = e.source();
    }
    None
}

/// A request error as a `PlatformError`. The URL is dropped from the message
/// so the root cause isn't crowded out by it.
fn send_error(e: reqwest::Error) -> PlatformError {
    match is_certificate_error(&e) {
        Some(message) => PlatformError::TlsCertificate { message },
        None => with_root_cause(&e.without_url()),
    }
}

pub(super) async fn http_get(
    url: &str,
    follow_redirects: bool,
) -> Result<HttpResponse, PlatformError> {
    let policy = if follow_redirects {
        // Follow only while the target host and port equal the original's, so a
        // LAN device cannot bounce us onto the internet.
        Policy::custom(|attempt| {
            let same_origin = attempt.previous().first().is_some_and(|first| {
                first.host_str() == attempt.url().host_str()
                    && first.port_or_known_default() == attempt.url().port_or_known_default()
            });
            if attempt.previous().len() > MAX_REDIRECTS || !same_origin {
                attempt.stop()
            } else {
                attempt.follow()
            }
        })
    } else {
        Policy::none()
    };
    // No proxy: proxy environment variables must never reroute LAN requests.
    let client = reqwest::Client::builder()
        .no_proxy()
        .user_agent(USER_AGENT)
        .redirect(policy)
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(io_error)?;
    let mut response = client.get(url).send().await.map_err(send_error)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());

    let mut body = Vec::new();
    while body.len() < BODY_PREFIX_LEN {
        match response.chunk().await.map_err(send_error)? {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }
    body.truncate(BODY_PREFIX_LEN);
    Ok(HttpResponse {
        status,
        location,
        body_prefix: String::from_utf8_lossy(&body).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_errors_are_found_inside_io_errors() {
        let tls = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
        let io = std::io::Error::new(std::io::ErrorKind::InvalidData, tls);
        assert!(is_certificate_error(&io).unwrap().contains("UnknownIssuer"));
        let other = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert_eq!(is_certificate_error(&other), None);
        let nested = std::io::Error::other(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
        ));
        assert!(is_certificate_error(&nested).unwrap().contains("Expired"));
        let not_cert = rustls::Error::General("boom".into());
        assert_eq!(is_certificate_error(&not_cert), None);
    }

    #[derive(Debug)]
    struct Wrapper(std::io::Error);

    impl std::fmt::Display for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("error sending request")
        }
    }

    impl std::error::Error for Wrapper {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    fn message(err: PlatformError) -> String {
        match err {
            PlatformError::Io { message } => message,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn io_messages_carry_the_root_cause_cut_to_100_chars() {
        let timed_out = Wrapper(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "operation timed out",
        ));
        assert_eq!(
            message(with_root_cause(&timed_out)),
            "error sending request: operation timed out"
        );
        let long = Wrapper(std::io::Error::other("é".repeat(150)));
        let expected = format!("error sending request: {}", "é".repeat(100));
        assert_eq!(message(with_root_cause(&long)), expected);
        let alone = std::io::Error::other("refused");
        assert_eq!(message(with_root_cause(&alone)), "refused");
    }

    #[tokio::test]
    async fn request_errors_leave_out_the_url() {
        // Port 1 on loopback is closed, so the connect is refused at once.
        let url = "http://127.0.0.1:1/a-long-path-that-would-crowd-out-the-cause";
        let err = reqwest::Client::new().get(url).send().await.unwrap_err();
        let text = message(send_error(err));
        assert!(!text.contains("a-long-path"), "{text}");
        assert!(!text.contains("127.0.0.1:1"), "{text}");
    }
}
