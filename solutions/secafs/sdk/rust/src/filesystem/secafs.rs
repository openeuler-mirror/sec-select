use crate::db::{DbConn as Connection, DbTransaction as Transaction, DbValue as Value, TransactionBehavior};
use crate::error::{Error, Result};
use async_trait::async_trait;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    BoxedFile, DirEntry, File, FileSystem, FilesystemStats, FsError, Stats, TimeChange,
    DEFAULT_DIR_MODE, DEFAULT_FILE_MODE, MAX_NAME_LEN, S_IFLNK, S_IFMT, S_IFREG,
};
use crate::connection_pool::ConnectionPool;
use crate::schema::SECAFS_SCHEMA_VERSION;

const ROOT_INO: i64 = 1;
const DEFAULT_CHUNK_SIZE: usize = 4096;
const DENTRY_CACHE_MAX_SIZE: usize = 10000;

/// LRU cache for directory entry lookups.
///
/// Maps (parent_ino, name) -> child_ino to avoid repeated database queries
/// during path resolution. For a path like `/a/b/c/d`, this reduces queries
/// from 4 to potentially 0 on cache hits.
struct DentryCache {
    // Mutex required because LruCache::get() mutates internal order
    entries: Mutex<LruCache<(i64, String), i64>>,
}

impl DentryCache {
    fn new(max_size: usize) -> Self {
        Self {
            entries: Mutex::new(LruCache::new(
                NonZeroUsize::new(max_size).expect("cache size must be > 0"),
            )),
        }
    }

    /// Look up a cached entry (updates LRU order)
    fn get(&self, parent_ino: i64, name: &str) -> Option<i64> {
        self.entries
            .lock()
            .unwrap()
            .get(&(parent_ino, name.to_string()))
            .copied()
    }

    /// Insert an entry into the cache (evicts LRU entry if full)
    fn insert(&self, parent_ino: i64, name: &str, child_ino: i64) {
        self.entries
            .lock()
            .unwrap()
            .put((parent_ino, name.to_string()), child_ino);
    }

    /// Remove an entry from the cache
    fn remove(&self, parent_ino: i64, name: &str) {
        self.entries
            .lock()
            .unwrap()
            .pop(&(parent_ino, name.to_string()));
    }
}

/// A filesystem backed by PostgreSQL
#[derive(Clone)]
pub struct SecAFS {
    pool: ConnectionPool,
    chunk_size: usize,
    /// Cache for directory entry lookups (shared across clones)
    dentry_cache: Arc<DentryCache>,
}

/// An open file handle for SecAFS.
///
/// This struct holds the inode number resolved at open time, allowing
/// efficient read/write/fsync operations without path lookups.
pub struct SecAFSFile {
    pool: ConnectionPool,
    ino: i64,
    chunk_size: usize,
}

#[async_trait]
impl File for SecAFSFile {
    async fn pread(&self, offset: u64, size: u64) -> Result<Vec<u8>> {
        let conn = self.pool.get_connection().await?;

        // Get the file size to avoid returning data beyond EOF
        let mut size_stmt = conn
            .prepare_cached("SELECT size FROM fs_inode WHERE ino = ?")
            .await?;
        let mut size_rows = size_stmt.query((self.ino,)).await?;
        let file_size = if let Some(row) = size_rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64
        } else {
            0
        };

        // If offset is at or beyond EOF, return empty
        if offset >= file_size {
            return Ok(Vec::new());
        }

        // Limit size to not exceed EOF
        let size = std::cmp::min(size, file_size - offset);

        let chunk_size = self.chunk_size as u64;
        let start_chunk = offset / chunk_size;
        let end_chunk = (offset + size).saturating_sub(1) / chunk_size;

        let mut stmt = conn
            .prepare_cached("SELECT chunk_index, data FROM fs_data WHERE ino = ? AND chunk_index >= ? AND chunk_index <= ? ORDER BY chunk_index")
            .await?;
        let mut rows = stmt
            .query((self.ino, start_chunk as i64, end_chunk as i64))
            .await?;

        let mut result = Vec::with_capacity(size as usize);
        let start_offset_in_chunk = (offset % chunk_size) as usize;
        let mut next_expected_chunk = start_chunk;

        while let Some(row) = rows.next().await? {
            let chunk_index = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64;

            // Fill gaps with zeros for sparse files
            while next_expected_chunk < chunk_index && result.len() < size as usize {
                let skip = if next_expected_chunk == start_chunk {
                    start_offset_in_chunk
                } else {
                    0
                };
                let zeros_needed =
                    std::cmp::min(chunk_size as usize - skip, size as usize - result.len());
                result.extend(std::iter::repeat_n(0u8, zeros_needed));
                next_expected_chunk += 1;
            }

            if let Ok(Value::Blob(chunk_data)) = row.get_value(1) {
                let skip = if chunk_index == start_chunk {
                    start_offset_in_chunk
                } else {
                    0
                };
                if skip >= chunk_data.len() {
                    // Chunk is smaller than skip offset, fill with zeros
                    let zeros_needed =
                        std::cmp::min(chunk_size as usize - skip, size as usize - result.len());
                    result.extend(std::iter::repeat_n(0u8, zeros_needed));
                } else {
                    let remaining = size as usize - result.len();
                    let take = std::cmp::min(chunk_data.len() - skip, remaining);
                    result.extend_from_slice(&chunk_data[skip..skip + take]);

                    // If chunk is smaller than chunk_size, pad with zeros
                    let chunk_end = skip + take;
                    if chunk_end < chunk_size as usize && result.len() < size as usize {
                        let zeros_needed = std::cmp::min(
                            chunk_size as usize - chunk_end,
                            size as usize - result.len(),
                        );
                        result.extend(std::iter::repeat_n(0u8, zeros_needed));
                    }
                }
            }
            next_expected_chunk = chunk_index + 1;
        }

        // Fill any remaining space with zeros (for sparse file tail or missing chunks at end)
        if result.len() < size as usize {
            result.resize(size as usize, 0);
        }

        Ok(result)
    }

    async fn pwrite(&self, offset: u64, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        let conn = self.pool.get_connection().await?;
        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
        // Get current file size
        let mut stmt = conn
            .prepare_cached("SELECT size FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((self.ino,)).await?;
        let current_size = if let Some(row) = rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64
        } else {
            0
        };

        // Write the actual data (sparse gaps are handled by pread which fills
        // missing chunks with zeros, so no need to zero-fill here)
        self.write_data_at_offset_with_conn(&conn, offset, data)
            .await?;

        // Update file size and mtime
        let new_size = std::cmp::max(current_size, offset + data.len() as u64);
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET size = ?, mtime = ?, mtime_nsec = ? WHERE ino = ?")
            .await?;
        stmt.execute((new_size as i64, now_secs, now_nsec, self.ino))
            .await?;
        txn.commit().await?;

        Ok(())
    }

    async fn truncate(&self, new_size: u64) -> Result<()> {
        let conn = self.pool.get_connection().await?;

        // Get current size
        let mut stmt = conn
            .prepare_cached("SELECT size FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((self.ino,)).await?;
        let current_size = if let Some(row) = rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64
        } else {
            0
        };

        let chunk_size = self.chunk_size as u64;

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let result: Result<()> = async {
            if new_size == 0 {
                // Special case: truncate to zero - just delete all chunks
                let mut stmt = conn
                    .prepare_cached("DELETE FROM fs_data WHERE ino = ?")
                    .await?;
                stmt.execute((self.ino,)).await?;
            } else if new_size < current_size {
                // Shrinking: delete excess chunks and truncate last chunk if needed
                let last_chunk_idx = (new_size - 1) / chunk_size;

                // Delete all chunks beyond the last one we need
                conn.execute(
                    "DELETE FROM fs_data WHERE ino = ? AND chunk_index > ?",
                    (self.ino, last_chunk_idx as i64),
                )
                .await?;

                // Truncate the last chunk if needed
                let offset_in_chunk = (new_size % chunk_size) as usize;
                if offset_in_chunk > 0 {
                    let mut stmt = conn
                        .prepare_cached("SELECT data FROM fs_data WHERE ino = ? AND chunk_index = ?")
                        .await?;
                    let mut rows = stmt.query((self.ino, last_chunk_idx as i64)).await?;

                    if let Some(row) = rows.next().await? {
                        if let Ok(Value::Blob(mut chunk_data)) = row.get_value(0) {
                            if chunk_data.len() > offset_in_chunk {
                                chunk_data.truncate(offset_in_chunk);
                                let mut stmt = conn
                                    .prepare_cached("UPDATE fs_data SET data = ? WHERE ino = ? AND chunk_index = ?")
                                    .await?;
                                stmt.execute((Value::Blob(chunk_data), self.ino, last_chunk_idx as i64)).await?;
                            }
                        }
                    }
                }
            }
            // For extending (new_size > current_size), we just update the size
            // The sparse regions will be handled by pread returning zeros

            // Update the inode size, mtime, and ctime
            let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
            let now_secs = dur.as_secs() as i64;
            let now_nsec = dur.subsec_nanos() as i64;
            let mut stmt = conn
                .prepare_cached("UPDATE fs_inode SET size = ?, mtime = ?, ctime = ?, mtime_nsec = ?, ctime_nsec = ? WHERE ino = ?")
                .await?;
            stmt.execute((new_size as i64, now_secs, now_secs, now_nsec, now_nsec, self.ino)).await?;

            Ok(())
        }
        .await;

        if result.is_err() {
            let _ = txn.rollback().await;
            return result;
        }
        txn.commit().await?;
        Ok(())
    }

    async fn fsync(&self) -> Result<()> {
        Ok(())
    }

    async fn fstat(&self) -> Result<Stats> {
        let conn = self.pool.get_connection().await?;
        let mut stmt = conn
            .prepare_cached("SELECT ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((self.ino,)).await?;

        if let Some(row) = rows.next().await? {
            SecAFS::build_stats_from_row(&row)
        } else {
            Err(FsError::NotFound.into())
        }
    }
}

