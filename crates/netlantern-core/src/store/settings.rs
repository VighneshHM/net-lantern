//! App-wide settings, kept in the `settings` table under the `global` scope.

use rusqlite::{params, OptionalExtension};

use super::{Store, StoreError};

/// The setting that turns phone access on or off; values `"on"` and `"off"`.
pub const PHONE_ACCESS_KEY: &str = "phone_access";

/// Network rules are scoped `network:<id>`, so this cannot collide with them.
const GLOBAL_SCOPE: &str = "global";

impl Store {
    pub async fn global_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        let key = key.to_string();
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT value FROM settings WHERE scope = ?1 AND key = ?2",
                    params![GLOBAL_SCOPE, key],
                    |r| r.get(0),
                )
                .optional()
            })
            .await
    }

    pub async fn set_global_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let (key, value) = (key.to_string(), value.to_string());
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO settings(scope, key, value) VALUES(?1, ?2, ?3)
                     ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value",
                    params![GLOBAL_SCOPE, key, value],
                )
                .map(|_| ())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NetworkId;
    use crate::monitor::RuleSettings;
    use crate::store::Db;

    fn store() -> Store {
        Store::new(Db::open_in_memory().unwrap())
    }

    #[tokio::test]
    async fn global_settings_round_trip_and_missing_is_none() {
        let s = store();
        assert_eq!(s.global_setting(PHONE_ACCESS_KEY).await.unwrap(), None);
        s.set_global_setting(PHONE_ACCESS_KEY, "on").await.unwrap();
        assert_eq!(
            s.global_setting(PHONE_ACCESS_KEY).await.unwrap().as_deref(),
            Some("on")
        );
        s.set_global_setting(PHONE_ACCESS_KEY, "off").await.unwrap();
        assert_eq!(
            s.global_setting(PHONE_ACCESS_KEY).await.unwrap().as_deref(),
            Some("off")
        );
    }

    #[tokio::test]
    async fn global_scope_does_not_collide_with_network_rules() {
        let s = store();
        let net = NetworkId(1);
        let before = s.rule_settings(net).await.unwrap();
        s.set_global_setting("notifications", "off").await.unwrap();
        let after = s.rule_settings(net).await.unwrap();
        assert_eq!(before, after);
        assert_eq!(after, RuleSettings::default());
    }
}
