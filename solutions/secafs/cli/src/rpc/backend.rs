/// Real FUSE mount backend that wires RPC dispatch to actual FUSE mounts.
///
/// Each mounted volume is tracked by a `FuseMountHandle` that wraps a
/// `BackgroundSession`. Dropping the handle unmounts the FUSE filesystem
/// (via the `Mount` Drop impl in the fuser shim). The backend also owns a
/// `ConnectionPool` for calling `volume::ensure` and `volume::destroy`.
///
/// Linux-only: FUSE is not available on macOS.
#[cfg(target_os = "linux")]
pub mod linux {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;

    use anyhow::Context as _;
    use async_trait::async_trait;
    use tokio::sync::{Mutex, RwLock};
    use tokio_postgres::NoTls;

    use secafs_sdk::connection_pool::ConnectionPool;
    use secafs_sdk::filesystem::secafs::SecAFS;
    use secafs_sdk::filesystem::volume;
    use secafs_sdk::DatabaseBackend;
    use secafs_sdk::FileSystem;

    /// Strip `opengauss://` prefix (driver only knows `postgres://`) and
    /// return the cleaned URL plus the inferred backend. Any other scheme
    /// (including bare `postgres://`) is treated as PostgreSQL.
    fn split_url_backend(pg_url: &str) -> (String, DatabaseBackend) {
        if let Some(rest) = pg_url.strip_prefix("opengauss://") {
            (format!("postgres://{rest}"), DatabaseBackend::OpenGauss)
        } else {
            (pg_url.to_string(), DatabaseBackend::Postgres)
        }
    }

    use crate::rpc::methods::MountBackend;

    /// Opaque handle to a live FUSE mount.
    ///
    /// The inner `BackgroundSession` owns the background thread that services
    /// FUSE kernel messages. When this handle is dropped the `Mount` guard
    /// inside `BackgroundSession` runs its Drop impl, which calls
    /// `fusermount -u` / `libc::umount2` to unmount.
    pub struct FuseMountHandle {
        _bg: crate::fuser::BackgroundSession,
    }

    /// Build a `SecAFS` filesystem from the pool, start a background FUSE
    /// session at `host_path`, and return a handle that unmounts on drop.
    ///
    /// This is a synchronous function because `spawn_mount2` (which starts the
    /// kernel session) is itself synchronous. Callers that live on an async
    /// executor should use `tokio::task::spawn_blocking`.
    ///
    /// `volume_id` is used to set the per-session GUC `secafs.volume_id` on
    /// every connection in the pool before the FUSE session starts, so that
    /// the v0.6 undo triggers can resolve the volume context per row write.
    fn start_background_mount(
        pool: ConnectionPool,
        root_ino: i64,
        host_path: &Path,
        volume_id: &str,
    ) -> anyhow::Result<FuseMountHandle> {
        // Build the SecAFS filesystem impl backed by the connection pool.
        // `from_pool` is async; give it a fresh single-threaded runtime.
        // It (re)runs `initialize_schema`, which on OpenGauss SETs the
        // capture-trigger GUCs to placeholder values (`__init__` / `true`)
        // on whichever connection it grabs — so we *must* assert the real
        // per-volume values *after* from_pool, not before.
        let secafs = crate::get_runtime()
            .block_on(SecAFS::from_pool(pool))
            .context("SecAFS::from_pool failed")?;

        // Set the per-volume GUC on all connections in the dedicated pool so
        // that the undo triggers (which read current_setting('secafs.volume_id'))
        // can identify which volume each inode write belongs to.
        crate::get_runtime()
            .block_on(secafs.get_pool().set_volume_id_guc(volume_id))
            .context("set_volume_id_guc failed")?;
        let fs: Arc<dyn FileSystem> = Arc::new(secafs);

        // Each mount gets its own Tokio runtime to drive async FileSystem ops
        // from within synchronous FUSE callbacks via `block_on`.
        let runtime = crate::get_runtime();
        let fsname = format!("secafs:{root_ino}");

        let bg = crate::fuse::spawn_mount(fs, host_path, fsname, runtime, root_ino)
            .context("fuse::spawn_mount failed")?;

        Ok(FuseMountHandle { _bg: bg })
    }

