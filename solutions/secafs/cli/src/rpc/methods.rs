use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex;

#[async_trait]
pub trait MountBackend: Send + Sync {
    async fn mount(&self, id: &str, host_path: &Path) -> anyhow::Result<()>;
    async fn unmount(&self, id: &str) -> anyhow::Result<()>;
    async fn destroy(&self, id: &str) -> anyhow::Result<()>;
    async fn get_connection(&self) -> anyhow::Result<secafs_sdk::db::DbConn>;
}

pub struct MountEntry {
    pub host_path: PathBuf,
    pub since: String,
}

pub struct State {
    pub backend: Arc<dyn MountBackend>,
    pub mount_root: PathBuf,
    pub mounts: Mutex<HashMap<String, MountEntry>>,
}

impl State {
    #[cfg(test)]
    pub fn with_fake_mount() -> Arc<Self> {
        struct Fake;
        #[async_trait]
        impl MountBackend for Fake {
            async fn mount(&self, _: &str, _: &Path) -> anyhow::Result<()> {
                Ok(())
            }
            async fn unmount(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn destroy(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn get_connection(&self) -> anyhow::Result<secafs_sdk::db::DbConn> {
                anyhow::bail!("get_connection: not supported in fake backend")
            }
        }
        Arc::new(Self {
            backend: Arc::new(Fake),
            mount_root: std::env::temp_dir().join("secafs-test-mounts"),
            mounts: Mutex::new(HashMap::new()),
        })
    }
}

pub async fn dispatch(state: &State, method: &str, params: Value) -> Result<Value, (i32, String)> {
    match method {
        "secafs.v1.mount" => mount(state, params).await,
        "secafs.v1.unmount" => unmount(state, params).await,
        "secafs.v1.list" => list(state).await,
        "secafs.v1.destroy" => destroy(state, params).await,
        "secafs.v1.ping" => ping(state).await,
        "secafs.v1.snapshot.enable" => snapshot_enable(state, params).await,
        "secafs.v1.snapshot.disable" => snapshot_disable(state, params).await,
        "secafs.v1.snapshot.commit" => snapshot_commit(state, params).await,
        "secafs.v1.snapshot.list" => snapshot_list(state, params).await,
        "secafs.v1.snapshot.restore" => snapshot_restore(state, params).await,
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

/// A previous daemon instance that died without unmounting leaves a
/// disconnected FUSE mountpoint behind ("Transport endpoint is not
/// connected"): stat fails with ENOTCONN, create_dir_all then reports
/// EEXIST, and the remount is stuck until someone cleans up. Detect the
/// carcass and lazy-detach it so mount-over-respawn just works.
#[cfg(target_os = "linux")]
fn detach_stale_fuse_mountpoint(host_path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    match std::fs::metadata(host_path) {
        Err(e) if e.raw_os_error() == Some(libc::ENOTCONN) => {
            let Ok(cpath) = std::ffi::CString::new(host_path.as_os_str().as_bytes()) else {
                return;
            };
            let rc = unsafe { libc::umount2(cpath.as_ptr(), libc::MNT_DETACH) };
            if rc == 0 {
                tracing::info!("detached stale FUSE mountpoint at {}", host_path.display());
            } else {
                tracing::warn!(
                    "failed to detach stale FUSE mountpoint at {}: {}",
                    host_path.display(),
                    std::io::Error::last_os_error()
                );
            }
        }
        _ => {}
    }
}

#[cfg(not(target_os = "linux"))]
fn detach_stale_fuse_mountpoint(_host_path: &Path) {}

fn require_conversation_id(params: &Value) -> Result<&str, (i32, String)> {
    params
        .get("conversationId")
        .and_then(|v| v.as_str())
        .ok_or((-32602, "conversationId required (string)".into()))
}

async fn mount(state: &State, params: Value) -> Result<Value, (i32, String)> {
    use super::types::FUSE_MOUNT_FAILED;
    let id = require_conversation_id(&params)?;
    let mut mounts = state.mounts.lock().await;
    if let Some(existing) = mounts.get(id) {
        return Ok(json!({"hostPath": existing.host_path, "mounted": true}));
    }
    let host_path = params
        .get("hostPath")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| state.mount_root.join(id));
    detach_stale_fuse_mountpoint(&host_path);
    std::fs::create_dir_all(&host_path)
        .map_err(|e| (FUSE_MOUNT_FAILED, format!("mkdir failed: {e}")))?;
    state
        .backend
        .mount(id, &host_path)
        .await
        .map_err(|e| (FUSE_MOUNT_FAILED, format!("mount failed: {e}")))?;
    mounts.insert(
        id.to_string(),
        MountEntry {
            host_path: host_path.clone(),
            since: Utc::now().to_rfc3339(),
        },
    );
    Ok(json!({"hostPath": host_path, "mounted": true}))
}

async fn unmount(state: &State, params: Value) -> Result<Value, (i32, String)> {
    use super::types::FUSE_MOUNT_FAILED;
    let id = require_conversation_id(&params)?;
    let mut mounts = state.mounts.lock().await;
    if mounts.remove(id).is_none() {
        return Ok(json!({"unmounted": false}));
    }
    state
        .backend
        .unmount(id)
        .await
        .map_err(|e| (FUSE_MOUNT_FAILED, format!("unmount failed: {e}")))?;
    Ok(json!({"unmounted": true}))
}

async fn list(state: &State) -> Result<Value, (i32, String)> {
    let mounts = state.mounts.lock().await;
    let arr: Vec<Value> = mounts
        .iter()
        .map(|(id, m)| {
            json!({
                "conversationId": id,
                "hostPath": m.host_path,
                "since": m.since,
            })
        })
        .collect();
    Ok(json!({"mounts": arr}))
}

async fn destroy(state: &State, params: Value) -> Result<Value, (i32, String)> {
    use super::types::FUSE_MOUNT_FAILED;
    let id = require_conversation_id(&params)?;
    // Capture host_path before unmount() removes the mount-table entry.
    let host_path = state.mounts.lock().await.get(id).map(|m| m.host_path.clone());
    // Implicit unmount if currently mounted
    let _ = unmount(state, json!({"conversationId": id})).await;
    state
        .backend
        .destroy(id)
        .await
        .map_err(|e| (FUSE_MOUNT_FAILED, format!("destroy failed: {e}")))?;
    // Best-effort cleanup of the host mount-point directory created during mount.
    // Falls back to mount_root/id if the table didn't have an entry (e.g., destroy
    // called on a never-mounted volume after a daemon restart).
    let target = host_path.unwrap_or_else(|| state.mount_root.join(id));
    let _ = std::fs::remove_dir(&target);
    Ok(json!({"destroyed": true}))
}

async fn ping(state: &State) -> Result<Value, (i32, String)> {
    // Live probe, not a startup-time flag: checking out a connection also
    // triggers the backend's lazy pool rebuild after a connection loss, so a
    // periodic status poll doubles as the self-healing tick.
    let pg_connected = match state.backend.get_connection().await {
        Ok(conn) => !conn.is_closed(),
        Err(e) => {
            tracing::warn!("ping: postgres unreachable: {e}");
            false
        }
    };
    let mounts = state.mounts.lock().await;
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pgConnected": pg_connected,
        "mountCount": mounts.len(),
    }))
}

