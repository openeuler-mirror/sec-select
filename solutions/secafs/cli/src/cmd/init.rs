use std::path::PathBuf;

use secafs_sdk::{SecAFS, SecAFSOptions, OverlayFS};
use anyhow::{Context, Result as AnyhowResult};

use crate::opts::MountBackend;

pub async fn open_secafs(options: SecAFSOptions) -> Result<SecAFS, secafs_sdk::error::Error> {
    // Use a single connection in CLI to avoid potential hang with multi-connection pool in block_on
    let options = options.with_postgres_pool_size(1);
    SecAFS::open(options).await
}

pub async fn init_database(
    postgres_url: String,
    base: Option<PathBuf>,
    command: Option<String>,
    backend: MountBackend,
) -> AnyhowResult<()> {
    let mut open_options = SecAFSOptions::with_postgres_url(&postgres_url).with_postgres_pool_size(1);
    if let Some(base_path) = base.as_ref() {
        open_options = open_options.with_base(base_path);
    }

    let secafs_inst = SecAFS::open(open_options)
        .await
        .context("Failed to initialize database")?;

    if let Some(ref base_path) = base {
        let base_path_str = base_path
            .canonicalize()
            .context("Failed to canonicalize base path")?
            .to_string_lossy()
            .to_string();

        let conn = secafs_inst.get_connection().await?;
        OverlayFS::init_schema(&conn, &base_path_str)
            .await
            .context("Failed to initialize overlay schema")?;

        eprintln!("Created overlay filesystem in PostgreSQL");
        eprintln!("Database: {}", postgres_url);
        eprintln!("Base: {}", base_path.display());
    } else {
        eprintln!("Created SecAFS filesystem in PostgreSQL");
        eprintln!("Database: {}", postgres_url);
    }

    if let Some(cmd_str) = command {
        run_init_cmd(&postgres_url, cmd_str, backend, base, secafs_inst).await?;
    }

    Ok(())
}

#[cfg(unix)]
async fn run_init_cmd(
    postgres_url: &str,
    cmd_str: String,
    backend: MountBackend,
    base: Option<PathBuf>,
    secafs_inst: SecAFS,
) -> AnyhowResult<()> {
    use crate::mount::{mount_fs, MountOpts};
    use secafs_sdk::{FileSystem, HostFS};
    use std::process::Command;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let fs: Arc<Mutex<dyn FileSystem + Send>> = if let Some(ref base_path) = base {
        let canonical = base_path
            .canonicalize()
            .context("Failed to canonicalize base path")?;
        let hostfs = HostFS::new(&canonical)?;
        let overlay = OverlayFS::new(Arc::new(hostfs), secafs_inst.fs);
        Arc::new(Mutex::new(overlay)) as Arc<Mutex<dyn FileSystem + Send>>
    } else {
        Arc::new(Mutex::new(secafs_inst.fs)) as Arc<Mutex<dyn FileSystem + Send>>
    };

    let exec_id = uuid::Uuid::new_v4().to_string();
    let mountpoint = std::env::temp_dir().join(format!("secafs-init-{}", exec_id));
    std::fs::create_dir_all(&mountpoint).context("Failed to create mount directory")?;

    let mount_opts = MountOpts {
        mountpoint: mountpoint.clone(),
        backend,
        fsname: format!("secafs:{}", postgres_url),
        uid: None,
        gid: None,
        allow_other: false,
        allow_root: false,
        auto_unmount: false,
        lazy_unmount: true,
        timeout: std::time::Duration::from_secs(10),
    };

    let mount_handle = mount_fs(fs, mount_opts).await?;

    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(&mountpoint)
        .status()
        .with_context(|| format!("Failed to execute: {}", cmd_str))?;

    drop(mount_handle);

    let _ = std::fs::remove_dir_all(&mountpoint);

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

#[cfg(not(unix))]
async fn run_init_cmd(
    _postgres_url: &str,
    _cmd_str: String,
    _backend: MountBackend,
    _base: Option<PathBuf>,
    _agent: SecAFS,
) -> AnyhowResult<()> {
    anyhow::bail!("The -c option is not supported on Windows")
}
