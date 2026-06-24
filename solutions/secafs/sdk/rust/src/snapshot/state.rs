//! Per-volume rollback state operations: enable, disable (always purges),
//! and read-back. All ops are idempotent.

use crate::db::{DbConn, DbValue};
use crate::error::{Error, Result};
use super::types::VolumeRollbackState;

/// Enable rollback for a volume. Idempotent: returns existing state if already
/// enabled; otherwise upserts a row with `rollback_enabled=true` and
/// `current_snap_id=1`.
pub async fn enable(conn: &DbConn, volume_id: &str) -> Result<VolumeRollbackState> {
    conn.execute(
        "INSERT INTO fs_volume_state (volume_id, rollback_enabled, current_snap_id)
         VALUES ($1, TRUE, 1)
         ON CONFLICT (volume_id) DO UPDATE SET rollback_enabled = TRUE",
        (volume_id,),
    ).await?;
    get_state(conn, volume_id).await?
        .ok_or_else(|| Error::Internal("fs_volume_state row missing after upsert".into()))
}

/// Disable rollback for a volume. Per design D5: always purges this
/// volume's existing snapshots and undo rows. Returns counts of pruned rows.
pub async fn disable(conn: &DbConn, volume_id: &str) -> Result<DisableResult> {
    let mut purged_undo: i64 = 0;
    for tbl in ["fs_inode_undo", "fs_dentry_undo", "fs_data_undo", "fs_symlink_undo", "kv_store_undo"] {
        let q = format!("DELETE FROM {tbl} WHERE volume_id = $1");
        purged_undo += conn.execute(&q, (volume_id,)).await? as i64;
    }
    let purged_snapshots = conn
        .execute("DELETE FROM fs_snapshots WHERE volume_id = $1", (volume_id,))
        .await? as i64;
    conn.execute(
        "UPDATE fs_volume_state SET rollback_enabled = FALSE, current_snap_id = 1 WHERE volume_id = $1",
        (volume_id,),
    ).await?;
    Ok(DisableResult { purged_snapshots, purged_undo_rows: purged_undo })
}

#[derive(Debug, Clone, Copy)]
pub struct DisableResult {
    pub purged_snapshots: i64,
    pub purged_undo_rows: i64,
}

/// Read back the rollback state for a volume. Returns `None` if no row exists.
///
/// Note: PG BOOL is cast to INTEGER in the SELECT so that the SDK's DbValue
/// mapping (which only handles i64, f64, Text, Blob) produces `DbValue::Integer`
/// rather than `DbValue::Null` for the `rollback_enabled` column.
pub async fn get_state(conn: &DbConn, volume_id: &str) -> Result<Option<VolumeRollbackState>> {
    let mut rows = conn
        .query(
            "SELECT volume_id, rollback_enabled::int::bigint, current_snap_id \
             FROM fs_volume_state WHERE volume_id = $1",
            (volume_id,),
        )
        .await?;
    if let Some(row) = rows.next().await? {
        let volume_id_val = match row.get_value(0)? {
            DbValue::Text(t) => t,
            _ => return Err(Error::Internal("volume_id: expected TEXT".into())),
        };
        let rollback_enabled = match row.get_value(1)? {
            DbValue::Integer(i) => i != 0,
            _ => return Err(Error::Internal("rollback_enabled: expected INTEGER (cast from BOOL)".into())),
        };
        let current_snap_id = *row.get_value(2)?.as_integer()
            .ok_or_else(|| Error::Internal("current_snap_id: expected INTEGER".into()))?;
        Ok(Some(VolumeRollbackState {
            volume_id: volume_id_val,
            rollback_enabled,
            current_snap_id,
        }))
    } else {
        Ok(None)
    }
}
