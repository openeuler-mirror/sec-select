pub mod connection_pool;
pub mod db;
pub mod error;
pub mod filesystem;
pub mod kvstore;
pub mod schema;
pub mod snapshot;
pub mod toolcalls;

use error::{Error, Result};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};
use crate::db::DbValue as Value;

// Re-export filesystem types
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use filesystem::HostFS;
pub use filesystem::{
    BoxedFile, DirEntry, File, FileSystem, FilesystemStats, FsError, OverlayFS, Stats, TimeChange,
    DEFAULT_DIR_MODE, DEFAULT_FILE_MODE, S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT,
    S_IFREG, S_IFSOCK,
};
pub use kvstore::KvStore;
pub use schema::{SchemaVersion, SECAFS_SCHEMA_VERSION};
pub use toolcalls::{ToolCall, ToolCallStats, ToolCallStatus, ToolCalls};

/// Directory containing secafs databases
pub fn secafs_dir() -> &'static std::path::Path {
    std::path::Path::new(".secafs")
}

/// Information about a mounted secafs filesystem
#[derive(Debug, Clone)]
pub struct Mount {
    /// The ID (from the mount source, e.g., "secafs:my-agent" -> "my-agent")
    pub id: String,
    /// The mountpoint path
    pub mountpoint: PathBuf,
}

/// Get all currently mounted secafs filesystems by parsing /proc/mounts
///
/// This is the authoritative source for mount information - if it's in /proc/mounts,
/// it's mounted. If not, it's not. No stale state possible.
#[cfg(target_os = "linux")]
pub fn get_mounts() -> Vec<Mount> {
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return vec![];
    };
    contents
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0].starts_with("secafs:") {
                let agent_id = parts[0].strip_prefix("secafs:")?.to_string();
                if agent_id == "fuse" {
                    return None;
                }
                Some(Mount {
                    id: agent_id,
                    mountpoint: PathBuf::from(parts[1]),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Get all currently mounted secafs filesystems (non-Linux stub)
#[cfg(not(target_os = "linux"))]
pub fn get_mounts() -> Vec<Mount> {
    vec![]
}

/// Database backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseBackend {
    #[default]
    Postgres,
    OpenGauss,
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseBackend::Postgres => write!(f, "postgres"),
            DatabaseBackend::OpenGauss => write!(f, "opengauss"),
        }
    }
}

/// Configuration options for opening a SecAFS instance
#[derive(Debug, Clone, Default)]
pub struct SecAFSOptions {
    /// Optional unique identifier for the agent.
    pub id: Option<String>,
    /// Optional custom path (unused with Postgres, kept for API compat).
    pub path: Option<String>,
    /// Postgres connection URL
    pub postgres_url: Option<String>,
    /// Postgres connection pool size (defaults to 4)
    pub postgres_pool_size: Option<usize>,
    /// Optional base directory for overlay filesystem (copy-on-write).
    pub base: Option<PathBuf>,
    /// Database backend type (Postgres or OpenGauss)
    pub backend: DatabaseBackend,
}

impl SecAFSOptions {
    /// Validates an agent ID to prevent path traversal and ensure safe filesystem operations.
    pub fn validate_agent_id(id: &str) -> bool {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    }

    /// Create options with a Postgres (or openGauss) connection URL.
    ///
    /// Accepts both `postgres://` and `opengauss://` URLs. An `opengauss://`
    /// URL selects the OpenGauss backend (so the SQL-compatibility rewrites in
    /// `db.rs` are applied) and is normalized to `postgres://` for the driver —
    /// i.e. this delegates to [`with_opengauss_url`]. Without scheme-based
    /// detection here, callers that pass an `opengauss://` URL through this
    /// constructor (the CLI, the daemon) would silently get the Postgres
    /// backend and emit PG-only syntax (e.g. `EXECUTE FUNCTION`) that OpenGauss
    /// rejects.
    pub fn with_postgres_url(url: impl Into<String>) -> Self {
        let url = url.into();
        if url.starts_with("opengauss://") {
            return Self::with_opengauss_url(url);
        }
        Self {
            id: None,
            path: None,
            postgres_url: Some(url),
            postgres_pool_size: None,
            base: None,
            backend: DatabaseBackend::Postgres,
        }
    }

