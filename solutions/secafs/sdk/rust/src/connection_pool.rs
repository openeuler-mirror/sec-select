//! Connection pool abstraction for PostgreSQL and OpenGauss.

use std::sync::Arc;

use crate::db::{DbConn, DbPool};
use crate::error::Result;
use crate::DatabaseBackend;

#[derive(Clone)]
pub struct ConnectionPool {
    inner: Arc<DbPool>,
}

impl ConnectionPool {
    pub fn new(clients: Vec<Arc<tokio_postgres::Client>>) -> Self {
        Self {
            inner: Arc::new(DbPool::new(clients)),
        }
    }

    pub fn with_backend(
        clients: Vec<Arc<tokio_postgres::Client>>,
        backend: DatabaseBackend,
    ) -> Self {
        Self {
            inner: Arc::new(DbPool::with_backend(clients, backend)),
        }
    }

    pub async fn get_connection(&self) -> Result<DbConn> {
        self.inner.get_connection().await
    }

    /// Number of underlying client connections in this pool.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Acquire the connection at the given pool index, panicking if `i` is
    /// out of range. Used by callers that need to apply a per-session
    /// configuration (such as a SET) to every connection in the pool.
    pub async fn get_connection_indexed(&self, i: usize) -> Result<DbConn> {
        self.inner.get_connection_at(i).await
    }

    /// Set `secafs.volume_id = '<volume_id>'` on **every** connection in the
    /// pool.  Call this once after creating a dedicated per-mount pool so that
    /// all subsequent FUSE-op connections carry the correct GUC, allowing the
    /// v0.6 undo triggers to identify which volume an inode write belongs to.
    ///
    /// Uses a plain `SET` (not `SET LOCAL`) so the GUC persists for the
    /// lifetime of each long-lived connection rather than being scoped to a
    /// single transaction.
    pub async fn set_volume_id_guc(&self, volume_id: &str) -> Result<()> {
        let pool_size = self.inner.len();
        for i in 0..pool_size {
            let conn = self.inner.get_connection_at(i).await?;
            // Use SET (session-level) so the GUC survives across transactions.
            // The SQL identifier is a plain string literal — no user-supplied
            // data is interpolated into the identifier position; volume_id is
            // embedded as a quoted string value.
            let sql = format!("SET secafs.volume_id = '{}'", volume_id.replace('\'', "''"));
            conn.batch_execute(&sql).await?;
            // OpenGauss requires the GUC to exist before the trigger reads
            // `current_setting('secafs.suppress_undo')`; PG accepts undefined
            // custom GUCs only via the 2-arg form which OG lacks. Initialize
            // to 'false' so the trigger short-circuit reads correctly. The
            // restore path overrides this with `SET LOCAL secafs.suppress_undo
            // = 'true'` for the duration of its transaction.
            conn.batch_execute("SET secafs.suppress_undo = 'false'").await?;
        }
        Ok(())
    }
}
