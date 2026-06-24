//! Restore a volume to a committed snapshot by replaying typed undo entries
//! in reverse undo_id order across all 5 undo tables.
//!
//! The whole replay runs in a single PG transaction with
//! `SET LOCAL secafs.suppress_undo = 'true'` so the trigger does not
//! re-journal the inverse-op writes.

use crate::db::{DbConn, DbValue};
use crate::error::{Error, Result};
use super::types::UndoOp;

#[derive(Debug, Clone, Copy)]
pub struct RestoreOutcome {
    pub pruned_snapshots: i64,
    pub pruned_undo_rows: i64,
}

/// Roll the volume back to snapshot `target_snap_id`.
pub async fn restore_to(
    conn: &DbConn,
    volume_id: &str,
    target_snap_id: i64,
) -> Result<RestoreOutcome> {
    conn.execute("BEGIN", ()).await?;
    let result = restore_inner(conn, volume_id, target_snap_id).await;
    match &result {
        Ok(_) => { conn.execute("COMMIT", ()).await?; }
        Err(_) => { let _ = conn.execute("ROLLBACK", ()).await; }
    }
    result
}

async fn restore_inner(conn: &DbConn, volume_id: &str, target_snap_id: i64) -> Result<RestoreOutcome> {
    conn.execute("SET LOCAL secafs.suppress_undo = 'true'", ()).await?;

    // Save fs_volumes rows before replaying inodes.  replay_fs_inode deletes
    // inode rows (including the volume root_ino) which CASCADE-deletes
    // fs_volumes due to the FK(root_ino) → fs_inode ON DELETE CASCADE.
    // The same CASCADE then propagates to fs_volume_state and fs_snapshots
    // (both reference fs_volumes.id ON DELETE CASCADE), so we must save and
    // re-insert all three sets of rows after the replay completes.
    let saved_root_ino: Option<i64> = {
        let mut rows = conn.query(
            "SELECT root_ino FROM fs_volumes WHERE id = $1",
            (volume_id,),
        ).await?;
        match rows.next().await? {
            Some(row) => Some(*row.get_value(0)?.as_integer()
                .ok_or_else(|| crate::error::Error::Internal("root_ino: expected integer".into()))?),
            None => None,
        }
    };

    // Snapshot fs_volume_state for re-insertion after replay.
    let saved_volume_state: Option<(bool, i64)> = {
        let mut rows = conn.query(
            "SELECT rollback_enabled::int::bigint, current_snap_id
             FROM fs_volume_state WHERE volume_id = $1",
            (volume_id,),
        ).await?;
        match rows.next().await? {
            Some(row) => {
                let enabled = match row.get_value(0)? {
                    DbValue::Integer(0) => false,
                    DbValue::Integer(_) => true,
                    _ => return Err(Error::Internal("rollback_enabled: expected integer cast".into())),
                };
                let csid: i64 = *row.get_value(1)?.as_integer()
                    .ok_or_else(|| Error::Internal("current_snap_id: expected integer".into()))?;
                Some((enabled, csid))
            }
            None => None,
        }
    };

    // Snapshot fs_snapshots rows we will keep (snap_id <= target) so we can
    // re-insert them after replay. We will not save rows with snap_id > target
    // since those will be pruned anyway. We do not preserve `committed_at`
    // because the SDK's DbValue cannot bind a timestamptz parameter directly;
    // the re-inserted row uses DEFAULT NOW() which is acceptable since rollback
    // semantics only depend on snap_id and label.
    let saved_snapshots: Vec<(i64, Option<String>)> = {
        let mut rows = conn.query(
            "SELECT snap_id, label FROM fs_snapshots
             WHERE volume_id = $1 AND snap_id <= $2",
            (volume_id, target_snap_id),
        ).await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            let sid: i64 = *r.get_value(0)?.as_integer()
                .ok_or_else(|| Error::Internal("snap_id: expected integer".into()))?;
            let label = match r.get_value(1)? {
                DbValue::Text(t) => Some(t),
                DbValue::Null => None,
                _ => return Err(Error::Internal("label: expected text or null".into())),
            };
            out.push((sid, label));
        }
        out
    };

    // Count how many snapshots will be pruned (snap_id > target). We compute
    // this separately because the inode replay's CASCADE wipes the
    // fs_snapshots table, making a post-replay DELETE-count return 0.
    let pruned_snapshots: i64 = {
        let mut rows = conn.query(
            "SELECT COUNT(*) FROM fs_snapshots WHERE volume_id = $1 AND snap_id > $2",
            (volume_id, target_snap_id),
        ).await?;
        match rows.next().await? {
            Some(row) => *row.get_value(0)?.as_integer()
                .ok_or_else(|| Error::Internal("count: expected integer".into()))?,
            None => 0,
        }
    };

    let mut pruned_undo_rows: i64 = 0;
    pruned_undo_rows += replay_fs_dentry(conn, volume_id, target_snap_id).await?;
    pruned_undo_rows += replay_fs_data(conn, volume_id, target_snap_id).await?;
    pruned_undo_rows += replay_fs_symlink(conn, volume_id, target_snap_id).await?;
    pruned_undo_rows += replay_fs_inode(conn, volume_id, target_snap_id).await?;
    pruned_undo_rows += replay_kv_store(conn, volume_id, target_snap_id).await?;

    // Re-insert the fs_volumes row if it was cascade-deleted during inode replay.
    // OpenGauss rejects `ON DUPLICATE KEY UPDATE` against unique/PK columns,
    // so use the portable `INSERT … SELECT … WHERE NOT EXISTS` idiom.
    if let Some(root_ino) = saved_root_ino {
        conn.execute(
            "INSERT INTO fs_volumes (id, root_ino) \
             SELECT $1, $2 \
             WHERE NOT EXISTS (SELECT 1 FROM fs_volumes WHERE id = $1)",
            (volume_id, root_ino),
        ).await?;
    }

    // Re-insert fs_volume_state if it was cascade-deleted with fs_volumes.
    // We restore current_snap_id to target_snap_id+1 below, so this re-insert
    // primarily preserves rollback_enabled. The SDK's DbValue lacks a Bool
    // variant; inline TRUE/FALSE as SQL literals to avoid PG cast errors.
    if let Some((enabled, _csid)) = saved_volume_state {
        let bool_lit = if enabled { "TRUE" } else { "FALSE" };
        let sql = format!(
            "INSERT INTO fs_volume_state (volume_id, rollback_enabled, current_snap_id) \
             SELECT $1, {bool_lit}, $2 \
             WHERE NOT EXISTS (SELECT 1 FROM fs_volume_state WHERE volume_id = $1)"
        );
        conn.execute(&sql, (volume_id, target_snap_id + 1)).await?;
    }

    // Re-insert fs_snapshots rows that we kept (snap_id <= target).
    for (sid, label) in &saved_snapshots {
        let label_db: DbValue = match label {
            Some(s) => DbValue::Text(s.clone()),
            None => DbValue::Null,
        };
        conn.execute(
            "INSERT INTO fs_snapshots (volume_id, snap_id, label) \
             SELECT $1, $2, $3 \
             WHERE NOT EXISTS (SELECT 1 FROM fs_snapshots WHERE volume_id = $1 AND snap_id = $2)",
            vec![
                DbValue::Text(volume_id.to_string()),
                DbValue::Integer(*sid),
                label_db,
            ],
        ).await?;
    }

    for tbl in ["fs_inode_undo", "fs_dentry_undo", "fs_data_undo", "fs_symlink_undo", "kv_store_undo"] {
        let q = format!("DELETE FROM {tbl} WHERE volume_id = $1 AND snap_id > $2");
        conn.execute(&q, (volume_id, target_snap_id)).await?;
    }
    // fs_snapshots rows with snap_id > target were already removed by CASCADE
    // when fs_volumes briefly disappeared during inode replay; we re-inserted
    // only the surviving snap_id <= target rows above. Nothing more to delete.
    conn.execute(
        "UPDATE fs_volume_state SET current_snap_id = $2 WHERE volume_id = $1",
        (volume_id, target_snap_id + 1),
    ).await?;

    Ok(RestoreOutcome { pruned_snapshots, pruned_undo_rows })
}

