use secafs::{
    cmd::{self, completions::handle_completions},
    get_runtime,
    opts::{Args, Command, FsCommand, PruneCommand, ServeCommand},
};
use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use tracing_subscriber::prelude::*;

fn main() {
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "secafs=info".into()),
        )
        .try_init();

    reset_sigpipe();

    CompleteEnv::with_factory(Args::command).complete();
    let args = Args::parse();

    match args.command {
        Command::Init {
            postgres_url,
            base,
            command,
            backend,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::init::init_database(
                postgres_url,
                base,
                command,
                backend,
            )) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Run {
            allow,
            no_default_allows,
            experimental_sandbox,
            strace,
            session,
            system,
            postgres_url,
            command,
            args,
        } => {
            let command = command.unwrap_or_else(default_shell);
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::handle_run_command(
                allow,
                no_default_allows,
                experimental_sandbox,
                strace,
                session,
                system,
                postgres_url,
                command,
                args,
            )) {
                eprintln!("Error: {e:?}");
                std::process::exit(1);
            }
        }
        #[cfg(unix)]
        Command::Exec {
            postgres_url,
            command,
            args,
            backend,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::exec::handle_exec_command(
                postgres_url, command, args, backend,
            )) {
                eprintln!("Error: {e:?}");
                std::process::exit(1);
            }
        }
        Command::Mount {
            id_or_path,
            mountpoint,
            auto_unmount,
            allow_root,
            system,
            foreground,
            uid,
            gid,
            backend,
        } => match (id_or_path, mountpoint) {
            (Some(id_or_path), Some(mountpoint)) => {
                if let Err(e) = cmd::mount(cmd::MountArgs {
                    id_or_path,
                    mountpoint,
                    auto_unmount,
                    allow_root,
                    allow_other: system,
                    foreground,
                    uid,
                    gid,
                    backend,
                }) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            (None, None) => {
                cmd::mount::list_mounts(&mut std::io::stdout());
            }
            _ => {
                eprintln!("Error: both POSTGRES_URL and MOUNTPOINT are required to mount");
                std::process::exit(1);
            }
        },
        Command::Diff { id_or_path } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::fs::diff_filesystem(id_or_path)) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Timeline {
            id_or_path,
            limit,
            filter,
            status,
            format,
        } => {
            let rt = get_runtime();
            let options = cmd::timeline::TimelineOptions {
                limit,
                filter,
                status,
                format,
            };
            if let Err(e) = rt.block_on(cmd::timeline::show_timeline(
                &mut std::io::stdout(),
                &id_or_path,
                &options,
            )) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Fs {
            command,
            postgres_url,
        } => {
            let rt = get_runtime();
            match command {
                FsCommand::Ls { fs_path } => {
                    if let Err(e) = rt.block_on(cmd::fs::ls_filesystem(
                        &mut std::io::stdout(),
                        postgres_url,
                        &fs_path,
                    )) {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
                FsCommand::Cat { file_path } => {
                    if let Err(e) = rt.block_on(cmd::fs::cat_filesystem(
                        &mut std::io::stdout(),
                        postgres_url,
                        &file_path,
                    )) {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
                FsCommand::Write { file_path, content } => {
                    if let Err(e) = rt.block_on(cmd::fs::write_filesystem(
                        postgres_url,
                        &file_path,
                        &content,
                    )) {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Command::Completions { command } => handle_completions(command),
        #[cfg(unix)]
        Command::Nfs {
            id_or_path,
            bind,
            port,
        } => {
            eprintln!("Warning: `secafs nfs` is deprecated, use `secafs serve nfs` instead");
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::nfs::handle_nfs_command(id_or_path, bind, port)) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::McpServer { id_or_path, tools } => {
            eprintln!(
                "Warning: `secafs mcp-server` is deprecated, use `secafs serve mcp` instead"
            );
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::mcp_server::handle_mcp_server_command(
                id_or_path, tools,
            )) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Serve { command } => match command {
            #[cfg(unix)]
            ServeCommand::Nfs {
                id_or_path,
                bind,
                port,
            } => {
                let rt = get_runtime();
                if let Err(e) = rt.block_on(cmd::nfs::handle_nfs_command(id_or_path, bind, port)) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            ServeCommand::Mcp { id_or_path, tools } => {
                let rt = get_runtime();
                if let Err(e) = rt.block_on(cmd::mcp_server::handle_mcp_server_command(
                    id_or_path, tools,
                )) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            #[cfg(target_os = "linux")]
            ServeCommand::Api {
                socket,
                pg_url,
                mount_root,
            } => {
                let rt = get_runtime();
                let serve_args = cmd::serve_api::ServeApiArgs {
                    socket: socket.unwrap_or_else(default_socket_path),
                    pg_url,
                    mount_root: mount_root.unwrap_or_else(default_mount_root),
                };
                if let Err(e) = rt.block_on(cmd::serve_api::run(serve_args)) {
                    eprintln!("Error: {e:?}");
                    std::process::exit(1);
                }
            }
        },
        Command::Ps => {
            if let Err(e) = cmd::ps::list_ps(&mut std::io::stdout()) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Prune { command } => match command {
            PruneCommand::Mounts { force } => {
                if let Err(e) = cmd::mount::prune_mounts(force) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        },
        Command::Migrate {
            id_or_path,
            dry_run,
            target_version,
        } => {
            let rt = get_runtime();
            if let Err(e) = rt.block_on(cmd::migrate::handle_migrate_command(
                &mut std::io::stdout(),
                id_or_path,
                dry_run,
                target_version,
            )) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

/// Reset SIGPIPE to the default behavior (terminate the process) so that
/// piping output to tools like `head` doesn't cause a panic.
#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

/// Returns the default shell for the current platform.
fn default_shell() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        std::path::PathBuf::from("zsh")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::path::PathBuf::from("bash")
    }
}

/// Default Unix socket path for `secafs serve api`.
///
/// Resolves to `$XDG_RUNTIME_DIR/secafs/secafs.sock`, falling back to
/// `/run/user/<uid>/secafs/secafs.sock` when the env var is absent.
#[cfg(target_os = "linux")]
fn default_socket_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let uid = unsafe { libc::getuid() };
            std::path::PathBuf::from(format!("/run/user/{uid}"))
        });
    base.join("secafs/secafs.sock")
}

/// Default mount-root directory for `secafs serve api`.
///
/// Resolves to `$XDG_STATE_HOME/secafs/mounts`, falling back to
/// `$HOME/.local/state/secafs/mounts` or `/tmp/secafs/mounts`.
#[cfg(target_os = "linux")]
fn default_mount_root() -> std::path::PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".local/state"))
                .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
        })
        .join("secafs/mounts")
}