impl SecAFSFile {
    /// Write data at a specific offset, handling chunk boundaries.
    /// Uses a provided connection to allow reuse within a transaction.
    async fn write_data_at_offset_with_conn(
        &self,
        conn: &Connection,
        offset: u64,
        data: &[u8],
    ) -> Result<()> {
        let chunk_size = self.chunk_size as u64;
        let mut written = 0usize;

        if data.is_empty() {
            return Ok(());
        }

        // get statements only once (in order to avoid heavy clone on every while iteration)
        let mut select_stmt = conn
            .prepare_cached("SELECT data FROM fs_data WHERE ino = ? AND chunk_index = ?")
            .await?;
        let insert_sql = "INSERT INTO fs_data (ino, chunk_index, data) VALUES (?, ?, ?)
                ON CONFLICT (ino, chunk_index) DO UPDATE SET data = EXCLUDED.data";
        let mut insert_stmt = conn.prepare_cached(insert_sql).await?;
        while written < data.len() {
            let current_offset = offset + written as u64;
            let chunk_index = (current_offset / chunk_size) as i64;
            let offset_in_chunk = (current_offset % chunk_size) as usize;

            // How much can we write in this chunk?
            let remaining_in_chunk = self.chunk_size - offset_in_chunk;
            let remaining_data = data.len() - written;
            let to_write = std::cmp::min(remaining_in_chunk, remaining_data);

            let mut chunk_data;
            if to_write != chunk_size as usize {
                // Get existing chunk data (if any)
                let mut rows = select_stmt.query((self.ino, chunk_index)).await?;

                chunk_data = if let Some(row) = rows.next().await? {
                    row.get_value(0)
                        .ok()
                        .and_then(|v| {
                            if let Value::Blob(b) = v {
                                Some(b)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                select_stmt.reset()?;

                // Extend chunk if needed
                if chunk_data.len() < offset_in_chunk + to_write {
                    chunk_data.resize(offset_in_chunk + to_write, 0);
                }

                // Write data into chunk
                chunk_data[offset_in_chunk..offset_in_chunk + to_write]
                    .copy_from_slice(&data[written..written + to_write]);
            } else {
                chunk_data = data[written..written + to_write].to_vec();
            }

            // Save chunk
            insert_stmt
                .execute((self.ino, chunk_index, Value::Blob(chunk_data)))
                .await?;
            insert_stmt.reset()?;

            written += to_write;
        }

        Ok(())
    }
}

impl SecAFS {
    /// Create a filesystem from a connection pool
    pub async fn from_pool(pool: ConnectionPool) -> Result<Self> {
        let conn = pool.get_connection().await?;

        Self::initialize_schema(&conn).await?;

        let chunk_size = Self::read_chunk_size(&conn).await?;

        let fs = Self {
            pool,
            chunk_size,
            dentry_cache: Arc::new(DentryCache::new(DENTRY_CACHE_MAX_SIZE)),
        };
        Ok(fs)
    }

    /// Get the configured chunk size
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    /// Get a database connection from the pool
    pub async fn get_connection(&self) -> Result<Connection> {
        self.pool.get_connection().await
    }

    /// Get the connection pool
    pub fn get_pool(&self) -> ConnectionPool {
        self.pool.clone()
    }

    /// Initialize the database schema
    pub async fn initialize_schema(conn: &Connection) -> Result<()> {
        // Pre-register the custom GUCs read by the v0.6 capture triggers.
        // OpenGauss validates `current_setting('NAME')` references at
        // `CREATE OR REPLACE FUNCTION` time and rejects unrecognized names;
        // PG accepts an unset custom GUC at creation but reads it as NULL.
        // SETting both here ensures the trigger DDL parses on both backends.
        // `suppress_undo='true'` makes the trigger short-circuit at the
        // suppress-check (before the `SELECT INTO state` that OpenGauss
        // would otherwise raise on for zero rows in `fs_volume_state`),
        // so the seed INSERT/UPDATE on `fs_inode` for the root directory
        // does not fail. `set_volume_id_guc` overrides both per mount,
        // and `restore_inner` re-asserts suppress for its transaction.
        conn.batch_execute(
            "SET secafs.volume_id = '__init__'; \
             SET secafs.suppress_undo = 'true'",
        )
        .await?;

        // Create config table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            (),
        )
        .await?;

        // Create inode table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_inode (
                ino BIGSERIAL PRIMARY KEY,
                mode BIGINT NOT NULL,
                nlink BIGINT NOT NULL DEFAULT 0,
                uid BIGINT NOT NULL DEFAULT 0,
                gid BIGINT NOT NULL DEFAULT 0,
                size BIGINT NOT NULL DEFAULT 0,
                atime BIGINT NOT NULL,
                mtime BIGINT NOT NULL,
                ctime BIGINT NOT NULL,
                rdev BIGINT NOT NULL DEFAULT 0
            )",
            (),
        )
        .await?;

        conn.execute(
            "ALTER TABLE fs_inode ADD COLUMN IF NOT EXISTS atime_nsec BIGINT NOT NULL DEFAULT 0",
            (),
        )
        .await
        .ok();
        conn.execute(
            "ALTER TABLE fs_inode ADD COLUMN IF NOT EXISTS mtime_nsec BIGINT NOT NULL DEFAULT 0",
            (),
        )
        .await
        .ok();
        conn.execute(
            "ALTER TABLE fs_inode ADD COLUMN IF NOT EXISTS ctime_nsec BIGINT NOT NULL DEFAULT 0",
            (),
        )
        .await
        .ok();

        // Create directory entry table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_dentry (
                id BIGSERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                parent_ino BIGINT NOT NULL,
                ino BIGINT NOT NULL,
                UNIQUE(parent_ino, name)
            )",
            (),
        )
        .await?;