async fn replay_fs_inode(conn: &DbConn, volume_id: &str, target: i64) -> Result<i64> {
    let mut rows = conn.query(
        "SELECT op, ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev,
                atime_nsec, mtime_nsec, ctime_nsec
         FROM fs_inode_undo
         WHERE volume_id = $1 AND snap_id > $2
         ORDER BY undo_id DESC",
        (volume_id, target),
    ).await?;
    let mut count: i64 = 0;
    while let Some(r) = rows.next().await? {
        let op = read_op(&r, 0)?;
        let ino: i64 = read_i64(&r, 1)?;
        conn.execute("DELETE FROM fs_inode WHERE ino = $1", (ino,)).await?;
        if op != UndoOp::Insert {
            // 13 params — exceeds the 12-element tuple limit; use Vec<DbValue>.
            let args: Vec<DbValue> = vec![
                DbValue::Integer(ino),
                DbValue::Integer(read_i64(&r, 2)?),
                DbValue::Integer(read_i64(&r, 3)?),
                DbValue::Integer(read_i64(&r, 4)?),
                DbValue::Integer(read_i64(&r, 5)?),
                DbValue::Integer(read_i64(&r, 6)?),
                DbValue::Integer(read_i64(&r, 7)?),
                DbValue::Integer(read_i64(&r, 8)?),
                DbValue::Integer(read_i64(&r, 9)?),
                DbValue::Integer(read_i64(&r, 10)?),
                DbValue::Integer(read_i64(&r, 11)?),
                DbValue::Integer(read_i64(&r, 12)?),
                DbValue::Integer(read_i64(&r, 13)?),
            ];
            conn.execute(
                "INSERT INTO fs_inode (ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev,
                                       atime_nsec, mtime_nsec, ctime_nsec)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                args,
            ).await?;
        }
        count += 1;
    }
    Ok(count)
}