    /// Real FUSE mount backend.
    ///
    /// Thread-safe: `mounts` is behind a `tokio::sync::Mutex`.
    ///
    /// Each call to `mount` creates a **dedicated** `ConnectionPool` for that
    /// volume and pre-sets `secafs.volume_id` on every connection in that pool.
    /// This ensures the v0.6 undo triggers can identify the owning volume for
    /// every inode write without any per-op overhead.
    pub struct FuseMountBackend {
        /// Shared pool used for management operations (volume::ensure, destroy,
        /// schema init). FUSE I/O uses per-mount dedicated pools instead.
        /// Behind a RwLock so it can be REBUILT when its connections die —
        /// the server closing idle connections (e.g. openGauss
        /// session_timeout) or a DB restart must not permanently brick the
        /// daemon; see `mgmt_conn`.
        pool: RwLock<ConnectionPool>,
        /// Postgres DSN stored so we can open fresh connections for each mount
        /// and rebuild the management pool after a connection loss.
        pg_url: String,
        /// Number of connections to open per mount.
        mount_pool_size: usize,
        mounts: Mutex<HashMap<String, FuseMountHandle>>,
    }

    impl FuseMountBackend {
        /// Create a new backend, connecting to Postgres at `pg_url`.
        ///
        /// Opens `pool_size` connections for management operations and runs
        /// `initialize_schema` (idempotent) before returning so the tables are
        /// ready on first use.
        pub async fn new(pg_url: &str, pool_size: usize) -> anyhow::Result<Arc<Self>> {
            let pool_size = pool_size.max(1);
            let pool = Self::build_mgmt_pool(pg_url, pool_size).await?;
            Ok(Arc::new(Self {
                pool: RwLock::new(pool),
                pg_url: pg_url.to_string(),
                mount_pool_size: pool_size,
                mounts: Mutex::new(HashMap::new()),
            }))
        }