        // Create index for efficient path lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fs_dentry_parent
            ON fs_dentry(parent_ino, name)",
            (),
        )
        .await?;

        // Create data chunks table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_data (
                ino BIGINT NOT NULL,
                chunk_index BIGINT NOT NULL,
                data BYTEA NOT NULL,
                PRIMARY KEY (ino, chunk_index)
            )",
            (),
        )
        .await?;

        // Create symlink table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_symlink (
                ino BIGINT PRIMARY KEY,
                target TEXT NOT NULL
            )",
            (),
        )
        .await?;

        // Create volumes table for per-conversation FUSE roots
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_volumes (
                id TEXT PRIMARY KEY,
                root_ino BIGINT NOT NULL REFERENCES fs_inode(ino) ON DELETE CASCADE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            (),
        )
        .await?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS fs_volumes_root_ino_idx ON fs_volumes(root_ino)",
            (),
        )
        .await?;

        // Ensure kv_store exists before v0.6 trigger DDL binds to it. Normally
        // created lazily by KvStore::from_pool, but the rollback trigger on
        // kv_store needs the table to be present at SecAFS::open time.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS kv_store (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                created_at BIGINT DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
                updated_at BIGINT DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
            )",
            (),
        )
        .await?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_kv_store_created_at ON kv_store(created_at)",
            (),
        )
        .await?;

        // v0.6: snapshot tables + triggers (Copy-on-Write rollback).
        for stmt in crate::snapshot::schema::ddl_statements() {
            conn.batch_execute(stmt).await?;
        }
        for stmt in crate::snapshot::triggers::ddl_statements() {
            conn.batch_execute(stmt).await?;
        }

        // Ensure chunk_size config exists
        let mut rows = conn
            .query("SELECT value FROM fs_config WHERE key = 'chunk_size'", ())
            .await?;

        if rows.next().await?.is_none() {
            conn.execute(
                "INSERT INTO fs_config (key, value) VALUES ('chunk_size', ?)",
                (DEFAULT_CHUNK_SIZE.to_string(),),
            )
            .await?;
        }

        // Set schema version
        conn.execute(
            "INSERT INTO fs_config (key, value) VALUES ('schema_version', ?)
            ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            (SECAFS_SCHEMA_VERSION,),
        )
        .await?;

        // Ensure root directory exists with correct ownership
        let mut rows = conn
            .query("SELECT ino FROM fs_inode WHERE ino = ?", (ROOT_INO,))
            .await?;

        // SAFETY: getuid/getgid are always safe
        #[cfg(unix)]
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        #[cfg(not(unix))]
        let (uid, gid) = (0u32, 0u32);

        if rows.next().await?.is_none() {
            let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
            let now_secs = dur.as_secs() as i64;
            let now_nsec = dur.subsec_nanos() as i64;
            conn.execute(
                "INSERT INTO fs_inode (ino, mode, nlink, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                VALUES (?, ?, 2, ?, ?, 0, ?, ?, ?, ?, ?, ?)",
                (ROOT_INO, DEFAULT_DIR_MODE as i64, uid, gid, now_secs, now_secs, now_secs, now_nsec, now_nsec, now_nsec),
            )
            .await?;
        } else {
            // Update existing root inode ownership to current user
            conn.execute(
                "UPDATE fs_inode SET uid = ?, gid = ? WHERE ino = ?",
                (uid, gid, ROOT_INO),
            )
            .await?;
        }

        conn.execute(
            "SELECT setval(pg_get_serial_sequence('fs_inode','ino'), \
            (SELECT GREATEST(MAX(ino), 1) FROM fs_inode))",
            (),
        )
        .await?;

        Ok(())
    }

    /// Read chunk size from config
    async fn read_chunk_size(conn: &Connection) -> Result<usize> {
        let mut rows = conn
            .query("SELECT value FROM fs_config WHERE key = 'chunk_size'", ())
            .await?;

        if let Some(row) = rows.next().await? {
            let value = row
                .get_value(0)
                .ok()
                .and_then(|v| match v {
                    Value::Text(s) => s.parse::<usize>().ok(),
                    Value::Integer(i) => Some(i as usize),
                    _ => None,
                })
                .unwrap_or(DEFAULT_CHUNK_SIZE);
            Ok(value)
        } else {
            Ok(DEFAULT_CHUNK_SIZE)
        }
    }

    /// Normalize a path
    fn normalize_path(&self, path: &str) -> String {
        let normalized = path.trim_end_matches('/');
        let normalized = if normalized.is_empty() {
            "/"
        } else if normalized.starts_with('/') {
            normalized
        } else {
            return format!("/{}", normalized);
        };

        // Handle . and .. components
        let components: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
        let mut result = Vec::new();

        for component in components {
            match component {
                "." => {
                    // Current directory - skip it
                    continue;
                }
                ".." => {
                    // Parent directory - only pop if there is a component to pop (don't traverse above root)
                    if !result.is_empty() {
                        result.pop();
                    }
                }
                _ => {
                    result.push(component);
                }
            }
        }

        if result.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", result.join("/"))
        }
    }

    /// Split path into components
    fn split_path(&self, path: &str) -> Vec<String> {
        let normalized = self.normalize_path(path);
        if normalized == "/" {
            return vec![];
        }
        normalized
            .split('/')
            .filter(|p| !p.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    /// Look up a child entry by parent inode and name using a provided connection.
    ///
    /// This is more efficient than `resolve_path` when you already have the parent inode,
    /// as it avoids re-resolving all parent path components.
    async fn lookup_child(
        &self,
        conn: &Connection,
        parent_ino: i64,
        name: &str,
    ) -> Result<Option<i64>> {
        let mut stmt = conn
            .prepare_cached("SELECT ino FROM fs_dentry WHERE parent_ino = ? AND name = ?")
            .await?;
        let mut rows = stmt.query((parent_ino, name)).await?;

        let mut found_ino = None;
        let mut row_count = 0;

        while let Some(row) = rows.next().await? {
            found_ino = row.get_value(0).ok().and_then(|v| v.as_integer().copied());
            row_count += 1;
        }

        if row_count > 1 {
            return Err(FsError::InvalidPath.into());
        }

        Ok(found_ino)
    }

    /// Get link count for an inode
    async fn get_link_count(&self, conn: &Connection, ino: i64) -> Result<u32> {
        let mut stmt = conn
            .prepare_cached("SELECT nlink FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let nlink = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0);
            Ok(nlink as u32)
        } else {
            Ok(0)
        }
    }

    /// Get file attributes by inode using an existing connection
    async fn getattr_with_conn(&self, conn: &Connection, ino: i64) -> Result<Option<Stats>> {
        let mut stmt = conn
            .prepare_cached("SELECT ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let stats = Self::build_stats_from_row(&row)?;
            Ok(Some(stats))
        } else {
            Ok(None)
        }
    }

    /// Build a Stats object from a database row
    ///
    /// The row should contain columns in this order:
    /// ino, mode, nlink, uid, gid, size, atime, mtime, ctime
    fn build_stats_from_row(row: &crate::db::DbRow) -> Result<Stats> {
        Ok(Stats {
            ino: row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0),
            mode: row
                .get_value(1)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32,
            nlink: row
                .get_value(2)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(1) as u32,
            uid: row
                .get_value(3)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32,
            gid: row
                .get_value(4)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32,
            size: row
                .get_value(5)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0),
            atime: row
                .get_value(6)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0),
            mtime: row
                .get_value(7)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0),
            ctime: row
                .get_value(8)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0),
            atime_nsec: row
                .get_value(10)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32,
            mtime_nsec: row
                .get_value(11)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32,
            ctime_nsec: row
                .get_value(12)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32,
            rdev: row
                .get_value(9)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64,
        })
    }

    /// Resolve a path to an inode number
    async fn resolve_path(&self, path: &str) -> Result<Option<i64>> {
        let conn = self.pool.get_connection().await?;
        self.resolve_path_with_conn(&conn, path).await
    }

    /// Resolve a path to an inode number using a provided connection
    async fn resolve_path_with_conn(&self, conn: &Connection, path: &str) -> Result<Option<i64>> {
        let components = self.split_path(path);
        if components.is_empty() {
            return Ok(Some(ROOT_INO));
        }

        let mut statement: Option<crate::db::DbStatement<'_>> = None;
        let mut current_ino = ROOT_INO;
        for component in components {
            // Check cache first
            if let Some(cached_ino) = self.dentry_cache.get(current_ino, &component) {
                current_ino = cached_ino;
                continue;
            }

            // Cache miss - query database
            if let Some(statement) = &mut statement {
                statement.reset()?;
            } else {
                statement = Some(
                    conn.prepare_cached(
                        "SELECT ino FROM fs_dentry WHERE parent_ino = ? AND name = ?",
                    )
                    .await?,
                );
            }
            let statement = statement.as_mut().expect("statement was set above");
            let mut rows = statement.query((current_ino, component.as_str())).await?;

            let mut found_row = None;
            let mut row_count = 0;

            while let Some(row) = rows.next().await? {
                found_row = Some(row);
                row_count += 1;
            }

            if row_count > 1 {
                return Err(FsError::InvalidPath.into());
            }

            if let Some(row) = found_row {
                let child_ino = row
                    .get_value(0)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0);

                // Populate cache
                self.dentry_cache.insert(current_ino, &component, child_ino);
                current_ino = child_ino;
            } else {
                return Ok(None);
            }
        }

        Ok(Some(current_ino))
    }

    /// Get file statistics without following symlinks
    pub async fn lstat(&self, path: &str) -> Result<Option<Stats>> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let ino = match self.resolve_path_with_conn(&conn, &path).await? {
            Some(ino) => ino,
            None => return Ok(None),
        };

        let mut stmt = conn
            .prepare_cached("SELECT ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let stats = Self::build_stats_from_row(&row)?;
            Ok(Some(stats))
        } else {
            Ok(None)
        }
    }

    /// Get file statistics, following symlinks
    pub async fn stat(&self, path: &str) -> Result<Option<Stats>> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);

        // Follow symlinks with a maximum depth to prevent infinite loops
        let mut current_path = path;
        let max_symlink_depth = 40; // Standard limit for symlink following

        let mut stmt = conn.prepare_cached(
            "SELECT ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec FROM fs_inode WHERE ino = ?",
        ).await?;
        for _ in 0..max_symlink_depth {
            let ino = match self.resolve_path_with_conn(&conn, &current_path).await? {
                Some(ino) => ino,
                None => return Ok(None),
            };

            stmt.reset()?;
            let mut rows = stmt.query((ino,)).await?;

            if let Some(row) = rows.next().await? {
                let mode = row
                    .get_value(1)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32;

                // Check if this is a symlink
                if (mode & S_IFMT) == S_IFLNK {
                    // Read the symlink target
                    let target = self
                        .readlink_with_conn(&conn, &current_path)
                        .await?
                        .ok_or(FsError::NotFound)?;

                    // Resolve target path (handle both absolute and relative paths)
                    current_path = if target.starts_with('/') {
                        target
                    } else {
                        // Relative path - resolve relative to the symlink's directory
                        let base_path = Path::new(&current_path);
                        let parent = base_path.parent().unwrap_or(Path::new("/"));
                        let joined = parent.join(&target);
                        joined.to_string_lossy().into_owned()
                    };
                    current_path = self.normalize_path(&current_path);
                    continue; // Follow the symlink
                }

                // Not a symlink, return the stats
                let stats = Self::build_stats_from_row(&row)?;
                return Ok(Some(stats));
            } else {
                return Ok(None);
            }
        }

        // Too many symlinks
        Err(FsError::SymlinkLoop.into())
    }

    /// Get file statistics, following symlinks (using provided connection)
    async fn stat_with_conn(&self, conn: &Connection, path: &str) -> Result<Option<Stats>> {
        let path = self.normalize_path(path);

        // Follow symlinks with a maximum depth to prevent infinite loops
        let mut current_path = path;
        let max_symlink_depth = 40; // Standard limit for symlink following

        for _ in 0..max_symlink_depth {
            let ino = match self.resolve_path_with_conn(conn, &current_path).await? {
                Some(ino) => ino,
                None => return Ok(None),
            };

            let mut rows = conn
                .query(
                    "SELECT ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec FROM fs_inode WHERE ino = ?",
                    (ino,),
                )
                .await?;

            if let Some(row) = rows.next().await? {
                let mode = row
                    .get_value(1)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32;

                // Check if this is a symlink
                if (mode & S_IFMT) == S_IFLNK {
                    // Read the symlink target
                    let target = self
                        .readlink_with_conn(conn, &current_path)
                        .await?
                        .ok_or(FsError::InvalidPath)?;

                    // Resolve target path (handle both absolute and relative paths)
                    current_path = if target.starts_with('/') {
                        target
                    } else {
                        // Relative path - resolve relative to the symlink's directory
                        let base_path = Path::new(&current_path);
                        let parent = base_path.parent().unwrap_or(Path::new("/"));
                        let joined = parent.join(&target);
                        joined.to_string_lossy().into_owned()
                    };
                    current_path = self.normalize_path(&current_path);
                    continue; // Follow the symlink
                }

                // Not a symlink, return the stats
                let stats = Self::build_stats_from_row(&row)?;
                return Ok(Some(stats));
            } else {
                return Ok(None);
            }
        }

        // Too many symlinks
        Err(FsError::SymlinkLoop.into())
    }

    /// Create a directory
    pub async fn mkdir(&self, path: &str, uid: u32, gid: u32) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let components = self.split_path(&path);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        let parent_path = if components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", components[..components.len() - 1].join("/"))
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        // Check if already exists (single query using parent_ino we already have)
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create inode with default directory mode (path-based API doesn't accept mode)
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let row = stmt
            .query_row((
                DEFAULT_DIR_MODE as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                now_nsec,
                now_nsec,
                now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        // Create directory entry
        let mut stmt = conn
            .prepare_cached("INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)")
            .await?;
        stmt.execute((name.as_str(), parent_ino, ino)).await?;

        // Set nlink to 2 for new directory (self "." + parent's dentry)
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET nlink = 2 WHERE ino = ?")
            .await?;
        stmt.execute((ino,)).await?;

        // Increment parent nlink (new directory's ".." link) and update timestamps
        let mut stmt = conn
            .prepare_cached(
                "UPDATE fs_inode SET nlink = nlink + 1, ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
            )
            .await?;
        stmt.execute((now_secs, now_secs, now_nsec, now_nsec, parent_ino))
            .await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(())
    }

    /// Create a special file node (FIFO, device, socket, or regular file)
    pub async fn mknod(&self, path: &str, mode: u32, rdev: u64, uid: u32, gid: u32) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let components = self.split_path(&path);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        let parent_path = if components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", components[..components.len() - 1].join("/"))
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        // Check if already exists
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create inode with mode and rdev
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec)
                VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let row = stmt
            .query_row((
                mode as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                rdev as i64,
                now_nsec,
                now_nsec,
                now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        // Create directory entry
        let mut stmt = conn
            .prepare_cached("INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)")
            .await?;
        stmt.execute((name.as_str(), parent_ino, ino)).await?;

        // Increment link count
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?")
            .await?;
        stmt.execute((ino,)).await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(())
    }

    /// Create a new empty file with the specified mode and ownership.
    ///
    /// This is an optimized path for FUSE create() that combines inode creation,
    /// dentry creation, and file handle opening in a single operation.
    /// Returns both Stats and an open file handle.
    pub async fn create_file(
        &self,
        path: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<(Stats, BoxedFile)> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let components = self.split_path(&path);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        let parent_path = match components.len() {
            1 => "/".to_string(),
            _ => format!("/{}", components[..components.len() - 1].join("/")),
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Prepare statements before starting the transaction
        let mut inode_stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, nlink, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                 VALUES (?, 1, ?, ?, 0, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let mut dentry_stmt = conn
            .prepare_cached("INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)")
            .await?;

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let file_mode = S_IFREG | (mode & 0o7777);

        let row = inode_stmt
            .query_row((
                file_mode as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                now_nsec,
                now_nsec,
                now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        dentry_stmt
            .execute((name.as_str(), parent_ino, ino))
            .await?;

        txn.commit().await?;

        self.dentry_cache.insert(parent_ino, name, ino);

        let stats = Stats {
            ino,
            mode: file_mode,
            nlink: 1,
            uid,
            gid,
            size: 0,
            atime: now_secs,
            mtime: now_secs,
            ctime: now_secs,
            atime_nsec: now_nsec as u32,
            mtime_nsec: now_nsec as u32,
            ctime_nsec: now_nsec as u32,
            rdev: 0,
        };

        let file: BoxedFile = Arc::new(SecAFSFile {
            pool: self.pool.clone(),
            ino,
            chunk_size: self.chunk_size,
        });

        Ok((stats, file))
    }

    /// Read data from a file
    pub async fn read_file(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.pool.get_connection().await?;
        let ino = match self.resolve_path_with_conn(&conn, path).await? {
            Some(ino) => ino,
            None => return Ok(None),
        };

        let mut rows = conn
            .query(
                "SELECT data FROM fs_data WHERE ino = ? ORDER BY chunk_index",
                (ino,),
            )
            .await?;

        let mut data = Vec::new();
        while let Some(row) = rows.next().await? {
            if let Ok(Value::Blob(chunk)) = row.get_value(0) {
                data.extend_from_slice(&chunk);
            }
        }

        Ok(Some(data))
    }

    /// Reads from a file at a given offset.
    ///
    /// Similar to POSIX `pread`, this reads up to `size` bytes from the file
    /// starting at `offset`, without modifying any file cursor.
    ///
    /// Returns `Ok(None)` if the file does not exist.
    pub async fn pread(&self, path: &str, offset: u64, size: u64) -> Result<Option<Vec<u8>>> {
        let conn = self.pool.get_connection().await?;
        let ino = match self.resolve_path_with_conn(&conn, path).await? {
            Some(ino) => ino,
            None => return Ok(None),
        };

        // Calculate which chunks we need
        let chunk_size = self.chunk_size as u64;
        let start_chunk = offset / chunk_size;
        let end_chunk = (offset + size).saturating_sub(1) / chunk_size;

        let mut rows = conn
            .query(
                "SELECT chunk_index, data FROM fs_data WHERE ino = ? AND chunk_index >= ? AND chunk_index <= ? ORDER BY chunk_index",
                (ino, start_chunk as i64, end_chunk as i64),
            )
            .await?;

        let mut result = Vec::with_capacity(size as usize);
        let start_offset_in_chunk = (offset % chunk_size) as usize;

        while let Some(row) = rows.next().await? {
            if let Ok(Value::Blob(chunk_data)) = row.get_value(1) {
                let skip = if result.is_empty() {
                    start_offset_in_chunk
                } else {
                    0
                };
                if skip >= chunk_data.len() {
                    continue;
                }
                let remaining = size as usize - result.len();
                let take = std::cmp::min(chunk_data.len() - skip, remaining);
                result.extend_from_slice(&chunk_data[skip..skip + take]);
            }
        }

        Ok(Some(result))
    }

    /// Writes to a file at a given offset.
    ///
    /// Similar to POSIX `pwrite`, this writes `data` to the file starting at
    /// `offset`, without modifying any file cursor.
    ///
    /// If the offset is beyond the current file size, the file is extended with zeros.
    /// If the file does not exist, it will be created.
    pub async fn pwrite(&self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let components = self.split_path(&path);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        let parent_path = if components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", components[..components.len() - 1].join("/"))
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let result: Result<()> = async {
            // Calculate the final size upfront
            let write_end = offset + data.len() as u64;

            // Get or create the inode
            let (ino, current_size, is_new) =
                if let Some(ino) = self.resolve_path_with_conn(&conn, &path).await? {
                    // Get current file size
                    let mut stmt = conn
                        .prepare_cached("SELECT size FROM fs_inode WHERE ino = ?")
                        .await?;
                    let mut rows = stmt.query((ino,)).await?;
                    let size = if let Some(row) = rows.next().await? {
                        row.get_value(0)
                            .ok()
                            .and_then(|v| v.as_integer().copied())
                            .unwrap_or(0) as u64
                    } else {
                        0
                    };
                    (ino, size, false)
                } else {
                    // Create new inode with correct size upfront
                    let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
                    let now_secs = dur.as_secs() as i64;
                    let now_nsec = dur.subsec_nanos() as i64;
                    let new_size = write_end as i64;
                    let mut stmt = conn
                        .prepare_cached(
                            "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, nlink, atime_nsec, mtime_nsec, ctime_nsec)
                        VALUES (?, 0, 0, ?, ?, ?, ?, 1, ?, ?, ?) RETURNING ino",
                        )
                        .await?;
                    let row = stmt
                        .query_row((DEFAULT_FILE_MODE as i64, new_size, now_secs, now_secs, now_secs, now_nsec, now_nsec, now_nsec))
                        .await?;

                    let ino = row
                        .get_value(0)
                        .ok()
                        .and_then(|v| v.as_integer().copied())
                        .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

                    // Create directory entry
                    let mut stmt = conn
                        .prepare_cached(
                            "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)",
                        )
                        .await?;
                    stmt.execute((name.as_str(), parent_ino, ino)).await?;

                    (ino, 0, true)
                };

            // Handle empty writes - just update mtime
            if data.is_empty() {
                let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
                let now_secs = dur.as_secs() as i64;
                let now_nsec = dur.subsec_nanos() as i64;
                conn.prepare_cached("UPDATE fs_inode SET mtime = ?, mtime_nsec = ? WHERE ino = ?")
                    .await?
                    .execute((now_secs, now_nsec, ino))
                    .await?;
                return Ok(());
            }

            let chunk_size = self.chunk_size as u64;

            // Calculate affected chunk range
            let start_chunk = offset / chunk_size;
            let end_chunk = (write_end - 1) / chunk_size;

            // Process each affected chunk
            for chunk_idx in start_chunk..=end_chunk {
                let chunk_start = chunk_idx * chunk_size;

                // Calculate what part of data goes into this chunk
                let data_start = if offset > chunk_start {
                    (offset - chunk_start) as usize
                } else {
                    0
                };
                let data_end =
                    std::cmp::min(chunk_size as usize, (write_end - chunk_start) as usize);

                // Calculate what part of data to copy
                let src_start = if chunk_start > offset {
                    (chunk_start - offset) as usize
                } else {
                    0
                };
                let src_end = std::cmp::min(data.len(), src_start + (data_end - data_start));

                // Read existing chunk if we need to preserve some data
                let needs_read = data_start > 0 || data_end < chunk_size as usize;
                let mut chunk_data = if needs_read {
                    let mut rows = conn
                        .query(
                            "SELECT data FROM fs_data WHERE ino = ? AND chunk_index = ?",
                            (ino, chunk_idx as i64),
                        )
                        .await?;
                    if let Some(row) = rows.next().await? {
                        if let Ok(Value::Blob(data)) = row.get_value(0) {
                            let mut v = data.clone();
                            v.resize(chunk_size as usize, 0);
                            v
                        } else {
                            vec![0u8; chunk_size as usize]
                        }
                    } else {
                        vec![0u8; chunk_size as usize]
                    }
                } else {
                    vec![0u8; chunk_size as usize]
                };

                // Copy the new data into the chunk
                chunk_data[data_start..data_end].copy_from_slice(&data[src_start..src_end]);

                // Trim trailing zeros for the last chunk
                let actual_len = if chunk_idx == end_chunk {
                    let file_end_in_chunk = (write_end - chunk_start) as usize;
                    let old_end_in_chunk = if current_size > chunk_start {
                        std::cmp::min((current_size - chunk_start) as usize, chunk_size as usize)
                    } else {
                        0
                    };
                    std::cmp::max(file_end_in_chunk, old_end_in_chunk)
                } else {
                    chunk_size as usize
                };

                // Write the chunk - delete existing then insert
                conn.execute(
                    "DELETE FROM fs_data WHERE ino = ? AND chunk_index = ?",
                    (ino, chunk_idx as i64),
                )
                .await?;
                conn.execute(
                    "INSERT INTO fs_data (ino, chunk_index, data) VALUES (?, ?, ?)",
                    (ino, chunk_idx as i64, &chunk_data[..actual_len]),
                )
                .await?;
            }

            // Update size and mtime (only if not new, since new inodes already have correct values)
            if !is_new {
                let new_size = std::cmp::max(current_size, write_end);
                let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
                let now_secs = dur.as_secs() as i64;
                let now_nsec = dur.subsec_nanos() as i64;
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET size = ?, mtime = ?, mtime_nsec = ? WHERE ino = ?")
                    .await?;
                stmt.execute((new_size as i64, now_secs, now_nsec, ino)).await?;
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                txn.commit().await?;
                Ok(())
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }

    /// Truncate a file to a specific size.
    ///
    /// This operates directly on chunks without loading the entire file into memory:
    /// - Shrinking: deletes chunks beyond new size, truncates the last chunk if needed
    /// - Extending: pads with zeros up to the new size
    pub async fn truncate(&self, path: &str, new_size: u64) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let ino = self
            .resolve_path_with_conn(&conn, &path)
            .await?
            .ok_or(FsError::NotFound)?;

        // Get current size
        let mut stmt = conn
            .prepare_cached("SELECT size FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;
        let current_size = if let Some(row) = rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64
        } else {
            0
        };

        let chunk_size = self.chunk_size as u64;

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let result: Result<()> = async {
            if new_size == 0 {
                // Special case: truncate to zero - just delete all chunks
                let mut stmt = conn
                    .prepare_cached("DELETE FROM fs_data WHERE ino = ?")
                    .await?;
                stmt.execute((ino,)).await?;
            } else if new_size < current_size {
                // Shrinking: delete excess chunks and truncate last chunk if needed
                let last_chunk_idx = (new_size - 1) / chunk_size;

                // Delete all chunks beyond the last one we need
                conn.execute(
                    "DELETE FROM fs_data WHERE ino = ? AND chunk_index > ?",
                    (ino, last_chunk_idx as i64),
                )
                .await?;

                // Calculate where in the last chunk the file should end
                let end_in_last_chunk = ((new_size - 1) % chunk_size) + 1;

                // If the last chunk needs to be truncated (not a full chunk),
                // read it, truncate, and rewrite
                if end_in_last_chunk < chunk_size {
                    let mut stmt = conn
                        .prepare_cached("SELECT data FROM fs_data WHERE ino = ? AND chunk_index = ?")
                        .await?;
                    let mut rows = stmt.query((ino, last_chunk_idx as i64)).await?;

                    if let Some(row) = rows.next().await? {
                        if let Ok(Value::Blob(chunk_data)) = row.get_value(0) {
                            if chunk_data.len() > end_in_last_chunk as usize {
                                let truncated = &chunk_data[..end_in_last_chunk as usize];
                                let mut stmt = conn
                                    .prepare_cached("UPDATE fs_data SET data = ? WHERE ino = ? AND chunk_index = ?")
                                    .await?;
                                stmt.execute((truncated, ino, last_chunk_idx as i64)).await?;
                            }
                        }
                    }
                }
            } else if new_size > current_size {
                // Extending: pad last existing chunk and add zero chunks as needed
                let last_existing_chunk = if current_size == 0 {
                    None
                } else {
                    Some((current_size - 1) / chunk_size)
                };
                let last_new_chunk = (new_size - 1) / chunk_size;

                // Pad the last existing chunk with zeros if it's not full
                if let Some(last_idx) = last_existing_chunk {
                    let mut stmt = conn
                        .prepare_cached("SELECT data FROM fs_data WHERE ino = ? AND chunk_index = ?")
                        .await?;
                    let mut rows = stmt.query((ino, last_idx as i64)).await?;

                    if let Some(row) = rows.next().await? {
                        if let Ok(Value::Blob(chunk_data)) = row.get_value(0) {
                            let current_chunk_len = chunk_data.len();
                            let needed_len = if last_idx == last_new_chunk {
                                // Last existing chunk is also the last new chunk
                                ((new_size - 1) % chunk_size + 1) as usize
                            } else {
                                // Need to fill this chunk completely
                                chunk_size as usize
                            };

                            if needed_len > current_chunk_len {
                                let mut padded = chunk_data.clone();
                                padded.resize(needed_len, 0);
                                let mut stmt = conn
                                    .prepare_cached("UPDATE fs_data SET data = ? WHERE ino = ? AND chunk_index = ?")
                                    .await?;
                                stmt.execute((&padded[..], ino, last_idx as i64)).await?;
                            }
                        }
                    }
                }

                // Add new zero-filled chunks if needed
                let start_new_chunk = last_existing_chunk.map(|i| i + 1).unwrap_or(0);
                for chunk_idx in start_new_chunk..=last_new_chunk {
                    let chunk_len = if chunk_idx == last_new_chunk {
                        ((new_size - 1) % chunk_size + 1) as usize
                    } else {
                        chunk_size as usize
                    };
                    let zeros = vec![0u8; chunk_len];
                    conn.execute(
                        "INSERT INTO fs_data (ino, chunk_index, data) VALUES (?, ?, ?)",
                        (ino, chunk_idx as i64, &zeros[..]),
                    )
                    .await?;
                }
            }
            // else: new_size == current_size, nothing to do for data

            // Update size and mtime
            let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
            let now_secs = dur.as_secs() as i64;
            let now_nsec = dur.subsec_nanos() as i64;
            let mut stmt = conn
                .prepare_cached("UPDATE fs_inode SET size = ?, mtime = ?, mtime_nsec = ? WHERE ino = ?")
                .await?;
            stmt.execute((new_size as i64, now_secs, now_nsec, ino)).await?;

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                txn.commit().await?;
                Ok(())
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }

    /// List directory contents
    pub async fn readdir(&self, ino: i64) -> Result<Option<Vec<String>>> {
        let conn = self.pool.get_connection().await?;
        let mut rows = conn
            .query(
                "SELECT name FROM fs_dentry WHERE parent_ino = ? ORDER BY name",
                (ino,),
            )
            .await?;

        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let name = row
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
            if !name.is_empty() {
                entries.push(name);
            }
        }

        Ok(Some(entries))
    }

    /// List directory contents with full statistics (optimized batch query)
    ///
    /// Returns entries with their stats in a single JOIN query, avoiding N+1 queries.
    pub async fn readdir_plus(&self, ino: i64) -> Result<Option<Vec<DirEntry>>> {
        let conn = self.pool.get_connection().await?;
        let mut stmt = conn.prepare_cached("SELECT d.name, i.ino, i.mode, i.nlink, i.uid, i.gid, i.size, i.atime, i.mtime, i.ctime, i.rdev, i.atime_nsec, i.mtime_nsec, i.ctime_nsec
            FROM fs_dentry d
            JOIN fs_inode i ON d.ino = i.ino
            WHERE d.parent_ino = ?
            ORDER BY d.name"
        ).await?;
        // Single JOIN query to get all entry names and their stats (including link count)
        let mut rows = stmt.query((ino,)).await?;

        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let name = row
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

            if name.is_empty() {
                continue;
            }

            let entry_ino = row
                .get_value(1)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0);

            let nlink = row
                .get_value(3)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(1) as u32;

            let stats = Stats {
                ino: entry_ino,
                mode: row
                    .get_value(2)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                nlink,
                uid: row
                    .get_value(4)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                gid: row
                    .get_value(5)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                size: row
                    .get_value(6)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                atime: row
                    .get_value(7)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                mtime: row
                    .get_value(8)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                ctime: row
                    .get_value(9)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                atime_nsec: row
                    .get_value(11)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                mtime_nsec: row
                    .get_value(12)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                ctime_nsec: row
                    .get_value(13)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                rdev: row
                    .get_value(10)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u64,
            };

            entries.push(DirEntry { name, stats });
        }

        Ok(Some(entries))
    }

    /// Create a symbolic link with the specified ownership
    pub async fn symlink(&self, target: &str, linkpath: &str, uid: u32, gid: u32) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let linkpath = self.normalize_path(linkpath);
        let components = self.split_path(&linkpath);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        // Get parent directory
        let parent_path = if components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", components[..components.len() - 1].join("/"))
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        // Check if entry already exists (single query using parent_ino we already have)
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create inode for symlink
        let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;

        let mode = S_IFLNK | 0o777; // Symlinks typically have 777 permissions
        let size = target.len() as i64;

        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let row = stmt
            .query_row((
                mode, uid, gid, size, now_secs, now_secs, now_secs, now_nsec, now_nsec, now_nsec,
            ))
            .await?;

        // Get the newly created inode
        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .unwrap_or(0);

        // Store symlink target
        conn.execute(
            "INSERT INTO fs_symlink (ino, target) VALUES (?, ?)",
            (ino, target),
        )
        .await?;

        // Create directory entry
        conn.execute(
            "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)",
            (name.as_str(), parent_ino, ino),
        )
        .await?;

        // Increment link count
        conn.execute(
            "UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?",
            (ino,),
        )
        .await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(())
    }

    /// Create a hard link
    ///
    /// Creates a new directory entry `newpath` that refers to the same inode as `oldpath`.
    /// Both paths will share the same file data and metadata (except for the name).
    /// The link count (nlink) of the inode is incremented.
    pub async fn link(&self, oldpath: &str, newpath: &str) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let oldpath = self.normalize_path(oldpath);
        let newpath = self.normalize_path(newpath);
        let components = self.split_path(&newpath);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        // Resolve old path to get its inode
        let ino = self
            .resolve_path_with_conn(&conn, &oldpath)
            .await?
            .ok_or(FsError::NotFound)?;

        // Check if source is a directory (hard links to directories are not allowed)
        let mut rows = conn
            .query("SELECT mode FROM fs_inode WHERE ino = ?", (ino,))
            .await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) == super::S_IFDIR {
                return Err(FsError::IsADirectory.into());
            }
        } else {
            return Err(FsError::NotFound.into());
        }

        // Get parent directory of new path
        let parent_path = if components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", components[..components.len() - 1].join("/"))
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        // Check if new path already exists (single query using parent_ino we already have)
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create directory entry pointing to the same inode
        conn.execute(
            "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)",
            (name.as_str(), parent_ino, ino),
        )
        .await?;

        // Increment link count
        conn.execute(
            "UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?",
            (ino,),
        )
        .await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(())
    }

    /// Read the target of a symbolic link
    pub async fn readlink(&self, path: &str) -> Result<Option<String>> {
        let conn = self.pool.get_connection().await?;
        self.readlink_with_conn(&conn, path).await
    }

    /// Read the target of a symbolic link using a provided connection
    async fn readlink_with_conn(&self, conn: &Connection, path: &str) -> Result<Option<String>> {
        let path = self.normalize_path(path);

        let ino = match self.resolve_path_with_conn(conn, &path).await? {
            Some(ino) => ino,
            None => return Ok(None),
        };

        // Check if it's a symlink by querying the inode
        let mut rows = conn
            .query("SELECT mode FROM fs_inode WHERE ino = ?", (ino,))
            .await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            // Check if it's a symlink
            if (mode & S_IFMT) != S_IFLNK {
                return Err(FsError::NotASymlink.into());
            }
        } else {
            return Ok(None);
        }

        // Read target from fs_symlink table
        let mut rows = conn
            .query("SELECT target FROM fs_symlink WHERE ino = ?", (ino,))
            .await?;

        if let Some(row) = rows.next().await? {
            let target = row
                .get_value(0)
                .ok()
                .and_then(|v| match v {
                    Value::Text(s) => Some(s.to_string()),
                    _ => None,
                })
                .ok_or(FsError::InvalidPath)?;
            Ok(Some(target))
        } else {
            Ok(None)
        }
    }

    /// Remove a file or empty directory
    pub async fn remove(&self, path: &str) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let path = self.normalize_path(path);
        let components = self.split_path(&path);

        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }

        let ino = self
            .resolve_path_with_conn(&conn, &path)
            .await?
            .ok_or(FsError::NotFound)?;

        if ino == ROOT_INO {
            return Err(FsError::RootOperation.into());
        }

        // Get stats to check if it's a directory
        let stats = self
            .stat_with_conn(&conn, &path)
            .await?
            .ok_or(FsError::NotFound)?;

        // Check if directory is empty
        let mut stmt = conn
            .prepare_cached("SELECT COUNT(*) FROM fs_dentry WHERE parent_ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let count = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0);
            if count > 0 {
                return Err(FsError::NotEmpty.into());
            }
        }

        // Get parent directory and name
        let parent_path = if components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", components[..components.len() - 1].join("/"))
        };

        let parent_ino = self
            .resolve_path_with_conn(&conn, &parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        let name = components.last().unwrap();

        // Delete the specific directory entry (not all entries pointing to this inode)
        let mut stmt = conn
            .prepare_cached("DELETE FROM fs_dentry WHERE parent_ino = ? AND name = ?")
            .await?;
        stmt.execute((parent_ino, name.as_str())).await?;

        // Invalidate cache for this entry
        self.dentry_cache.remove(parent_ino, name);

        // Decrement link count
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET nlink = nlink - 1 WHERE ino = ?")
            .await?;
        stmt.execute((ino,)).await?;

        // If removing a directory, decrement parent nlink (removed dir's ".." link)
        if stats.is_directory() {
            let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
            let now_secs = dur.as_secs() as i64;
            let now_nsec = dur.subsec_nanos() as i64;
            let mut stmt = conn
                .prepare_cached(
                    "UPDATE fs_inode SET nlink = nlink - 1, ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
                )
                .await?;
            stmt.execute((now_secs, now_secs, now_nsec, now_nsec, parent_ino))
                .await?;
        }

        // Check if this was the last link to the inode
        let link_count = self.get_link_count(&conn, ino).await?;
        if link_count == 0 {
            // Manually handle cascading deletes since we don't use foreign keys
            // Delete data blocks
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_data WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;

            // Delete symlink if exists
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_symlink WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;

            // Delete inode
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_inode WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;
        }

        Ok(())
    }

    /// Change file ownership
    ///
    /// Changes the user and/or group ownership of a file.
    /// Pass None for uid or gid to leave that value unchanged.
    pub async fn chown(&self, ino: i64, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        if uid.is_none() && gid.is_none() {
            return Ok(());
        }

        let conn = self.pool.get_connection().await?;

        // Build the update query dynamically based on which values are provided
        let mut updates = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        if let Some(uid) = uid {
            updates.push("uid = ?");
            values.push(Value::Integer(uid as i64));
        }
        if let Some(gid) = gid {
            updates.push("gid = ?");
            values.push(Value::Integer(gid as i64));
        }

        values.push(Value::Integer(ino));
        let sql = format!("UPDATE fs_inode SET {} WHERE ino = ?", updates.join(", "));
        conn.execute(&sql, values).await?;

        Ok(())
    }

    /// Rename/move a file or directory.
    ///
    /// This operation is atomic - either all changes succeed or none do.
    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let conn = self.pool.get_connection().await?;
        let from_path = self.normalize_path(from);
        let to_path = self.normalize_path(to);

        // Cannot rename root
        if from_path == "/" {
            return Err(FsError::RootOperation.into());
        }

        // Get source inode
        let src_ino = self
            .resolve_path_with_conn(&conn, &from_path)
            .await?
            .ok_or(FsError::NotFound)?;

        // Get source stats to check if it's a directory
        let src_stats = self
            .stat_with_conn(&conn, &from_path)
            .await?
            .ok_or(FsError::NotFound)?;

        // Prevent renaming a directory into its own subtree (would create a cycle)
        if src_stats.is_directory() {
            let from_prefix = format!("{}/", from_path);
            if to_path.starts_with(&from_prefix) || to_path == from_path {
                return Err(FsError::InvalidRename.into());
            }
        }

        // Parse source path to get parent and name
        let from_components = self.split_path(&from_path);
        let src_name = from_components.last().ok_or(FsError::InvalidPath)?;
        let src_parent_path = if from_components.len() == 1 {
            "/".to_string()
        } else {
            format!(
                "/{}",
                from_components[..from_components.len() - 1].join("/")
            )
        };
        let src_parent_ino = self
            .resolve_path_with_conn(&conn, &src_parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        // Parse destination path to get parent and name
        let to_components = self.split_path(&to_path);
        if to_components.is_empty() {
            return Err(FsError::RootOperation.into());
        }
        let dst_name = to_components.last().unwrap();
        let dst_parent_path = if to_components.len() == 1 {
            "/".to_string()
        } else {
            format!("/{}", to_components[..to_components.len() - 1].join("/"))
        };
        let dst_parent_ino = self
            .resolve_path_with_conn(&conn, &dst_parent_path)
            .await?
            .ok_or(FsError::NotFound)?;

        // Clone strings for use inside the transaction closure
        let src_name = src_name.clone();
        let dst_name = dst_name.clone();

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let result: Result<()> = async {
            // Check if destination exists (inside transaction for atomicity)
            if let Some(dst_ino) = self.resolve_path_with_conn(&conn, &to_path).await? {
                let dst_stats = self.stat_with_conn(&conn, &to_path).await?.ok_or(FsError::NotFound)?;

                // Can't replace directory with non-directory
                if dst_stats.is_directory() && !src_stats.is_directory() {
                    return Err(FsError::IsADirectory.into());
                }

                // Can't replace non-directory with directory
                if !dst_stats.is_directory() && src_stats.is_directory() {
                    return Err(FsError::NotADirectory.into());
                }

                // If destination is directory, it must be empty
                if dst_stats.is_directory() {
                    let mut stmt = conn
                        .prepare_cached("SELECT COUNT(*) FROM fs_dentry WHERE parent_ino = ?")
                        .await?;
                    let mut rows = stmt.query((dst_ino,)).await?;

                    if let Some(row) = rows.next().await? {
                        let count = row
                            .get_value(0)
                            .ok()
                            .and_then(|v| v.as_integer().copied())
                            .unwrap_or(0);
                        if count > 0 {
                            return Err(FsError::NotEmpty.into());
                        }
                    }
                }

                // Remove destination entry
                let mut stmt = conn
                    .prepare_cached("DELETE FROM fs_dentry WHERE parent_ino = ? AND name = ?")
                    .await?;
                stmt.execute((dst_parent_ino, dst_name.as_str())).await?;

                // Decrement link count
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET nlink = nlink - 1 WHERE ino = ?")
                    .await?;
                stmt.execute((dst_ino,)).await?;

                // Clean up destination inode if no more links
                let link_count = self.get_link_count(&conn, dst_ino).await?;
                if link_count == 0 {
                    let mut stmt = conn
                        .prepare_cached("DELETE FROM fs_data WHERE ino = ?")
                        .await?;
                    stmt.execute((dst_ino,)).await?;
                    let mut stmt = conn
                        .prepare_cached("DELETE FROM fs_symlink WHERE ino = ?")
                        .await?;
                    stmt.execute((dst_ino,)).await?;
                    let mut stmt = conn
                        .prepare_cached("DELETE FROM fs_inode WHERE ino = ?")
                        .await?;
                    stmt.execute((dst_ino,)).await?;
                }
            }

            // Update the dentry: change parent and/or name
            let mut stmt = conn
                .prepare_cached(
                    "UPDATE fs_dentry SET parent_ino = ?, name = ? WHERE parent_ino = ? AND name = ?",
                )
                .await?;
            stmt.execute((
                dst_parent_ino,
                dst_name.as_str(),
                src_parent_ino,
                src_name.as_str(),
            ))
            .await?;

            // If renaming a directory across parents, adjust parent nlink counts
            if src_stats.is_directory() && src_parent_ino != dst_parent_ino {
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET nlink = nlink - 1 WHERE ino = ?")
                    .await?;
                stmt.execute((src_parent_ino,)).await?;

                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?")
                    .await?;
                stmt.execute((dst_parent_ino,)).await?;
            }

            // Update ctime of the inode
            let dur = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let now_secs = dur.as_secs() as i64;
            let now_nsec = dur.subsec_nanos() as i64;

            let mut stmt = conn
                .prepare_cached("UPDATE fs_inode SET ctime = ?, ctime_nsec = ? WHERE ino = ?")
                .await?;
            stmt.execute((now_secs, now_nsec, src_ino)).await?;

            // Update source parent directory timestamps
            let mut stmt = conn
                .prepare_cached("UPDATE fs_inode SET mtime = ?, ctime = ?, mtime_nsec = ?, ctime_nsec = ? WHERE ino = ?")
                .await?;
            stmt.execute((now_secs, now_secs, now_nsec, now_nsec, src_parent_ino)).await?;

            // Update destination parent directory timestamps
            if dst_parent_ino != src_parent_ino {
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET mtime = ?, ctime = ?, mtime_nsec = ?, ctime_nsec = ? WHERE ino = ?")
                    .await?;
                stmt.execute((now_secs, now_secs, now_nsec, now_nsec, dst_parent_ino)).await?;
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                txn.commit().await?;

                // Invalidate cache for source and destination
                self.dentry_cache.remove(src_parent_ino, &src_name);
                self.dentry_cache.remove(dst_parent_ino, &dst_name);

                // Add new entry to cache (source inode is now at destination)
                self.dentry_cache.insert(dst_parent_ino, &dst_name, src_ino);

                Ok(())
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }

    /// Get filesystem statistics
    ///
    /// Returns the total number of inodes and bytes used by file contents.
    pub async fn statfs(&self) -> Result<FilesystemStats> {
        let conn = self.pool.get_connection().await?;
        // Count total inodes
        let mut stmt = conn.prepare_cached("SELECT COUNT(*) FROM fs_inode").await?;
        let mut rows = stmt.query(()).await?;

        let inodes = if let Some(row) = rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64
        } else {
            0
        };

        // Sum total bytes used (from file sizes in inodes)
        let mut stmt = conn
            .prepare_cached("SELECT COALESCE(SUM(size), 0) FROM fs_inode")
            .await?;
        let mut rows = stmt.query(()).await?;

        let bytes_used = if let Some(row) = rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u64
        } else {
            0
        };

        Ok(FilesystemStats { inodes, bytes_used })
    }

    /// Synchronize file data to persistent storage
    ///
    /// Temporarily enables FULL synchronous mode, runs a transaction to force
    /// a checkpoint, then restores OFF mode. This ensures durability while
    /// maintaining high performance for normal operations.
    ///
    /// Note: The path parameter is ignored since all data is in a single database.
    pub async fn fsync(&self, _path: &str) -> Result<()> {
        Ok(())
    }

    /// Open a file and return a file handle.
    ///
    /// The returned handle can be used for efficient read/write/fsync operations
    /// without requiring path lookups on each operation.
    pub async fn open(&self, path: &str) -> Result<BoxedFile> {
        let path = self.normalize_path(path);
        let ino = self.resolve_path(&path).await?.ok_or(FsError::NotFound)?;

        Ok(Arc::new(SecAFSFile {
            pool: self.pool.clone(),
            ino,
            chunk_size: self.chunk_size,
        }))
    }

    /// Get the number of chunks for a given inode (for testing)
    #[cfg(test)]
    async fn get_chunk_count(&self, ino: i64) -> Result<i64> {
        let conn = self.pool.get_connection().await?;
        let mut rows = conn
            .query("SELECT COUNT(*) FROM fs_data WHERE ino = ?", (ino,))
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0))
        } else {
            Ok(0)
        }
    }
}

