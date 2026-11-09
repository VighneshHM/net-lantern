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

fn send_error(e: reqwest::Error) -> PlatformError {
    match is_certificate_error(&e) {
        Some(message) => PlatformError::TlsCertificate { message },
        None => io_error(e),
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
        match response.chunk().await.map_err(io_error)? {
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
}
