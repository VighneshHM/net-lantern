//! Networks: a gateway MAC plus a subnet.

use netlantern_platform::MacAddr;
use rusqlite::params;

use super::{Store, StoreError};
use crate::model::NetworkId;

impl Store {
    /// The network behind this gateway and subnet; created on first sight.
    pub async fn network_for(
        &self,
        gateway_mac: MacAddr,
        subnet: &str,
        now: i64,
    ) -> Result<NetworkId, StoreError> {
        let (mac, subnet) = (gateway_mac.to_string(), subnet.to_string());
        self.db
            .call(move |c| {
                c.query_row(
                    "INSERT INTO networks(gateway_mac, subnet, first_seen, last_seen) VALUES(?1, ?2, ?3, ?3)
                     ON CONFLICT(gateway_mac, subnet) DO UPDATE SET last_seen = excluded.last_seen
                     RETURNING id",
                    params![mac, subnet, now],
                    |r| r.get(0).map(NetworkId),
                )
            })
            .await
    }

    /// Whether the user allowed deep scans on this network. Off by default.
    pub async fn scan_consent(&self, network: NetworkId) -> Result<bool, StoreError> {
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT scan_consent FROM networks WHERE id = ?1",
                    [network.0],
                    |r| r.get(0),
                )
            })
            .await
    }

    pub async fn set_scan_consent(
        &self,
        network: NetworkId,
        granted: bool,
    ) -> Result<(), StoreError> {
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE networks SET scan_consent = ?1 WHERE id = ?2",
                    params![granted, network.0],
                )
                .map(|_| ())
            })
            .await
    }

    /// Like [`Store::network_for`], but never creates one.
    pub async fn find_network(
        &self,
        gateway_mac: MacAddr,
        subnet: &str,
    ) -> Result<Option<NetworkId>, StoreError> {
        use rusqlite::OptionalExtension;
        let (mac, subnet) = (gateway_mac.to_string(), subnet.to_string());
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT id FROM networks WHERE gateway_mac = ?1 AND subnet = ?2",
                    params![mac, subnet],
                    |r| r.get(0).map(NetworkId),
                )
                .optional()
            })
            .await
    }
}
