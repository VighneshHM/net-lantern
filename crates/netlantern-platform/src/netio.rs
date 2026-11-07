use std::net::Ipv4Addr;

use async_trait::async_trait;

use crate::PlatformError;

/// The parts of an HTTP response the scanner cares about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpResponse {
    pub status: u16,
    /// The start of the response body (the real implementation caps its length).
    pub body_prefix: String,
}

/// The OS seam for application-layer lookups. The Windows implementation
/// arrives in phase 1; [`crate::sim::SimNetwork`] stands in for tests and `--demo`.
#[async_trait]
pub trait NetIo: Send + Sync {
    /// Resolve `name` to IPv4 addresses by asking the DNS `server` directly.
    async fn dns_resolve(
        &self,
        server: Ipv4Addr,
        name: &str,
    ) -> Result<Vec<Ipv4Addr>, PlatformError>;

    /// Plain HTTP GET. With `follow_redirects` false a 3xx is returned as-is,
    /// which is how captive portals are detected.
    async fn http_get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PlatformError>;
}