    /// Create options with an OpenGauss connection URL.
    /// The `opengauss://` scheme is normalized to `postgres://` for the driver.
    pub fn with_opengauss_url(url: impl Into<String>) -> Self {
        let url = url.into();
        let pg_url = if url.starts_with("opengauss://") {
            url.replacen("opengauss://", "postgres://", 1)
        } else {
            url
        };
        Self {
            id: None,
            path: None,
            postgres_url: Some(pg_url),
            postgres_pool_size: None,
            base: None,
            backend: DatabaseBackend::OpenGauss,
        }
    }

    /// Configure Postgres pool size
    pub fn with_postgres_pool_size(mut self, size: usize) -> Self {
        self.postgres_pool_size = Some(size);
        self
    }

    /// Set the base directory for overlay filesystem (copy-on-write)
    pub fn with_base(mut self, base: impl Into<PathBuf>) -> Self {
        self.base = Some(base.into());
        self
    }

    /// Resolve an id-or-path string to SecAFSOptions
    pub fn resolve(id_or_path: impl Into<String>) -> Result<Self> {
        let id_or_path = id_or_path.into();

        if id_or_path.starts_with("postgres://")
            || id_or_path.starts_with("postgresql://")
        {
            return Ok(Self::with_postgres_url(id_or_path));
        }

        if id_or_path.starts_with("opengauss://") {
            return Ok(Self::with_opengauss_url(id_or_path));
        }

        Err(Error::Internal(
            "only postgres:// or opengauss:// URLs are supported".to_string(),
        ))
    }
}

/// The main SecAFS SDK struct
///
/// This provides a unified interface to the filesystem, key-value store,
/// and tool calls tracking backed by PostgreSQL.
pub struct SecAFS {
    pool: connection_pool::ConnectionPool,
    pub kv: KvStore,
    pub fs: filesystem::SecAFS,
    pub tools: ToolCalls,
}

impl SecAFS {
    /// Open a SecAFS instance
    ///
    /// # Arguments
    /// * `options` - Configuration options
    ///
    /// # Examples
    /// ```no_run
    /// use secafs_sdk::{SecAFS, SecAFSOptions};
    ///
    /// # async fn example() -> secafs_sdk::error::Result<()> {
    /// let agent = SecAFS::open(SecAFSOptions::with_postgres_url("postgres://localhost/mydb")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn open(options: SecAFSOptions) -> Result<Self> {
        if let Some(ref path) = options.base {
            if !path.exists() {
                return Err(Error::BaseDirectoryNotFound(path.display().to_string()));
            }
            if !path.is_dir() {
                return Err(Error::NotADirectory(path.display().to_string()));
            }
        }

        let mut pg_url = options
            .postgres_url
            .clone()
            .ok_or_else(|| Error::Internal("postgres_url is required".to_string()))?;
        if !pg_url.contains("connect_timeout=") {
            let sep = if pg_url.contains('?') { "&" } else { "?" };
            pg_url.push_str(&format!("{}connect_timeout=10", sep));
        }

