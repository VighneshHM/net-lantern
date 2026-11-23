use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use super::adapters::active_adapter_guid;
use super::arp::arp_resolve_blocking;
use super::firewall::read_firewall_status;
use super::icmp::ping_blocking;
use super::netlist::read_network_category;
use super::wlan::{wifi_bss_list_blocking, wifi_current_blocking, wifi_scan_start_blocking};
use super::{read_adapters, read_neighbors};
use crate::{
    choose_interface, tcp_connect_probe, ArpEntry, FirewallStatus, Interface, MacAddr,
    NearbyNetwork, NetworkCategory, PingReply, Platform, PlatformError, TcpProbe, WifiConnection,
};

/// How long a Wi-Fi scan waits after `WlanScan` before reading the BSS list.
const SCAN_WAIT: Duration = Duration::from_secs(4);

/// The real Windows `Platform`: IP Helper for ICMP, ARP and the neighbour table,
/// and the WLAN API for Wi-Fi.
#[derive(Default)]
pub struct WindowsPlatform {
    if_index: Mutex<Option<u32>>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        Self::default()
    }

    fn cached_index(&self) -> Option<u32> {
        *self.if_index.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn join_err(e: tokio::task::JoinError) -> PlatformError {
    PlatformError::Io {
        message: format!("blocking task failed: {e}"),
    }
}

#[async_trait]
impl Platform for WindowsPlatform {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        let adapters = tokio::task::spawn_blocking(read_adapters)
            .await
            .map_err(join_err)??;
        let (iface, idx) = choose_interface(&adapters).ok_or(PlatformError::NotConnected)?;
        *self.if_index.lock().unwrap_or_else(|e| e.into_inner()) = Some(idx);
        Ok(iface)
    }

    async fn ping(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<PingReply>, PlatformError> {
        tokio::task::spawn_blocking(move || ping_blocking(ip, timeout))
            .await
            .map_err(join_err)?
    }

    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        let task = tokio::task::spawn_blocking(move || arp_resolve_blocking(ip));
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(mac)) => Ok(mac),
            // A panicked task or a timeout both mean nobody answered.
            _ => Ok(None),
        }
    }

    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        let idx = match self.cached_index() {
            Some(i) => i,
            None => {
                self.active_interface().await?;
                self.cached_index().ok_or(PlatformError::NotConnected)?
            }
        };
        tokio::task::spawn_blocking(move || read_neighbors(idx))
            .await
            .map_err(join_err)?
    }

    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        Ok(tcp_connect_probe(ip, port, timeout).await)
    }

    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        tokio::task::spawn_blocking(wifi_current_blocking)
            .await
            .map_err(join_err)?
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        tokio::task::spawn_blocking(wifi_scan_start_blocking)
            .await
            .map_err(join_err)??;
        tokio::time::sleep(SCAN_WAIT).await;
        tokio::task::spawn_blocking(wifi_bss_list_blocking)
            .await
            .map_err(join_err)?
    }

    async fn network_category(&self) -> Result<NetworkCategory, PlatformError> {
        tokio::task::spawn_blocking(|| {
            let adapter = active_adapter_guid()?.ok_or(PlatformError::NotConnected)?;
            read_network_category(adapter)
        })
        .await
        .map_err(join_err)?
    }

    async fn firewall_status(
        &self,
        exe: &std::path::Path,
        port: u16,
    ) -> Result<FirewallStatus, PlatformError> {
        let exe = exe.to_path_buf();
        tokio::task::spawn_blocking(move || read_firewall_status(&exe, port))
            .await
            .map_err(join_err)?
    }

    async fn os_description(&self) -> String {
        tokio::task::spawn_blocking(windows_description)
            .await
            .unwrap_or_else(|_| "unknown".to_string())
    }
}

/// "Windows 11 (build 26200)" from `RtlGetVersion`, which (unlike
/// `GetVersionEx`) reports the real version whatever the manifest says.
fn windows_description() -> String {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid, correctly sized OSVERSIONINFOW.
    if unsafe { RtlGetVersion(&mut info) }.is_err() {
        return "unknown".to_string();
    }
    describe_windows(info.dwMajorVersion, info.dwBuildNumber)
}

/// Windows 11 reports major version 10 like Windows 10; build 22000 and up
/// is Windows 11.
fn describe_windows(major: u32, build: u32) -> String {
    let name = match major {
        10 if build >= 22000 => "Windows 11",
        10 => "Windows 10",
        _ => "Windows",
    };
    format!("{name} (build {build})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_at_or_above_22000_are_windows_11() {
        assert_eq!(describe_windows(10, 26200), "Windows 11 (build 26200)");
        assert_eq!(describe_windows(10, 22000), "Windows 11 (build 22000)");
        assert_eq!(describe_windows(10, 19045), "Windows 10 (build 19045)");
    }

    /// Reads this machine's real category and firewall rules. Read-only, but it
    /// needs a real network, so it runs only on request.
    #[tokio::test]
    #[ignore = "reads this machine's network category and firewall"]
    async fn live_category_and_firewall_read_without_error() {
        let platform = WindowsPlatform::new();
        let category = platform.network_category().await;
        assert!(category.is_ok(), "network_category: {category:?}");
        let exe = std::env::current_exe().unwrap();
        let firewall = platform.firewall_status(&exe, 47800).await;
        assert!(firewall.is_ok(), "firewall_status: {firewall:?}");
        println!("category: {category:?}\nfirewall: {firewall:?}");
    }
}