async fn replay_fs_dentry(conn: &DbConn, volume_id: &str, target: i64) -> Result<i64> {
    let mut rows = conn.query(
        "SELECT op, id, name, parent_ino, ino FROM fs_dentry_undo
         WHERE volume_id = $1 AND snap_id > $2 ORDER BY undo_id DESC",
        (volume_id, target),
    ).await?;
    let mut count = 0;
    while let Some(r) = rows.next().await? {
        let op = read_op(&r, 0)?;
        let id: i64 = read_i64(&r, 1)?;
        conn.execute("DELETE FROM fs_dentry WHERE id = $1", (id,)).await?;
        if op != UndoOp::Insert {
            let name = read_text(&r, 2)?;
            let parent_ino = read_i64(&r, 3)?;
            let ino = read_i64(&r, 4)?;
            conn.execute(
                "INSERT INTO fs_dentry (id, name, parent_ino, ino) VALUES ($1, $2, $3, $4)",
                (id, name, parent_ino, ino),
            ).await?;
        }
        count += 1;
    }
    Ok(count)
}

async fn replay_fs_data(conn: &DbConn, volume_id: &str, target: i64) -> Result<i64> {
    let mut rows = conn.query(
        "SELECT op, ino, chunk_index, data FROM fs_data_undo
         WHERE volume_id = $1 AND snap_id > $2 ORDER BY undo_id DESC",
        (volume_id, target),
    ).await?;
    let mut count = 0;
    while let Some(r) = rows.next().await? {
        let op = read_op(&r, 0)?;
        let ino: i64 = read_i64(&r, 1)?;
        let chunk: i64 = read_i64(&r, 2)?;
        conn.execute("DELETE FROM fs_data WHERE ino = $1 AND chunk_index = $2", (ino, chunk)).await?;
        if op != UndoOp::Insert {
            let data = match r.get_value(3)? {
                DbValue::Blob(b) => b,
                DbValue::Null => return Err(Error::Internal("fs_data_undo.data: NULL on non-INSERT".into())),
                _ => return Err(Error::Internal("fs_data_undo.data: expected BYTEA".into())),
            };
            // Build args manually because BYTEA needs DbValue::Blob.
            let args: Vec<DbValue> = vec![
                DbValue::Integer(ino),
                DbValue::Integer(chunk),
                DbValue::Blob(data),
            ];
            conn.execute(
                "INSERT INTO fs_data (ino, chunk_index, data) VALUES ($1, $2, $3)",
                args,
            ).await?;
        }
        count += 1;
    }
    Ok(count)
}

