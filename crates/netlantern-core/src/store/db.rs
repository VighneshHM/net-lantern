//! The database handle: one dedicated thread owns the connection and runs
//! closures sent to it.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use rusqlite::Connection;
use tokio::sync::oneshot;

use super::migrations::{self, MIGRATIONS};
use super::StoreError;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Handle to the writer thread. Clones share the thread; it stops when the
/// last clone is dropped.
#[derive(Clone)]
pub struct Db {
    tx: mpsc::Sender<Job>,
}

impl Db {
    /// Open (creating if needed) the database file and bring it up to date.
    pub fn open(path: &Path) -> Result<Db, StoreError> {
        let mut conn = Connection::open(path)?;
        Self::start(&mut conn, Some(path))?;
        Ok(Self::spawn(conn))
    }

    pub fn open_in_memory() -> Result<Db, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        Self::start(&mut conn, None)?;
        Ok(Self::spawn(conn))
    }

    fn start(conn: &mut Connection, path: Option<&Path>) -> Result<(), StoreError> {
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        // Refuse a newer file before changing anything about it (WAL included).
        migrations::check_version(conn, MIGRATIONS.len() as u32)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        migrations::apply(conn, path, MIGRATIONS)?;
        migrations::fail_interrupted_scans(conn)?;
        super::sessions::end_dangling_sessions(conn)?;
        Ok(())
    }

    fn spawn(mut conn: Connection) -> Db {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("netlantern-db".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    if catch_unwind(AssertUnwindSafe(|| job(&mut conn))).is_err() {
                        tracing::error!("a database job panicked");
                    }
                }
            })
            .expect("spawn the database thread");
        Db { tx }
    }

    /// Run `f` on the writer thread and wait for its result.
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T, StoreError> {
        self.call_store(move |c| f(c).map_err(StoreError::from))
            .await
    }

    /// Like [`Db::call`], for closures that can fail with a [`StoreError`].
    pub(crate) async fn call_store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        let (reply, rx) = oneshot::channel();
        let job: Job = Box::new(move |c| {
            let result = catch_unwind(AssertUnwindSafe(|| f(c)))
                .unwrap_or_else(|_| Err(StoreError::Sqlite("database job panicked".into())));
            let _ = reply.send(result);
        });
        self.tx.send(job).map_err(|_| StoreError::Closed)?;
        rx.await.map_err(|_| StoreError::Closed)?
    }
}