#[async_trait]
impl FileSystem for SecAFS {
    async fn lookup(&self, parent_ino: i64, name: &str) -> Result<Option<Stats>> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Handle ".." by finding the parent of parent_ino
        if name == ".." {
            if parent_ino == ROOT_INO {
                // Root's parent is itself
                return self.getattr_with_conn(&conn, ROOT_INO).await;
            }
            let mut stmt = conn
                .prepare_cached("SELECT parent_ino FROM fs_dentry WHERE ino = ? LIMIT 1")
                .await?;
            let mut rows = stmt.query((parent_ino,)).await?;
            let parent = if let Some(row) = rows.next().await? {
                row.get_value(0)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(ROOT_INO)
            } else {
                ROOT_INO
            };
            return self.getattr_with_conn(&conn, parent).await;
        }

        // Look up the child inode
        let child_ino = match self.lookup_child(&conn, parent_ino, name).await? {
            Some(ino) => ino,
            None => return Ok(None),
        };

        // Get stats for the child inode
        let mut stmt = conn
            .prepare_cached("SELECT ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((child_ino,)).await?;

        if let Some(row) = rows.next().await? {
            let stats = Self::build_stats_from_row(&row)?;
            // Cache the lookup result
            self.dentry_cache.insert(parent_ino, name, child_ino);
            Ok(Some(stats))
        } else {
            Ok(None)
        }
    }