async fn replay_fs_symlink(conn: &DbConn, volume_id: &str, target: i64) -> Result<i64> {
    let mut rows = conn.query(
        "SELECT op, ino, target FROM fs_symlink_undo
         WHERE volume_id = $1 AND snap_id > $2 ORDER BY undo_id DESC",
        (volume_id, target),
    ).await?;
    let mut count = 0;
    while let Some(r) = rows.next().await? {
        let op = read_op(&r, 0)?;
        let ino: i64 = read_i64(&r, 1)?;
        conn.execute("DELETE FROM fs_symlink WHERE ino = $1", (ino,)).await?;
        if op != UndoOp::Insert {
            let target_path = read_text(&r, 2)?;
            conn.execute(
                "INSERT INTO fs_symlink (ino, target) VALUES ($1, $2)",
                (ino, target_path),
            ).await?;
        }
        count += 1;
    }
    Ok(count)
}

async fn replay_kv_store(conn: &DbConn, volume_id: &str, target: i64) -> Result<i64> {
    let mut rows = conn.query(
        "SELECT op, key, value, created_at, updated_at FROM kv_store_undo
         WHERE volume_id = $1 AND snap_id > $2 ORDER BY undo_id DESC",
        (volume_id, target),
    ).await?;
    let mut count = 0;
    while let Some(r) = rows.next().await? {
        let op = read_op(&r, 0)?;
        let key = read_text(&r, 1)?;
        conn.execute("DELETE FROM kv_store WHERE key = $1", (key.clone(),)).await?;
        if op != UndoOp::Insert {
            let value = read_text(&r, 2)?;
            let created_at = read_i64(&r, 3)?;
            let updated_at = read_i64(&r, 4)?;
            conn.execute(
                "INSERT INTO kv_store (key, value, created_at, updated_at) VALUES ($1,$2,$3,$4)",
                (key, value, created_at, updated_at),
            ).await?;
        }
        count += 1;
    }
    Ok(count)
}

// --- helpers ---

fn read_op(row: &crate::db::DbRow, idx: usize) -> Result<UndoOp> {
    let s = read_text(row, idx)?;
    let c = s.chars().next()
        .ok_or_else(|| Error::Internal("op: empty".into()))?;
    UndoOp::from_char(c).ok_or_else(|| Error::Internal(format!("op: invalid '{c}'")))
}

fn read_i64(row: &crate::db::DbRow, idx: usize) -> Result<i64> {
    Ok(*row.get_value(idx)?
        .as_integer()
        .ok_or_else(|| Error::Internal(format!("col {idx}: expected integer")))?)
}

fn read_text(row: &crate::db::DbRow, idx: usize) -> Result<String> {
    match row.get_value(idx)? {
        DbValue::Text(t) => Ok(t),
        DbValue::Null => Err(Error::Internal(format!("col {idx}: unexpected NULL"))),
        _ => Err(Error::Internal(format!("col {idx}: expected text"))),
    }
}