        let pool_size = options.postgres_pool_size.unwrap_or(4).max(1);
        let mut clients = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let (client, connection) =
                tokio_postgres::connect(pg_url.as_str(), tokio_postgres::NoTls).await?;
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    tracing::error!("postgres connection error: {err}");
                }
            });
            clients.push(Arc::new(client));
        }
        let pool = connection_pool::ConnectionPool::with_backend(clients, options.backend);

        let conn = pool.get_connection().await?;
        schema::check_schema_version(&conn).await?;
        drop(conn);

        if let Some(base_path) = options.base {
            let canonical_base = std::fs::canonicalize(base_path)?;
            let base_path_str = canonical_base.to_string_lossy().to_string();
            let conn = pool.get_connection().await?;
            OverlayFS::init_schema(&conn, &base_path_str).await?;
        }

        Self::open_with_pool(pool).await
    }

    /// Open a SecAFS instance from a connection pool
    pub async fn open_with_pool(
        pool: connection_pool::ConnectionPool,
    ) -> Result<Self> {
        let kv = KvStore::from_pool(pool.clone()).await?;
        let fs = filesystem::SecAFS::from_pool(pool.clone()).await?;
        let tools = ToolCalls::from_pool(pool.clone()).await?;

        Ok(Self {
            pool,
            kv,
            fs,
            tools,
        })
    }

    /// Get a connection from the pool
    pub async fn get_connection(&self) -> Result<db::DbConn> {
        self.pool.get_connection().await
    }

    /// Get the connection pool
    pub fn get_pool(&self) -> connection_pool::ConnectionPool {
        self.pool.clone()
    }

    /// Get all paths in the delta layer (files in fs_dentry)
    pub async fn get_delta_paths(&self) -> Result<HashSet<String>> {
        const ROOT_INO: i64 = 1;
        let conn = self.pool.get_connection().await?;

        let mut paths = HashSet::new();
        let mut queue: VecDeque<(i64, String)> = VecDeque::new();
        queue.push_back((ROOT_INO, String::new()));

        while let Some((parent_ino, prefix)) = queue.pop_front() {
            let query = format!(
                "SELECT d.name, d.ino, i.mode FROM fs_dentry d
                 JOIN fs_inode i ON d.ino = i.ino
                 WHERE d.parent_ino = {}
                 ORDER BY d.name",
                parent_ino
            );

            let mut rows = conn.query(&query, ()).await?;

            while let Some(row) = rows.next().await? {
                let name: String = row
                    .get_value(0)
                    .ok()
                    .and_then(|v| {
                        if let Value::Text(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                let ino: i64 = row
                    .get_value(1)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0);

                let mode: u32 = row
                    .get_value(2)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32;

                let full_path = if prefix.is_empty() {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", prefix, name)
                };

                paths.insert(full_path.clone());

                let is_dir = mode & S_IFMT == S_IFDIR;
                if is_dir {
                    queue.push_back((ino, full_path));
                }
            }
        }

        Ok(paths)
    }

    /// Get the file mode for a path in the delta layer
    pub async fn get_file_mode(&self, path: &str) -> Result<Option<u32>> {
        const ROOT_INO: i64 = 1;
        let conn = self.pool.get_connection().await?;

        let components: Vec<&str> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if components.is_empty() {
            let mut rows = conn
                .query("SELECT mode FROM fs_inode WHERE ino = ?", (ROOT_INO,))
                .await?;

            if let Some(row) = rows.next().await? {
                let mode = row
                    .get_value(0)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32;
                return Ok(Some(mode));
            }
            return Ok(None);
        }

        let mut current_ino = ROOT_INO;
        for component in &components {
            let query = format!(
                "SELECT ino FROM fs_dentry WHERE parent_ino = {} AND name = '{}'",
                current_ino, component
            );

            let mut rows = conn.query(&query, ()).await?;

            if let Some(row) = rows.next().await? {
                current_ino = row
                    .get_value(0)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0);
            } else {
                return Ok(None);
            }
        }

        let mut rows = conn
            .query("SELECT mode FROM fs_inode WHERE ino = ?", (current_ino,))
            .await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;
            return Ok(Some(mode));
        }

        Ok(None)
    }

    /// Get all whiteouts (deleted paths from base layer)
    pub async fn get_whiteouts(&self) -> Result<HashSet<String>> {
        let conn = self.pool.get_connection().await?;
        let mut whiteouts = HashSet::new();

        let result = conn.query("SELECT path FROM fs_whiteout", ()).await;

        if let Ok(mut rows) = result {
            while let Some(row) = rows.next().await? {
                if let Ok(Value::Text(path)) = row.get_value(0) {
                    whiteouts.insert(path.clone());
                }
            }
        }

        Ok(whiteouts)
    }

    /// Check if overlay is enabled for this filesystem
    pub async fn is_overlay_enabled(&self) -> Result<Option<String>> {
        let conn = self.pool.get_connection().await?;
        let result = conn
            .query(
                "SELECT value FROM fs_overlay_config WHERE key = 'base_path'",
                (),
            )
            .await;

        match result {
            Ok(mut rows) => {
                if let Some(row) = rows.next().await? {
                    let base_path: String = row
                        .get_value(0)
                        .ok()
                        .and_then(|v| {
                            if let Value::Text(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    Ok(Some(base_path))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }
}