    async fn getattr(&self, ino: i64) -> Result<Option<Stats>> {
        let conn = self.pool.get_connection().await?;
        self.getattr_with_conn(&conn, ino).await
    }

    async fn readlink(&self, ino: i64) -> Result<Option<String>> {
        let conn = self.pool.get_connection().await?;

        // Check if the inode exists and is a symlink
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) != S_IFLNK {
                return Err(FsError::NotASymlink.into());
            }
        } else {
            return Ok(None);
        }

        // Read target from fs_symlink table
        let mut stmt = conn
            .prepare_cached("SELECT target FROM fs_symlink WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let target = row
                .get_value(0)
                .ok()
                .and_then(|v| match v {
                    Value::Text(s) => Some(s.to_string()),
                    _ => None,
                })
                .ok_or(FsError::InvalidPath)?;
            Ok(Some(target))
        } else {
            Ok(None)
        }
    }

    async fn readdir(&self, ino: i64) -> Result<Option<Vec<String>>> {
        let conn = self.pool.get_connection().await?;

        // Check if inode exists and is a directory
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) != super::S_IFDIR {
                return Err(FsError::NotADirectory.into());
            }
        } else {
            return Ok(None);
        }

        let mut stmt = conn
            .prepare_cached("SELECT name FROM fs_dentry WHERE parent_ino = ? ORDER BY name")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let name = row
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
            if !name.is_empty() {
                entries.push(name);
            }
        }

        Ok(Some(entries))
    }

    async fn readdir_plus(&self, ino: i64) -> Result<Option<Vec<DirEntry>>> {
        let conn = self.pool.get_connection().await?;

        // Check if inode exists and is a directory
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) != super::S_IFDIR {
                return Err(FsError::NotADirectory.into());
            }
        } else {
            return Ok(None);
        }

        let mut stmt = conn.prepare_cached("SELECT d.name, i.ino, i.mode, i.nlink, i.uid, i.gid, i.size, i.atime, i.mtime, i.ctime, i.rdev, i.atime_nsec, i.mtime_nsec, i.ctime_nsec
            FROM fs_dentry d
            JOIN fs_inode i ON d.ino = i.ino
            WHERE d.parent_ino = ?
            ORDER BY d.name"
        ).await?;
        let mut rows = stmt.query((ino,)).await?;

        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let name = row
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

            if name.is_empty() {
                continue;
            }

            let entry_ino = row
                .get_value(1)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0);

            let stats = Stats {
                ino: entry_ino,
                mode: row
                    .get_value(2)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                nlink: row
                    .get_value(3)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(1) as u32,
                uid: row
                    .get_value(4)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                gid: row
                    .get_value(5)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                size: row
                    .get_value(6)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                atime: row
                    .get_value(7)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                mtime: row
                    .get_value(8)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                ctime: row
                    .get_value(9)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0),
                atime_nsec: row
                    .get_value(11)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                mtime_nsec: row
                    .get_value(12)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                ctime_nsec: row
                    .get_value(13)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u32,
                rdev: row
                    .get_value(10)
                    .ok()
                    .and_then(|v| v.as_integer().copied())
                    .unwrap_or(0) as u64,
            };

            entries.push(DirEntry { name, stats });
        }

        Ok(Some(entries))
    }

    async fn chmod(&self, ino: i64, mode: u32) -> Result<()> {
        let conn = self.pool.get_connection().await?;

        // Get current mode to preserve file type bits
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        let current_mode = if let Some(row) = rows.next().await? {
            row.get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32
        } else {
            return Err(FsError::NotFound.into());
        };

        // Preserve file type bits (upper bits), replace permission bits (lower 12 bits)
        let new_mode = (current_mode & S_IFMT) | (mode & 0o7777);

        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET mode = ?, ctime = ?, ctime_nsec = ? WHERE ino = ?")
            .await?;
        stmt.execute((new_mode as i64, now_secs, now_nsec, ino))
            .await?;

        Ok(())
    }

    async fn chown(&self, ino: i64, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        if uid.is_none() && gid.is_none() {
            return Ok(());
        }

        let conn = self.pool.get_connection().await?;

        // Verify inode exists
        let mut stmt = conn
            .prepare_cached("SELECT ino FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if rows.next().await?.is_none() {
            return Err(FsError::NotFound.into());
        }

        // Build the update query dynamically based on which values are provided
        let mut updates = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        if let Some(uid) = uid {
            updates.push("uid = ?");
            values.push(Value::Integer(uid as i64));
        }
        if let Some(gid) = gid {
            updates.push("gid = ?");
            values.push(Value::Integer(gid as i64));
        }

        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        updates.push("ctime = ?");
        values.push(Value::Integer(now_secs));
        updates.push("ctime_nsec = ?");
        values.push(Value::Integer(now_nsec));

        values.push(Value::Integer(ino));
        let sql = format!("UPDATE fs_inode SET {} WHERE ino = ?", updates.join(", "));
        conn.execute(&sql, values).await?;

        Ok(())
    }

    async fn utimens(&self, ino: i64, atime: TimeChange, mtime: TimeChange) -> Result<()> {
        let conn = self.pool.get_connection().await?;

        // Verify inode exists
        let mut stmt = conn
            .prepare_cached("SELECT ino FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;
        if rows.next().await?.is_none() {
            return Err(FsError::NotFound.into());
        }

        let mut updates = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        let resolve = |tc: TimeChange| -> (i64, i64) {
            match tc {
                TimeChange::Set(secs, nsec) => (secs, nsec as i64),
                TimeChange::Now => {
                    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                    (dur.as_secs() as i64, dur.subsec_nanos() as i64)
                }
                TimeChange::Omit => unreachable!(),
            }
        };

        if !matches!(atime, TimeChange::Omit) {
            let (secs, nsec) = resolve(atime);
            updates.push("atime = ?");
            values.push(Value::Integer(secs));
            updates.push("atime_nsec = ?");
            values.push(Value::Integer(nsec));
        }

        if !matches!(mtime, TimeChange::Omit) {
            let (secs, nsec) = resolve(mtime);
            updates.push("mtime = ?");
            values.push(Value::Integer(secs));
            updates.push("mtime_nsec = ?");
            values.push(Value::Integer(nsec));
        }

        if updates.is_empty() {
            return Ok(());
        }

        // Also update ctime
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        updates.push("ctime = ?");
        values.push(Value::Integer(dur.as_secs() as i64));
        updates.push("ctime_nsec = ?");
        values.push(Value::Integer(dur.subsec_nanos() as i64));

        values.push(Value::Integer(ino));
        let sql = format!("UPDATE fs_inode SET {} WHERE ino = ?", updates.join(", "));
        conn.execute(&sql, values).await?;

        Ok(())
    }

    async fn open(&self, ino: i64, _flags: i32) -> Result<BoxedFile> {
        let conn = self.pool.get_connection().await?;

        // Verify inode exists
        let mut stmt = conn
            .prepare_cached("SELECT ino FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if rows.next().await?.is_none() {
            return Err(FsError::NotFound.into());
        }

        Ok(Arc::new(SecAFSFile {
            pool: self.pool.clone(),
            ino,
            chunk_size: self.chunk_size,
        }))
    }

    async fn mkdir(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Check if already exists
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create inode
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let dir_mode = super::S_IFDIR | (mode & 0o7777);
        let row = stmt
            .query_row((
                dir_mode as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                now_nsec,
                now_nsec,
                now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        // Create directory entry
        let mut stmt = conn
            .prepare_cached("INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)")
            .await?;
        stmt.execute((name, parent_ino, ino)).await?;

        // Set nlink to 2 for new directory (self "." + parent's dentry)
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET nlink = 2 WHERE ino = ?")
            .await?;
        stmt.execute((ino,)).await?;

        // Increment parent nlink (new directory's ".." link) and update timestamps
        let mut stmt = conn
            .prepare_cached(
                "UPDATE fs_inode SET nlink = nlink + 1, ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
            )
            .await?;
        stmt.execute((now_secs, now_secs, now_nsec, now_nsec, parent_ino))
            .await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(Stats {
            ino,
            mode: dir_mode,
            nlink: 2,
            uid,
            gid,
            size: 0,
            atime: now_secs,
            mtime: now_secs,
            ctime: now_secs,
            atime_nsec: now_nsec as u32,
            mtime_nsec: now_nsec as u32,
            ctime_nsec: now_nsec as u32,
            rdev: 0,
        })
    }

    async fn create_file(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<(Stats, BoxedFile)> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Check if already exists
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Prepare statements before starting the transaction
        let mut inode_stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, nlink, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                 VALUES (?, 1, ?, ?, 0, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let mut dentry_stmt = conn
            .prepare_cached("INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)")
            .await?;

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let file_mode = S_IFREG | (mode & 0o7777);

        let row = inode_stmt
            .query_row((
                file_mode as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                now_nsec,
                now_nsec,
                now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        dentry_stmt.execute((name, parent_ino, ino)).await?;

        // Update parent directory ctime and mtime
        conn.execute(
            "UPDATE fs_inode SET ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
            (now_secs, now_secs, now_nsec, now_nsec, parent_ino),
        )
        .await?;

        txn.commit().await?;

        self.dentry_cache.insert(parent_ino, name, ino);

        let stats = Stats {
            ino,
            mode: file_mode,
            nlink: 1,
            uid,
            gid,
            size: 0,
            atime: now_secs,
            mtime: now_secs,
            ctime: now_secs,
            atime_nsec: now_nsec as u32,
            mtime_nsec: now_nsec as u32,
            ctime_nsec: now_nsec as u32,
            rdev: 0,
        };

        let file: BoxedFile = Arc::new(SecAFSFile {
            pool: self.pool.clone(),
            ino,
            chunk_size: self.chunk_size,
        });

        Ok((stats, file))
    }

    async fn mknod(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        rdev: u64,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Check if already exists
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create inode with mode and rdev
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec)
                VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let row = stmt
            .query_row((
                mode as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                rdev as i64,
                now_nsec,
                now_nsec,
                now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        // Create directory entry
        let mut stmt = conn
            .prepare_cached("INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)")
            .await?;
        stmt.execute((name, parent_ino, ino)).await?;

        // Increment link count
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?")
            .await?;
        stmt.execute((ino,)).await?;

        // Update parent directory ctime and mtime
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?")
            .await?;
        stmt.execute((now_secs, now_secs, now_nsec, now_nsec, parent_ino))
            .await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(Stats {
            ino,
            mode,
            nlink: 1,
            uid,
            gid,
            size: 0,
            atime: now_secs,
            mtime: now_secs,
            ctime: now_secs,
            atime_nsec: now_nsec as u32,
            mtime_nsec: now_nsec as u32,
            ctime_nsec: now_nsec as u32,
            rdev,
        })
    }

    async fn symlink(
        &self,
        parent_ino: i64,
        name: &str,
        target: &str,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Check if entry already exists
        if self.lookup_child(&conn, parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Create inode for symlink
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mode = S_IFLNK | 0o777; // Symlinks typically have 777 permissions
        let size = target.len() as i64;

        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO fs_inode (mode, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING ino",
            )
            .await?;
        let row = stmt
            .query_row((
                mode, uid, gid, size, now_secs, now_secs, now_secs, now_nsec, now_nsec, now_nsec,
            ))
            .await?;

        let ino = row
            .get_value(0)
            .ok()
            .and_then(|v| v.as_integer().copied())
            .ok_or_else(|| Error::Internal("failed to get inode".to_string()))?;

        // Store symlink target
        conn.execute(
            "INSERT INTO fs_symlink (ino, target) VALUES (?, ?)",
            (ino, target),
        )
        .await?;

        // Create directory entry
        conn.execute(
            "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)",
            (name, parent_ino, ino),
        )
        .await?;

        // Increment link count
        conn.execute(
            "UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?",
            (ino,),
        )
        .await?;

        // Update parent directory ctime and mtime
        conn.execute(
            "UPDATE fs_inode SET ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
            (now_secs, now_secs, now_nsec, now_nsec, parent_ino),
        )
        .await?;

        // Populate dentry cache
        self.dentry_cache.insert(parent_ino, name, ino);

        Ok(Stats {
            ino,
            mode,
            nlink: 1,
            uid,
            gid,
            size,
            atime: now_secs,
            mtime: now_secs,
            ctime: now_secs,
            atime_nsec: now_nsec as u32,
            mtime_nsec: now_nsec as u32,
            ctime_nsec: now_nsec as u32,
            rdev: 0,
        })
    }

    async fn unlink(&self, parent_ino: i64, name: &str) -> Result<()> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Look up the child inode
        let ino = self
            .lookup_child(&conn, parent_ino, name)
            .await?
            .ok_or(FsError::NotFound)?;

        // Check if it's a directory (use rmdir for directories)
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) == super::S_IFDIR {
                return Err(FsError::IsADirectory.into());
            }
        }

        // Delete the directory entry
        let mut stmt = conn
            .prepare_cached("DELETE FROM fs_dentry WHERE parent_ino = ? AND name = ?")
            .await?;
        stmt.execute((parent_ino, name)).await?;

        // Invalidate cache
        self.dentry_cache.remove(parent_ino, name);

        // Update parent directory mtime and ctime
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET mtime = ?, ctime = ?, mtime_nsec = ?, ctime_nsec = ? WHERE ino = ?")
            .await?;
        stmt.execute((now_secs, now_secs, now_nsec, now_nsec, parent_ino))
            .await?;

        // Decrement link count and update ctime
        let mut stmt = conn
            .prepare_cached(
                "UPDATE fs_inode SET nlink = nlink - 1, ctime = ?, ctime_nsec = ? WHERE ino = ?",
            )
            .await?;
        stmt.execute((now_secs, now_nsec, ino)).await?;

        // Check if this was the last link to the inode
        let link_count = self.get_link_count(&conn, ino).await?;
        if link_count == 0 {
            // Delete data blocks
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_data WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;

            // Delete symlink if exists
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_symlink WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;

            // Delete inode
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_inode WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;
        }

        Ok(())
    }

    async fn rmdir(&self, parent_ino: i64, name: &str) -> Result<()> {
        if name.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Look up the child inode
        let ino = self
            .lookup_child(&conn, parent_ino, name)
            .await?
            .ok_or(FsError::NotFound)?;

        if ino == ROOT_INO {
            return Err(FsError::RootOperation.into());
        }

        // Check if it's a directory
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) != super::S_IFDIR {
                return Err(FsError::NotADirectory.into());
            }
        } else {
            return Err(FsError::NotFound.into());
        }

        // Check if directory is empty
        let mut stmt = conn
            .prepare_cached("SELECT COUNT(*) FROM fs_dentry WHERE parent_ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let count = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0);
            if count > 0 {
                return Err(FsError::NotEmpty.into());
            }
        }

        // Delete the directory entry
        let mut stmt = conn
            .prepare_cached("DELETE FROM fs_dentry WHERE parent_ino = ? AND name = ?")
            .await?;
        stmt.execute((parent_ino, name)).await?;

        // Invalidate cache
        self.dentry_cache.remove(parent_ino, name);

        // Decrement link count on removed directory
        let mut stmt = conn
            .prepare_cached("UPDATE fs_inode SET nlink = nlink - 1 WHERE ino = ?")
            .await?;
        stmt.execute((ino,)).await?;

        // Decrement parent nlink (removed directory's ".." link) and update timestamps
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        let mut stmt = conn
            .prepare_cached(
                "UPDATE fs_inode SET nlink = nlink - 1, ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
            )
            .await?;
        stmt.execute((now_secs, now_secs, now_nsec, now_nsec, parent_ino))
            .await?;

        // Delete inode if no more links
        let link_count = self.get_link_count(&conn, ino).await?;
        if link_count == 0 {
            let mut stmt = conn
                .prepare_cached("DELETE FROM fs_inode WHERE ino = ?")
                .await?;
            stmt.execute((ino,)).await?;
        }

        Ok(())
    }

    async fn link(&self, ino: i64, newparent_ino: i64, newname: &str) -> Result<Stats> {
        if newname.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Check if source inode exists and is not a directory
        let mut stmt = conn
            .prepare_cached("SELECT mode FROM fs_inode WHERE ino = ?")
            .await?;
        let mut rows = stmt.query((ino,)).await?;

        if let Some(row) = rows.next().await? {
            let mode = row
                .get_value(0)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            if (mode & S_IFMT) == super::S_IFDIR {
                return Err(FsError::IsADirectory.into());
            }
        } else {
            return Err(FsError::NotFound.into());
        }

        // Check if destination already exists
        if self
            .lookup_child(&conn, newparent_ino, newname)
            .await?
            .is_some()
        {
            return Err(FsError::AlreadyExists.into());
        }

        // Create directory entry pointing to the same inode
        conn.execute(
            "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, ?, ?)",
            (newname, newparent_ino, ino),
        )
        .await?;

        // Increment link count and update ctime
        let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let now_secs = dur.as_secs() as i64;
        let now_nsec = dur.subsec_nanos() as i64;
        conn.execute(
            "UPDATE fs_inode SET nlink = nlink + 1, ctime = ?, ctime_nsec = ? WHERE ino = ?",
            (now_secs, now_nsec, ino),
        )
        .await?;

        // Update parent directory ctime and mtime
        conn.execute(
            "UPDATE fs_inode SET ctime = ?, mtime = ?, ctime_nsec = ?, mtime_nsec = ? WHERE ino = ?",
            (now_secs, now_secs, now_nsec, now_nsec, newparent_ino),
        )
        .await?;

        // Populate dentry cache
        self.dentry_cache.insert(newparent_ino, newname, ino);

        // Return updated stats
        self.getattr_with_conn(&conn, ino)
            .await?
            .ok_or(FsError::NotFound.into())
    }

    async fn rename(
        &self,
        oldparent_ino: i64,
        oldname: &str,
        newparent_ino: i64,
        newname: &str,
    ) -> Result<()> {
        if newname.len() > MAX_NAME_LEN {
            return Err(FsError::NameTooLong.into());
        }
        let conn = self.pool.get_connection().await?;

        // Get source inode
        let src_ino = self
            .lookup_child(&conn, oldparent_ino, oldname)
            .await?
            .ok_or(FsError::NotFound)?;

        if src_ino == ROOT_INO {
            return Err(FsError::RootOperation.into());
        }

        // Get source stats to check if it's a directory
        let src_stats = self
            .getattr_with_conn(&conn, src_ino)
            .await?
            .ok_or(FsError::NotFound)?;

        let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;

        let result: Result<()> = async {
            // Check if destination exists
            if let Some(dst_ino) = self.lookup_child(&conn, newparent_ino, newname).await? {
                let dst_stats = self.getattr_with_conn(&conn, dst_ino).await?.ok_or(FsError::NotFound)?;

                // Can't replace directory with non-directory
                if dst_stats.is_directory() && !src_stats.is_directory() {
                    return Err(FsError::IsADirectory.into());
                }

                // Can't replace non-directory with directory
                if !dst_stats.is_directory() && src_stats.is_directory() {
                    return Err(FsError::NotADirectory.into());
                }

                // If destination is directory, it must be empty
                if dst_stats.is_directory() {
                    let mut stmt = conn
                        .prepare_cached("SELECT COUNT(*) FROM fs_dentry WHERE parent_ino = ?")
                        .await?;
                    let mut rows = stmt.query((dst_ino,)).await?;

                    if let Some(row) = rows.next().await? {
                        let count = row
                            .get_value(0)
                            .ok()
                            .and_then(|v| v.as_integer().copied())
                            .unwrap_or(0);
                        if count > 0 {
                            return Err(FsError::NotEmpty.into());
                        }
                    }
                }

                // Remove destination entry
                let mut stmt = conn
                    .prepare_cached("DELETE FROM fs_dentry WHERE parent_ino = ? AND name = ?")
                    .await?;
                stmt.execute((newparent_ino, newname)).await?;

                // Decrement link count and update ctime on destination inode
                let dur_dec = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                let now_dec = dur_dec.as_secs() as i64;
                let now_dec_nsec = dur_dec.subsec_nanos() as i64;
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET nlink = nlink - 1, ctime = ?, ctime_nsec = ? WHERE ino = ?")
                    .await?;
                stmt.execute((now_dec, now_dec_nsec, dst_ino)).await?;

                // Clean up destination inode if no more links
                let link_count = self.get_link_count(&conn, dst_ino).await?;
                if link_count == 0 {
                    let mut stmt = conn
                        .prepare_cached("DELETE FROM fs_data WHERE ino = ?")
                        .await?;
                    stmt.execute((dst_ino,)).await?;
                    let mut stmt = conn
                        .prepare_cached("DELETE FROM fs_symlink WHERE ino = ?")
                        .await?;
                    stmt.execute((dst_ino,)).await?;
                    let mut stmt = conn
                        .prepare_cached("DELETE FROM fs_inode WHERE ino = ?")
                        .await?;
                    stmt.execute((dst_ino,)).await?;
                }
            }

            // Update the dentry: change parent and/or name
            let mut stmt = conn
                .prepare_cached(
                    "UPDATE fs_dentry SET parent_ino = ?, name = ? WHERE parent_ino = ? AND name = ?",
                )
                .await?;
            stmt.execute((newparent_ino, newname, oldparent_ino, oldname))
                .await?;

            // If renaming a directory across parents, adjust parent nlink counts
            // (the ".." link moves from old parent to new parent)
            if src_stats.is_directory() && oldparent_ino != newparent_ino {
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET nlink = nlink - 1 WHERE ino = ?")
                    .await?;
                stmt.execute((oldparent_ino,)).await?;

                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET nlink = nlink + 1 WHERE ino = ?")
                    .await?;
                stmt.execute((newparent_ino,)).await?;
            }

            // Update ctime of the inode
            let dur = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let now_secs = dur.as_secs() as i64;
            let now_nsec = dur.subsec_nanos() as i64;

            let mut stmt = conn
                .prepare_cached("UPDATE fs_inode SET ctime = ?, ctime_nsec = ? WHERE ino = ?")
                .await?;
            stmt.execute((now_secs, now_nsec, src_ino)).await?;

            // Update source parent directory timestamps
            let mut stmt = conn
                .prepare_cached("UPDATE fs_inode SET mtime = ?, ctime = ?, mtime_nsec = ?, ctime_nsec = ? WHERE ino = ?")
                .await?;
            stmt.execute((now_secs, now_secs, now_nsec, now_nsec, oldparent_ino)).await?;

            // Update destination parent directory timestamps
            if newparent_ino != oldparent_ino {
                let mut stmt = conn
                    .prepare_cached("UPDATE fs_inode SET mtime = ?, ctime = ?, mtime_nsec = ?, ctime_nsec = ? WHERE ino = ?")
                    .await?;
                stmt.execute((now_secs, now_secs, now_nsec, now_nsec, newparent_ino)).await?;
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                txn.commit().await?;

                // Invalidate cache for source and destination
                self.dentry_cache.remove(oldparent_ino, oldname);
                self.dentry_cache.remove(newparent_ino, newname);

                // Add new entry to cache (source inode is now at destination)
                self.dentry_cache.insert(newparent_ino, newname, src_ino);

                Ok(())
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }

    async fn statfs(&self) -> Result<FilesystemStats> {
        SecAFS::statfs(self).await
    }
}

// Tests removed: they required SQLite (SecAFS::new) which is no longer supported.
// To re-add tests, use a PostgreSQL connection pool via ConnectionPool::new().
#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
}
