use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use super::arp::arp_resolve_blocking;
use super::icmp::ping_blocking;
use super::{read_adapters, read_neighbors};
use crate::{
    choose_interface, tcp_connect_probe, ArpEntry, Interface, MacAddr, PingReply, Platform,
    PlatformError, TcpProbe,
};

/// The real Windows `Platform`: IP Helper for ICMP, ARP and the neighbour table.
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
}
