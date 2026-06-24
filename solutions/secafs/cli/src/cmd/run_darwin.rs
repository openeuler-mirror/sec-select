//! Darwin (macOS) run command implementation.
//!
//! This module provides a sandboxed execution environment using NFS for
//! filesystem mounting. The current working directory becomes a
//! copy-on-write overlay backed by SecAFS (PostgreSQL), mounted via a
//! localhost NFS server.
//!
//! Sandboxing is enforced using macOS sandbox-exec with dynamically generated
//! profiles that restrict file writes to the NFS mountpoint and allowed paths.

#![cfg(unix)]

use secafs_sdk::{SecAFS, SecAFSOptions, FileSystem, HostFS, OverlayFS};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::nfs::AgentNFS;
use crate::nfsserve::tcp::NFSTcp;

#[cfg(target_os = "macos")]
use crate::sandbox::darwin::{generate_sandbox_profile, SandboxConfig};

/// Default NFS port to try (use a high port to avoid needing root)
const DEFAULT_NFS_PORT: u32 = 11111;

/// Default PostgreSQL URL prefix for auto-created session databases
const DEFAULT_PG_PREFIX: &str = "postgres://localhost";

/// Run the command in a Darwin sandbox.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    allow: Vec<PathBuf>,
    no_default_allows: bool,
    _experimental_sandbox: bool,
    _strace: bool,
    session_id: Option<String>,
    _system: bool,
    postgres_url: Option<String>,
    command: PathBuf,
    args: Vec<String>,
) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let home = dirs::home_dir().context("Failed to get home directory")?;

    let session = setup_run_directory(session_id, allow, no_default_allows, &cwd, &home)?;

    // Check if we're joining an existing session
    if is_mountpoint(&session.mountpoint) {
        if is_mount_healthy(&session.mountpoint) {
            eprintln!("Joining existing session: {}", session.session_id);
            eprintln!();
            let exit_code = run_command_in_mount(&session, command, args)?;
            std::process::exit(exit_code);
        } else {
            eprintln!("Cleaning up stale NFS mount...");
            if let Err(e) = unmount(&session.mountpoint) {
                eprintln!("Warning: Failed to unmount stale mount: {}", e);
            }
        }
    }

    // Resolve PostgreSQL URL for this session's delta layer
    let pg_url = resolve_session_pg_url(&postgres_url, &session.session_id)?;

    let options = SecAFSOptions::with_postgres_url(&pg_url);
    let secafs_inst = SecAFS::open(options)
        .await
        .context("Failed to create SecAFS")?;

    // Create overlay filesystem with CWD as base
    let base_str = cwd.to_string_lossy().to_string();
    let hostfs = HostFS::new(&base_str).context("Failed to create HostFS")?;
    let overlay = OverlayFS::new(Arc::new(hostfs), secafs_inst.fs);

    overlay
        .init(&base_str)
        .await
        .context("Failed to initialize overlay")?;

    let fs: Arc<Mutex<dyn FileSystem>> = Arc::new(Mutex::new(overlay));

    let nfs = AgentNFS::new(fs);
    let port = find_available_port(DEFAULT_NFS_PORT)?;

    let bind_addr = format!("127.0.0.1:{}", port);
    let listener = crate::nfsserve::tcp::NFSTcpListener::bind(&bind_addr, nfs)
        .await
        .context("Failed to bind NFS server")?;

    let server_handle = tokio::spawn(async move {
        if let Err(e) = listener.handle_forever().await {
            eprintln!("NFS server error: {}", e);
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    mount_nfs(port, &session.mountpoint)?;

    print_welcome_banner(&session);

    let exit_code = run_command_in_mount(&session, command, args)?;

    unmount(&session.mountpoint)?;
    server_handle.abort();

    if let Err(e) = std::fs::remove_dir(&session.mountpoint) {
        eprintln!(
            "Warning: Failed to clean up mountpoint {}: {}",
            session.mountpoint.display(),
            e
        );
    }

    eprintln!();
    eprintln!("Session: {}", session.session_id);
    eprintln!("Database: {}", pg_url);
    eprintln!();
    eprintln!("To resume this session:");
    eprintln!("  secafs run --session {}", session.session_id);
    eprintln!();
    eprintln!("To see what changed:");
    eprintln!("  secafs diff {}", pg_url);

    std::process::exit(exit_code);
}

/// Resolve the PostgreSQL URL for a session's delta layer.
fn resolve_session_pg_url(user_url: &Option<String>, session_id: &str) -> Result<String> {
    match user_url {
        Some(url) => Ok(url.clone()),
        None => {
            let db_name = format!("secafs_{}", session_id.replace('-', "_"));
            Ok(format!("{}/{}", DEFAULT_PG_PREFIX, db_name))
        }
    }
}

/// Print the welcome banner (macOS).
#[cfg(target_os = "macos")]
fn print_welcome_banner(session: &RunSession) {
    use crate::sandbox::group_paths_by_parent;

    eprintln!("Welcome to SecAFS!");
    eprintln!();
    eprintln!("The following directories are writable:");
    eprintln!();
    eprintln!("  - {} (copy-on-write)", session.cwd.display());
    eprintln!("  - /tmp");
    for grouped_path in group_paths_by_parent(&session.allow_paths) {
        eprintln!("  - {}", grouped_path);
    }
    eprintln!();
    eprintln!("To join this session from another terminal:");
    eprintln!();
    eprintln!("  secafs run --session {} <command>", session.session_id);
    eprintln!();
}

/// Print the welcome banner (Linux).
#[cfg(target_os = "linux")]
fn print_welcome_banner(session: &RunSession) {
    eprintln!("Welcome to SecAFS!");
    eprintln!();
    eprintln!("  {} (copy-on-write)", session.cwd.display());
    eprintln!();
}

struct RunSession {
    run_dir: PathBuf,
    mountpoint: PathBuf,
    session_id: String,
    allow_paths: Vec<PathBuf>,
    cwd: PathBuf,
}

const DEFAULT_ALLOWED_DIRS: &[&str] = &[
    ".amp",
    ".claude",
    ".claude.json",
    ".gemini",
    ".local",
    ".npm",
    ".config",
    ".cache",
    ".bun",
];

fn setup_run_directory(
    session_id: Option<String>,
    user_allow_paths: Vec<PathBuf>,
    no_default_allows: bool,
    cwd: &Path,
    home: &Path,
) -> Result<RunSession> {
    let run_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let run_dir = home.join(".secafs").join("run").join(&run_id);
    std::fs::create_dir_all(&run_dir).context("Failed to create run directory")?;

    let mountpoint = run_dir.join("mnt");
    std::fs::create_dir_all(&mountpoint).context("Failed to create mountpoint")?;

    let mut allow_paths = user_allow_paths;
    if !no_default_allows {
        for dir in DEFAULT_ALLOWED_DIRS {
            let path = home.join(dir);
            if path.exists() {
                allow_paths.push(path);
            }
        }
    }

    let zsh_dir = run_dir.join("zsh");
    std::fs::create_dir_all(&zsh_dir).context("Failed to create zsh config directory")?;
    std::fs::write(zsh_dir.join(".zshrc"), "PROMPT='%~%# '\n")
        .context("Failed to write zsh config")?;

    Ok(RunSession {
        run_dir,
        mountpoint,
        session_id: run_id,
        allow_paths,
        cwd: cwd.to_path_buf(),
    })
}

fn is_mountpoint(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(path_meta) = std::fs::metadata(path) else {
        return false;
    };

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("/"));

    let Ok(parent_meta) = std::fs::metadata(parent) else {
        return false;
    };

    path_meta.dev() != parent_meta.dev()
}

fn is_mount_healthy(mountpoint: &Path) -> bool {
    std::fs::read_dir(mountpoint).is_ok()
}

fn find_available_port(start_port: u32) -> Result<u32> {
    for port in start_port..start_port + 100 {
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!(
        "Could not find an available port in range {}-{}",
        start_port,
        start_port + 100
    );
}

#[cfg(target_os = "macos")]
fn mount_nfs(port: u32, mountpoint: &Path) -> Result<()> {
    let output = Command::new("/sbin/mount_nfs")
        .args([
            "-o",
            &format!(
                "locallocks,vers=3,tcp,port={},mountport={},soft,timeo=100,retrans=5",
                port, port
            ),
            "127.0.0.1:/",
            mountpoint.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute mount_nfs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to mount NFS: {}", stderr.trim());
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn mount_nfs(port: u32, mountpoint: &Path) -> Result<()> {
    let output = Command::new("mount")
        .args([
            "-t",
            "nfs",
            "-o",
            &format!(
                "vers=3,tcp,port={},mountport={},nolock,soft,timeo=100,retrans=5",
                port, port
            ),
            "127.0.0.1:/",
            mountpoint.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute mount. Make sure nfs-common is installed.")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to mount NFS: {}. Make sure nfs-common is installed (apt-get install nfs-common).",
            stderr.trim()
        );
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_command_in_mount(session: &RunSession, command: PathBuf, args: Vec<String>) -> Result<i32> {
    let config = SandboxConfig {
        mountpoint: session.mountpoint.clone(),
        allow_paths: session.allow_paths.clone(),
        allow_read_paths: Vec::new(),
        allow_network: true,
        session_id: session.session_id.clone(),
    };
    let profile = generate_sandbox_profile(&config);

    let mut cmd = Command::new("sandbox-exec");
    cmd.arg("-p")
        .arg(&profile)
        .arg(&command)
        .args(&args)
        .current_dir(&session.mountpoint)
        .env("SECAFS", "1")
        .env("SECAFS_SANDBOX", "macos-sandbox")
        .env("PS1", "\\w\\$ ")
        .env("ZDOTDIR", session.run_dir.join("zsh"));

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute command: {}", command.display()))?;

    Ok(status.code().unwrap_or(1))
}

#[cfg(target_os = "linux")]
fn run_command_in_mount(session: &RunSession, command: PathBuf, args: Vec<String>) -> Result<i32> {
    let mut cmd = Command::new(&command);
    cmd.args(&args)
        .current_dir(&session.mountpoint)
        .env("SECAFS", "1")
        .env("PS1", "\\u@\\h:\\w\\$ ")
        .env("ZDOTDIR", session.run_dir.join("zsh"));

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute command: {}", command.display()))?;

    Ok(status.code().unwrap_or(1))
}

#[cfg(target_os = "macos")]
fn unmount(mountpoint: &Path) -> Result<()> {
    let output = Command::new("/sbin/umount")
        .arg(mountpoint)
        .output()
        .context("Failed to execute umount")?;

    if !output.status.success() {
        let output2 = Command::new("/sbin/umount")
            .arg("-f")
            .arg(mountpoint)
            .output()?;

        if !output2.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Failed to unmount: {}. You may need to manually unmount with: umount -f {}",
                stderr.trim(),
                mountpoint.display()
            );
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn unmount(mountpoint: &Path) -> Result<()> {
    let output = Command::new("umount")
        .arg(mountpoint)
        .output()
        .context("Failed to execute umount")?;

    if !output.status.success() {
        let output2 = Command::new("umount").arg("-l").arg(mountpoint).output()?;

        if !output2.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Failed to unmount: {}. You may need to manually unmount with: umount -l {}",
                stderr.trim(),
                mountpoint.display()
            );
        }
    }

    Ok(())
}
