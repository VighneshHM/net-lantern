//! The SQLite store: a writer thread, versioned migrations, and repositories
//! for networks, devices (identified by MAC) and scans.

mod checks;
mod db;
mod devices;
mod migrations;
mod networks;
mod presence;
mod rules;
mod samples;
mod scans;
mod sessions;

pub use db::Db;
pub use devices::DevicePatch;
pub use presence::PresenceSighting;

/// Why a store call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(String),
    #[error("the database was written by a newer Net Lantern (schema {found}, this build supports {supported})")]
    NewerSchema { found: u32, supported: u32 },
    #[error("the database thread has stopped")]
    Closed,
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e.to_string())
    }
}

/// Typed access to the database. Cheap to clone; clones share one writer thread.
#[derive(Clone)]
pub struct Store {
    db: Db,
}

impl Store {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

/// An enum's snake_case serde name, as stored in text columns.
pub(crate) fn enum_text<T: serde::Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// Inverse of [`enum_text`].
pub(crate) fn enum_parse<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| StoreError::Sqlite(format!("unexpected stored value {s:?}")))
}
