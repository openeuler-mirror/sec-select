//! Commit a snapshot point and list committed snapshots.

use crate::db::{DbConn, DbValue};
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use super::types::SnapshotInfo;

/// Commit a snapshot at the volume's current_snap_id. Bumps current_snap_id by 1.
/// Idempotent on duplicate `(volume_id, label)` — returns the existing snapshot.
pub async fn commit(conn: &DbConn, volume_id: &str, label: Option<&str>) -> Result<SnapshotInfo> {
    conn.execute("BEGIN", ()).await?;
    let result = commit_inner(conn, volume_id, label).await;
    match &result {
        Ok(_) => { conn.execute("COMMIT", ()).await?; }
        Err(_) => { let _ = conn.execute("ROLLBACK", ()).await; }
    }
    result
}

async fn commit_inner(conn: &DbConn, volume_id: &str, label: Option<&str>) -> Result<SnapshotInfo> {
    // Cast BOOL to int because the SDK's DbValue mapping returns NULL for PG BOOL.
    let mut rows = conn.query(
        "SELECT current_snap_id, rollback_enabled::int::bigint
         FROM fs_volume_state WHERE volume_id = $1",
        (volume_id,),
    ).await?;
    let row = rows.next().await?
        .ok_or_else(|| Error::Internal(format!("rollback not enabled for volume {volume_id}")))?;
    let current_snap_id: i64 = *row.get_value(0)?
        .as_integer()
        .ok_or_else(|| Error::Internal("current_snap_id: expected integer".into()))?;
    let enabled = match row.get_value(1)? {
        DbValue::Integer(0) => false,
        DbValue::Integer(_) => true,
        _ => return Err(Error::Internal("rollback_enabled: expected integer cast".into())),
    };
    if !enabled {
        return Err(Error::Internal(format!("rollback disabled for volume {volume_id}")));
    }

    // Idempotency on (volume_id, label).
    if let Some(label_val) = label {
        let mut existing = conn.query(
            "SELECT snap_id, committed_at::text FROM fs_snapshots
             WHERE volume_id = $1 AND label = $2",
            (volume_id, label_val),
        ).await?;
        if let Some(r) = existing.next().await? {
            let snap_id: i64 = *r.get_value(0)?.as_integer()
                .ok_or_else(|| Error::Internal("snap_id".into()))?;
            let ts_txt = match r.get_value(1)? {
                DbValue::Text(t) => t,
                _ => return Err(Error::Internal("committed_at: expected text".into())),
            };
            let committed_at = parse_ts(&ts_txt)?;
            return Ok(SnapshotInfo { snap_id, committed_at, label: Some(label_val.to_string()) });
        }
    }

    // Insert new snapshot row.
    // Option<&str> is not Into<DbValue>, so convert explicitly to DbValue.
    let label_db: DbValue = match label {
        Some(s) => DbValue::Text(s.to_string()),
        None => DbValue::Null,
    };
    conn.execute(
        "INSERT INTO fs_snapshots (volume_id, snap_id, label) VALUES ($1, $2, $3)",
        vec![
            DbValue::Text(volume_id.to_string()),
            DbValue::Integer(current_snap_id),
            label_db,
        ],
    ).await?;
    conn.execute(
        "UPDATE fs_volume_state SET current_snap_id = current_snap_id + 1 WHERE volume_id = $1",
        (volume_id,),
    ).await?;

    let mut rows = conn.query(
        "SELECT committed_at::text FROM fs_snapshots WHERE volume_id = $1 AND snap_id = $2",
        (volume_id, current_snap_id),
    ).await?;
    let row = rows.next().await?.ok_or_else(|| Error::Internal("just-inserted snapshot not found".into()))?;
    let ts_txt = match row.get_value(0)? {
        DbValue::Text(t) => t,
        _ => return Err(Error::Internal("committed_at: expected text".into())),
    };
    Ok(SnapshotInfo {
        snap_id: current_snap_id,
        committed_at: parse_ts(&ts_txt)?,
        label: label.map(str::to_string),
    })
}

/// List all committed snapshots for a volume, ordered by snap_id ascending.
pub async fn list(conn: &DbConn, volume_id: &str) -> Result<Vec<SnapshotInfo>> {
    let mut rows = conn.query(
        "SELECT snap_id, committed_at::text, label FROM fs_snapshots
         WHERE volume_id = $1 ORDER BY snap_id ASC",
        (volume_id,),
    ).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        let snap_id: i64 = *r.get_value(0)?.as_integer()
            .ok_or_else(|| Error::Internal("snap_id".into()))?;
        let ts_txt = match r.get_value(1)? {
            DbValue::Text(t) => t,
            _ => return Err(Error::Internal("committed_at: expected text".into())),
        };
        let label = match r.get_value(2)? {
            DbValue::Text(t) => Some(t),
            DbValue::Null => None,
            _ => return Err(Error::Internal("label: expected text or null".into())),
        };
        out.push(SnapshotInfo {
            snap_id,
            committed_at: parse_ts(&ts_txt)?,
            label,
        });
    }
    Ok(out)
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    // PG TIMESTAMPTZ::text format is "YYYY-MM-DD HH:MM:SS.fff+00" (note: space, not T, and short tz).
    // Try RFC3339 first (some PG versions do produce that), then fall back to PG's default.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    let normalized = s.replace(' ', "T");
    let normalized = if normalized.ends_with("+00") {
        format!("{}:00", normalized)
    } else {
        normalized
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("parse timestamptz {s:?}: {e}")))
}