        /// Connect a fresh management pool: open `pool_size` connections,
        /// pre-set the trigger GUCs, and run `initialize_schema` (idempotent).
        /// Used at startup and to replace the pool after connection loss.
        async fn build_mgmt_pool(pg_url: &str, pool_size: usize) -> anyhow::Result<ConnectionPool> {
            let (driver_url, backend) = split_url_backend(pg_url);
            let mut clients = Vec::with_capacity(pool_size);
            for _ in 0..pool_size {
                let (client, connection) = tokio_postgres::connect(&driver_url, NoTls)
                    .await
                    .context("failed to connect to postgres")?;
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        tracing::error!("postgres connection dropped: {e}");
                    }
                });
                clients.push(Arc::new(client));
            }
            let pool = ConnectionPool::with_backend(clients, backend);

            // OpenGauss requires the v0.6 trigger GUCs to exist on every
            // session that may write to a triggered table — initialize_schema
            // only sets them on the connection it grabs, so every other
            // management-pool connection would raise "unrecognized
            // configuration parameter" on the next `volume::ensure`.
            // Pre-SET both on the whole pool with `suppress_undo='true'` so
            // any seed/admin insert short-circuits the trigger before its
            // `SELECT INTO state` (which OpenGauss raises on for zero rows).
            // PG accepts these SETs as harmless custom GUC writes. Mount
            // pools override these on their own connections.
            for i in 0..pool_size {
                let conn = pool
                    .get_connection_indexed(i)
                    .await
                    .context("failed to acquire mgmt-pool connection for GUC init")?;
                conn.batch_execute(
                    "SET secafs.volume_id = '__init__'; \
                     SET secafs.suppress_undo = 'true'",
                )
                .await
                .context("failed to pre-set GUCs on mgmt-pool connection")?;
            }

            // Ensure schema exists (idempotent CREATE TABLE IF NOT EXISTS).
            let conn = pool
                .get_connection()
                .await
                .context("failed to get connection for schema init")?;
            SecAFS::initialize_schema(&conn)
                .await
                .context("initialize_schema failed")?;
            drop(conn);

            Ok(pool)
        }

        /// Acquire a management connection, transparently rebuilding the pool
        /// if the round-robin handed back a dead one. Lazy self-healing: a
        /// dropped DB (idle-timeout kill, restart, network blip) costs one
        /// failed checkout + reconnect on the next management op instead of
        /// permanently failing until the daemon is restarted.
        async fn mgmt_conn(&self) -> anyhow::Result<secafs_sdk::db::DbConn> {
            {
                let pool = self.pool.read().await;
                let conn = pool
                    .get_connection()
                    .await
                    .map_err(|e| anyhow::anyhow!("pool.get_connection: {e}"))?;
                if !conn.is_closed() {
                    return Ok(conn);
                }
            }
            let mut pool = self.pool.write().await;
            // Double-check under the write lock: a concurrent caller may have
            // already swapped in a fresh pool while we waited.
            let conn = pool
                .get_connection()
                .await
                .map_err(|e| anyhow::anyhow!("pool.get_connection: {e}"))?;
            if !conn.is_closed() {
                return Ok(conn);
            }
            drop(conn);
            tracing::warn!("management pool connection closed; rebuilding pool");
            *pool = Self::build_mgmt_pool(&self.pg_url, self.mount_pool_size)
                .await
                .context("management pool rebuild failed (database unreachable?)")?;
            tracing::info!("management pool rebuilt");
            pool.get_connection()
                .await
                .map_err(|e| anyhow::anyhow!("pool.get_connection after rebuild: {e}"))
        }

        /// Open a fresh `ConnectionPool` with `self.mount_pool_size` connections.
        /// Used by `mount()` to create a dedicated per-volume pool so that setting
        /// `secafs.volume_id` on those connections does not affect the shared pool.
        async fn open_mount_pool(&self) -> anyhow::Result<ConnectionPool> {
            let (driver_url, backend) = split_url_backend(&self.pg_url);
            let mut clients = Vec::with_capacity(self.mount_pool_size);
            for _ in 0..self.mount_pool_size {
                let (client, connection) = tokio_postgres::connect(&driver_url, NoTls)
                    .await
                    .context("failed to open mount connection")?;
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        tracing::error!("mount postgres connection dropped: {e}");
                    }
                });
                clients.push(Arc::new(client));
            }
            Ok(ConnectionPool::with_backend(clients, backend))
        }
    }

    #[async_trait]
    impl MountBackend for FuseMountBackend {
        async fn get_connection(&self) -> anyhow::Result<secafs_sdk::db::DbConn> {
            self.mgmt_conn().await
        }

        async fn mount(&self, id: &str, host_path: &Path) -> anyhow::Result<()> {
            // Ensure the volume row and its root inode exist; idempotent.
            let conn = self.mgmt_conn().await?;
            let root_ino = volume::ensure(&conn, id)
                .await
                .context("volume::ensure failed")?;
            drop(conn);

            // Open a dedicated pool for this mount so the GUC we set below
            // does not bleed into the shared pool used for management ops.
            let mount_pool = self.open_mount_pool().await?;

            // `start_background_mount` is synchronous (spawns an OS thread),
            // so use spawn_blocking to avoid stalling the async executor.
            let host_path = host_path.to_path_buf();
            let volume_id = id.to_string();
            let handle = tokio::task::spawn_blocking(move || {
                start_background_mount(mount_pool, root_ino, &host_path, &volume_id)
            })
            .await
            .context("spawn_blocking panicked")??;

            self.mounts.lock().await.insert(id.to_string(), handle);
            Ok(())
        }

        async fn unmount(&self, id: &str) -> anyhow::Result<()> {
            // Dropping the handle triggers BackgroundSession → Mount Drop → unmount.
            self.mounts.lock().await.remove(id);
            Ok(())
        }

        async fn destroy(&self, id: &str) -> anyhow::Result<()> {
            // The dispatcher (methods::dispatch) calls unmount before destroy,
            // so the FUSE session is torn down before we delete DB rows.
            let conn = self.mgmt_conn().await?;
            volume::destroy(&conn, id)
                .await
                .context("volume::destroy failed")?;
            Ok(())
        }
    }
}

// Re-export for callers so they don't need to qualify the platform module.
#[cfg(target_os = "linux")]
pub use linux::FuseMountBackend;
#[cfg(target_os = "linux")]
pub use linux::FuseMountHandle;