// --- snapshot handlers ---

async fn snapshot_enable(state: &State, params: Value) -> Result<Value, (i32, String)> {
    let id = require_conversation_id(&params)?;
    let conn = get_conn(state).await?;
    let result = secafs_sdk::snapshot::enable(&conn, id).await
        .map_err(|e| (-32603, format!("snapshot.enable: {e}")))?;
    Ok(json!({
        "enabled": result.rollback_enabled,
        "currentSnapId": result.current_snap_id,
    }))
}

async fn snapshot_disable(state: &State, params: Value) -> Result<Value, (i32, String)> {
    let id = require_conversation_id(&params)?;
    let conn = get_conn(state).await?;
    let r = secafs_sdk::snapshot::disable(&conn, id).await
        .map_err(|e| (-32603, format!("snapshot.disable: {e}")))?;
    Ok(json!({
        "disabled": true,
        "purgedSnapshots": r.purged_snapshots,
        "purgedUndoRows": r.purged_undo_rows,
    }))
}

async fn snapshot_commit(state: &State, params: Value) -> Result<Value, (i32, String)> {
    let id = require_conversation_id(&params)?;
    let label = params.get("label").and_then(|v| v.as_str());
    let conn = get_conn(state).await?;
    let info = secafs_sdk::snapshot::commit(&conn, id, label).await
        .map_err(|e| (super::types::ROLLBACK_NOT_ENABLED, format!("snapshot.commit: {e}")))?;
    Ok(json!({
        "snapId": info.snap_id,
        "committedAt": info.committed_at.to_rfc3339(),
        "label": info.label,
    }))
}

async fn snapshot_list(state: &State, params: Value) -> Result<Value, (i32, String)> {
    let id = require_conversation_id(&params)?;
    let conn = get_conn(state).await?;
    let snaps = secafs_sdk::snapshot::list(&conn, id).await
        .map_err(|e| (-32603, format!("snapshot.list: {e}")))?;
    let arr: Vec<Value> = snaps.into_iter().map(|s| json!({
        "snapId": s.snap_id,
        "committedAt": s.committed_at.to_rfc3339(),
        "label": s.label,
    })).collect();
    Ok(json!({"snapshots": arr}))
}

async fn snapshot_restore(state: &State, params: Value) -> Result<Value, (i32, String)> {
    let id = require_conversation_id(&params)?;
    let snap_id = params.get("snapId").and_then(|v| v.as_i64())
        .ok_or((-32602, "snapId required (i64)".into()))?;
    let conn = get_conn(state).await?;
    let outcome = secafs_sdk::snapshot::restore_to(&conn, id, snap_id).await
        .map_err(|e| {
            // Walk std::error::Error::source so the underlying tokio_postgres
            // server message ("ERROR: ...") is included instead of just the
            // top-level "postgres error: db error".
            let mut msg = format!("snapshot.restore: {e}");
            let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
            while let Some(s) = src {
                msg.push_str(&format!(" | caused by: {s}"));
                src = s.source();
            }
            (-32603, msg)
        })?;
    Ok(json!({
        "restored": true,
        "prunedSnapshots": outcome.pruned_snapshots,
        "prunedUndoRows": outcome.pruned_undo_rows,
    }))
}

/// Obtain a DB connection from the backend's shared pool.
async fn get_conn(state: &State) -> Result<secafs_sdk::db::DbConn, (i32, String)> {
    state.backend.get_connection().await
        .map_err(|e| (-32603, format!("get_connection: {e}")))
}
