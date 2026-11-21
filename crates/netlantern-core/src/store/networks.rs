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

    /// Records the Wi-Fi name this network was last seen under.
    pub async fn set_ssid_label(&self, network: NetworkId, ssid: &str) -> Result<(), StoreError> {
        let ssid = ssid.to_string();
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE networks SET ssid_label = ?1 WHERE id = ?2",
                    params![ssid, network.0],
                )
                .map(|_| ())
            })
            .await
    }

    /// The most recently seen network with this SSID and subnet whose gateway MAC differs from `gateway_mac`.
    pub async fn replaced_router_candidate(
        &self,
        gateway_mac: MacAddr,
        subnet: &str,
        ssid: &str,
    ) -> Result<Option<(NetworkId, String)>, StoreError> {
        use rusqlite::OptionalExtension;
        let (mac, subnet, ssid) = (
            gateway_mac.to_string(),
            subnet.to_string(),
            ssid.to_string(),
        );
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT id, ssid_label FROM networks
                     WHERE subnet = ?1 AND ssid_label = ?2 AND gateway_mac != ?3
                     ORDER BY last_seen DESC, id DESC LIMIT 1",
                    params![subnet, ssid, mac],
                    |r| Ok((NetworkId(r.get(0)?), r.get(1)?)),
                )
                .optional()
            })
            .await
    }

    /// Points `network` at a new gateway MAC (the "keep my devices" answer).
    /// Fails, changing nothing, when that MAC already has a network on the same
    /// subnet: `UNIQUE(gateway_mac, subnet)` rejects the update.
    pub async fn move_network_to_gateway(
        &self,
        network: NetworkId,
        gateway_mac: MacAddr,
    ) -> Result<(), StoreError> {
        let mac = gateway_mac.to_string();
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE networks SET gateway_mac = ?1 WHERE id = ?2",
                    params![mac, network.0],
                )
                .map(|_| ())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;

    fn store() -> Store {
        Store::new(Db::open_in_memory().unwrap())
    }

    fn mac(last: u8) -> MacAddr {
        MacAddr([2, 0, 0, 0, 0, last])
    }

    const SUBNET: &str = "192.168.1.0/24";

    async fn net_with_ssid(
        s: &Store,
        m: MacAddr,
        subnet: &str,
        ssid: &str,
        seen: i64,
    ) -> NetworkId {
        let id = s.network_for(m, subnet, seen).await.unwrap();
        s.set_ssid_label(id, ssid).await.unwrap();
        id
    }

    #[tokio::test]
    async fn ssid_label_is_stored_and_overwritten() {
        let s = store();
        let id = s.network_for(mac(1), SUBNET, 10).await.unwrap();
        s.set_ssid_label(id, "Home").await.unwrap();
        s.set_ssid_label(id, "Home 5G").await.unwrap();
        let (_, label) = s
            .replaced_router_candidate(mac(2), SUBNET, "Home 5G")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(label, "Home 5G");
        assert!(s
            .replaced_router_candidate(mac(2), SUBNET, "Home")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn replaced_router_candidate_matches_ssid_and_subnet_only() {
        let s = store();
        let id = net_with_ssid(&s, mac(1), SUBNET, "Home", 10).await;
        let found = s
            .replaced_router_candidate(mac(2), SUBNET, "Home")
            .await
            .unwrap();
        assert_eq!(found, Some((id, "Home".to_string())));
        assert!(s
            .replaced_router_candidate(mac(1), SUBNET, "Home")
            .await
            .unwrap()
            .is_none());
        assert!(s
            .replaced_router_candidate(mac(2), "10.0.0.0/24", "Home")
            .await
            .unwrap()
            .is_none());
        assert!(s
            .replaced_router_candidate(mac(2), SUBNET, "Office")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn replaced_router_candidate_picks_most_recently_seen() {
        let s = store();
        net_with_ssid(&s, mac(1), SUBNET, "Home", 10).await;
        let newer = net_with_ssid(&s, mac(2), SUBNET, "Home", 20).await;
        net_with_ssid(&s, mac(3), SUBNET, "Home", 15).await;
        let found = s
            .replaced_router_candidate(mac(9), SUBNET, "Home")
            .await
            .unwrap();
        assert_eq!(found.map(|f| f.0), Some(newer));
    }

    #[tokio::test]
    async fn move_network_to_gateway_makes_find_network_match_the_new_mac() {
        let s = store();
        let id = s.network_for(mac(1), SUBNET, 10).await.unwrap();
        s.move_network_to_gateway(id, mac(2)).await.unwrap();
        assert_eq!(s.find_network(mac(2), SUBNET).await.unwrap(), Some(id));
        assert_eq!(s.find_network(mac(1), SUBNET).await.unwrap(), None);
    }

    #[tokio::test]
    async fn move_network_to_gateway_refuses_when_the_new_mac_already_has_a_network() {
        let s = store();
        let a = s.network_for(mac(1), SUBNET, 10).await.unwrap();
        let b = s.network_for(mac(2), SUBNET, 20).await.unwrap();
        assert!(s.move_network_to_gateway(a, mac(2)).await.is_err());
        assert_eq!(s.find_network(mac(1), SUBNET).await.unwrap(), Some(a));
        assert_eq!(s.find_network(mac(2), SUBNET).await.unwrap(), Some(b));
    }
}
