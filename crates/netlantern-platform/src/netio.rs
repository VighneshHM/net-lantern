use std::net::Ipv4Addr;

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::PlatformError;

/// The parts of an HTTP response the scanner cares about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpResponse {
    pub status: u16,
    /// The `Location` header, set on redirects.
    pub location: Option<String>,
    /// The start of the response body (the real implementation caps its length).
    pub body_prefix: String,
}

/// One mDNS service instance seen on the LAN.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdnsService {
    pub ip: Ipv4Addr,
    /// Service type, e.g. `_ipp._tcp`.
    pub service_type: String,
    /// Instance name, e.g. `Office printer`.
    pub instance: String,
    /// Host name from the SRV record, e.g. `brn3c2af4.local`.
    pub hostname: Option<String>,
    /// TXT `fn`.
    pub friendly_name: Option<String>,
    /// TXT `md`, `model` or `ty`.
    pub model: Option<String>,
}

/// One SSDP (UPnP) response to an M-SEARCH.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsdpReply {
    pub ip: Ipv4Addr,
    /// The `ST` header.
    pub st: String,
    /// The `SERVER` header.
    pub server: Option<String>,
    /// The `LOCATION` header (URL of the device description).
    pub location: Option<String>,
}

/// A NetBIOS node-status answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetbiosInfo {
    pub name: String,
    pub workgroup: Option<String>,
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

    /// Ask the DNS `server` for the PTR record of `ip`.
    async fn reverse_dns(
        &self,
        server: Ipv4Addr,
        ip: Ipv4Addr,
    ) -> Result<Option<String>, PlatformError>;

    /// Browse mDNS services for `window`, sending from the interface address `local`.
    async fn mdns_browse(
        &self,
        local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError>;

    /// Send an SSDP M-SEARCH from `local` and collect replies for `window`.
    async fn ssdp_search(
        &self,
        local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<SsdpReply>, PlatformError>;

    /// Unicast NetBIOS node-status query to `ip`; `None` when it does not answer.
    async fn netbios_status(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError>;
}
