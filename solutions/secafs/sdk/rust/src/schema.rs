//! Schema versioning and detection for SecAFS databases.

use crate::db::{DbConn, DbValue};
use crate::error::{Error, Result};

/// Current schema version.
pub const SECAFS_SCHEMA_VERSION: &str = "0.6";

/// Detected schema version based on column introspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaVersion {
    /// Base schema: fs_inode, fs_dentry, fs_data, fs_symlink, fs_config, kv_store, tool_calls
    V0_0,
    /// Added nlink column to fs_inode
    V0_2,
    /// Added atime_nsec, mtime_nsec, ctime_nsec, rdev columns to fs_inode
    V0_4,
    /// Added fs_volumes table for per-conversation FUSE roots
    V0_5,
    /// Added fs_volume_state table for Copy-on-Write rollback
    V0_6,
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaVersion::V0_0 => write!(f, "0.0"),
            SchemaVersion::V0_2 => write!(f, "0.2"),
            SchemaVersion::V0_4 => write!(f, "0.4"),
            SchemaVersion::V0_5 => write!(f, "0.5"),
            SchemaVersion::V0_6 => write!(f, "0.6"),
        }
    }
}

impl SchemaVersion {
    /// Returns the version string.
    pub fn as_str(&self) -> &'static str {
        match self {
            SchemaVersion::V0_0 => "0.0",
            SchemaVersion::V0_2 => "0.2",
            SchemaVersion::V0_4 => "0.4",
            SchemaVersion::V0_5 => "0.5",
            SchemaVersion::V0_6 => "0.6",
        }
    }

    /// Returns true if this version is the current version.
    pub fn is_current(&self) -> bool {
        matches!(self, SchemaVersion::V0_6)
    }
}

/// Column information from table introspection.
#[derive(Debug)]
struct ColumnInfo {
    name: String,
}

/// Detect the schema version of an existing database by introspecting fs_inode columns.
/// Returns None if the database has no fs_inode table (new database).
pub async fn detect_schema_version(conn: &DbConn) -> Result<Option<SchemaVersion>> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = ?",
            ("fs_inode",),
        )
        .await?;
    let table_exists = rows.next().await?.is_some();

    if !table_exists {
        return Ok(None);
    }

    let columns = get_table_columns(conn, "fs_inode").await?;

    let has_nlink = columns.iter().any(|c| c.name == "nlink");
    let has_atime_nsec = columns.iter().any(|c| c.name == "atime_nsec");
    let has_mtime_nsec = columns.iter().any(|c| c.name == "mtime_nsec");
    let has_ctime_nsec = columns.iter().any(|c| c.name == "ctime_nsec");
    let has_rdev = columns.iter().any(|c| c.name == "rdev");

    // V0_5 adds fs_volumes table; detect by checking its presence
    if has_atime_nsec && has_mtime_nsec && has_ctime_nsec && has_rdev {
        let mut vol_rows = conn
            .query(
                "SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = ?",
                ("fs_volumes",),
            )
            .await?;
        if vol_rows.next().await?.is_some() {
            let mut state_rows = conn
                .query(
                    "SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = ?",
                    ("fs_volume_state",),
                )
                .await?;
            if state_rows.next().await?.is_some() {
                return Ok(Some(SchemaVersion::V0_6));
            }
            return Ok(Some(SchemaVersion::V0_5));
        }
        return Ok(Some(SchemaVersion::V0_4));
    }

    if has_nlink {
        return Ok(Some(SchemaVersion::V0_2));
    }

    Ok(Some(SchemaVersion::V0_0))
}

/// Check that a database has a compatible schema version.
/// Returns Ok(()) for new databases or databases at the current version.
/// Returns Err(SchemaVersionMismatch) for databases with old schemas.
pub async fn check_schema_version(conn: &DbConn) -> Result<()> {
    if let Some(version) = detect_schema_version(conn).await? {
        if !version.is_current() {
            return Err(Error::SchemaVersionMismatch {
                found: version.to_string(),
                expected: SECAFS_SCHEMA_VERSION.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests_v0_6 {
    use super::*;

    #[test]
    fn version_v0_6_is_current_and_str() {
        assert!(SchemaVersion::V0_6.is_current());
        assert_eq!(SchemaVersion::V0_6.as_str(), "0.6");
        assert_eq!(SECAFS_SCHEMA_VERSION, "0.6");
    }
}

/// Get column information for a table using information_schema.
async fn get_table_columns(conn: &DbConn, table_name: &str) -> Result<Vec<ColumnInfo>> {
    let mut rows = conn
        .query(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = ?",
            (table_name,),
        )
        .await?;

    let mut columns = Vec::new();
    while let Some(row) = rows.next().await? {
        let name = match row.get_value(0) {
            Ok(DbValue::Text(s)) => s,
            Ok(DbValue::Integer(i)) => i.to_string(),
            _ => continue,
        };
        columns.push(ColumnInfo { name });
    }

    Ok(columns)
}
