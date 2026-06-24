//! Standalone NFS server command.
//!
//! This module provides a standalone NFS server that exports a SecAFS
//! filesystem over the network, allowing remote systems (like VMs) to mount
//! it as their root filesystem.

use secafs_sdk::{SecAFSOptions, FileSystem, HostFS, OverlayFS};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::Mutex;

use crate::cmd::init::open_secafs;
use crate::nfs::AgentNFS;

/// Handle the `nfs` command - start a standalone NFS server.
pub async fn handle_nfs_command(postgres_url: String, bind: String, port: u32) -> Result<()> {
    let options = SecAFSOptions::resolve(&postgres_url)?;
    let secafs_inst = open_secafs(options).await?;

    let base_path = secafs_inst
        .is_overlay_enabled()
        .await
        .context("Failed to check overlay config")?;

    let fs: Arc<Mutex<dyn FileSystem>> = if let Some(base_str) = base_path {
        let hostfs = HostFS::new(&base_str).context("Failed to create HostFS")?;
        let overlay = OverlayFS::new(Arc::new(hostfs), secafs_inst.fs);
        overlay.load().await?;

        eprintln!("Mode: overlay (base: {})", base_str);
        Arc::new(Mutex::new(overlay))
    } else {
        eprintln!("Mode: direct SecAFS");
        Arc::new(Mutex::new(secafs_inst.fs))
    };

    let nfs = AgentNFS::new(fs);

    let bind_addr_str = format!("{}:{}", bind, port);
    let listener = crate::nfsserve::tcp::NFSTcpListener::bind(&bind_addr_str, nfs)
        .await
        .with_context(|| format!("Failed to bind NFS server to {}", bind_addr_str))?;

    eprintln!();
    eprintln!("SecAFS NFS Server");
    eprintln!("  Database: {}", postgres_url);
    eprintln!("  Listening: {}", bind_addr_str);
    eprintln!("  Export: /");
    eprintln!();
    eprintln!("Mount from client:");
    eprintln!(
        "  mount -t nfs -o vers=3,tcp,port={},mountport={},nolock {}:/ /mnt",
        port, port, bind
    );
    eprintln!();
    eprintln!("Press Ctrl+C to stop.");
    eprintln!();

    use crate::nfsserve::tcp::NFSTcp;
    let server_handle = tokio::spawn(async move {
        if let Err(e) = listener.handle_forever().await {
            eprintln!("NFS server error: {}", e);
        }
    });

    signal::ctrl_c()
        .await
        .context("Failed to listen for ctrl+c")?;

    eprintln!();
    eprintln!("Shutting down...");

    server_handle.abort();

    Ok(())
}
