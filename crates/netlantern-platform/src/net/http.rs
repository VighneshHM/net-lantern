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

pub(super) async fn http_get(
    url: &str,
    follow_redirects: bool,
) -> Result<HttpResponse, PlatformError> {
    let policy = if follow_redirects {
        Policy::limited(5)
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
    let mut response = client.get(url).send().await.map_err(io_error)?;
    let status = response.status().as_u16();

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
        body_prefix: String::from_utf8_lossy(&body).into_owned(),
    })
}
