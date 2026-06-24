//! Execute a command with a SecAFS filesystem mounted.
//!
//! This module provides the `secafs exec` command which mounts a SecAFS
//! filesystem to a temporary directory, runs a command with that as the
//! working directory, and automatically unmounts when done.

use secafs_sdk::{SecAFSOptions, FileSystem, HostFS, OverlayFS};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use secafs_sdk::db::DbValue as Value;

use crate::cmd::init::open_secafs;
use crate::mount::{mount_fs, MountBackend, MountOpts};

/// Handle the exec command.
pub async fn handle_exec_command(
    postgres_url: String,
    command: PathBuf,
    args: Vec<String>,
    backend: MountBackend,
) -> Result<()> {
    let opts = SecAFSOptions::resolve(&postgres_url)?;

    let secafs_inst = open_secafs(opts).await?;

    // Check for overlay configuration
    let fs: Arc<Mutex<dyn FileSystem + Send>> = {
        let conn = secafs_inst.get_connection().await?;

        let query = "SELECT value FROM fs_overlay_config WHERE key = 'base_path'";
        let base_path: Option<String> = match conn.query(query, ()).await {
            Ok(mut rows) => {
                if let Ok(Some(row)) = rows.next().await {
                    row.get_value(0).ok().and_then(|v| {
                        if let Value::Text(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        if let Some(base_path) = base_path {
            eprintln!("Using overlay filesystem with base: {}", base_path);
            let hostfs = HostFS::new(&base_path)?;
            let overlay = OverlayFS::new(Arc::new(hostfs), secafs_inst.fs);
            overlay.load().await?;
            Arc::new(Mutex::new(overlay)) as Arc<Mutex<dyn FileSystem + Send>>
        } else {
            Arc::new(Mutex::new(secafs_inst.fs)) as Arc<Mutex<dyn FileSystem + Send>>
        }
    };

    let exec_id = uuid::Uuid::new_v4().to_string();
    let mountpoint = std::env::temp_dir().join(format!("secafs-exec-{}", exec_id));
    std::fs::create_dir_all(&mountpoint).context("Failed to create mount directory")?;

    let fsname = format!("secafs:{}", postgres_url);

    let mount_opts = MountOpts {
        mountpoint: mountpoint.clone(),
        backend,
        fsname,
        uid: None,
        gid: None,
        allow_other: false,
        allow_root: false,
        auto_unmount: false,
        lazy_unmount: true,
        timeout: std::time::Duration::from_secs(10),
    };

    let _mount_handle = mount_fs(fs, mount_opts).await?;

    let status = Command::new(&command)
        .args(&args)
        .current_dir(&mountpoint)
        .status()
        .with_context(|| format!("Failed to execute: {}", command.display()))?;

    drop(_mount_handle);

    let _ = std::fs::remove_dir_all(&mountpoint);

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
