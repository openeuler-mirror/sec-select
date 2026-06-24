use crate::cmd::completions::Shell;
use clap::{Parser, Subcommand};
use clap_complete::{
    engine::ValueCompleter, ArgValueCompleter, CompletionCandidate, PathCompleter,
};
use std::path::PathBuf;

/// Mount backend type
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum MountBackend {
    /// FUSE filesystem (Linux only)
    Fuse,
    /// NFS over localhost
    Nfs,
}

// Platform-specific default: FUSE on Linux, NFS elsewhere
#[allow(clippy::derivable_impls)]
impl Default for MountBackend {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        {
            MountBackend::Fuse
        }
        #[cfg(not(target_os = "linux"))]
        {
            MountBackend::Nfs
        }
    }
}

impl std::fmt::Display for MountBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountBackend::Fuse => write!(f, "fuse"),
            MountBackend::Nfs => write!(f, "nfs"),
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "secafs")]
#[command(version = env!("SECAFS_VERSION"))]
#[command(about = "SecAFS - Secure Agent Filesystem (PostgreSQL)", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage shell completions
    Completions {
        #[command(subcommand)]
        command: CompletionsCommand,
    },
    /// Initialize a new agent filesystem database in PostgreSQL
    Init {
        /// PostgreSQL connection URL (e.g. postgres://user:pass@host/dbname)
        #[arg(env = "SECAFS_POSTGRES_URL")]
        postgres_url: String,

        /// Base directory for overlay filesystem (copy-on-write)
        #[arg(long)]
        base: Option<PathBuf>,

        /// Command to execute after initialization (mounts the filesystem, runs command, unmounts)
        #[arg(short = 'c', long = "command")]
        command: Option<String>,

        /// Backend to use for mounting when using -c (default: fuse on Linux, nfs on macOS)
        #[arg(long, default_value_t = MountBackend::default())]
        backend: MountBackend,
    },
    /// Filesystem operations
    Fs {
        /// PostgreSQL connection URL (e.g. postgres://user:pass@host/dbname)
        #[arg(env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        postgres_url: String,

        #[command(subcommand)]
        command: FsCommand,
    },
    /// Run a command in the sandboxed environment.
    ///
    /// Uses copy-on-write overlay backed by a PostgreSQL database.
    /// All changes are captured in a per-session database, leaving
    /// the original files untouched.
    Run {
        /// Allow write access to additional directories (can be specified multiple times)
        #[arg(long = "allow", value_name = "PATH")]
        allow: Vec<PathBuf>,

        /// Disable default allowed directories (~/.config, ~/.cache, ~/.local, ~/.npm, etc.)
        #[arg(long = "no-default-allows")]
        no_default_allows: bool,

        /// Use experimental ptrace-based syscall interception sandbox
        #[arg(long = "experimental-sandbox")]
        experimental_sandbox: bool,

        /// Enable strace-like output for system calls
        /// Only used with --experimental-sandbox
        #[arg(long = "strace")]
        strace: bool,

        /// Session identifier for sharing delta layer across multiple runs.
        /// If not provided, a unique session ID is generated for each run.
        #[arg(long = "session", value_name = "ID")]
        session: Option<String>,

        /// Allow other system users to access this mount (requires /etc/fuse.conf
        /// user_allow_other; use cautiously)
        #[arg(long = "system")]
        system: bool,

        /// PostgreSQL connection URL for the delta layer
        /// (defaults to postgres://localhost/secafs_session_<id>)
        #[arg(long, env = "SECAFS_POSTGRES_URL")]
        postgres_url: Option<String>,

        /// Command to execute (defaults to bash on Linux, zsh on macOS)
        command: Option<PathBuf>,

        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Execute a command with a SecAFS filesystem mounted.
    ///
    /// Mounts the specified SecAFS to a temporary directory, runs the command
    /// with that directory as the working directory, then automatically unmounts.
    #[cfg(unix)]
    Exec {
        /// PostgreSQL connection URL
        #[arg(value_name = "POSTGRES_URL", env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        postgres_url: String,

        /// Command to execute
        #[arg(value_name = "COMMAND")]
        command: PathBuf,

        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        /// Backend to use for mounting (default: fuse on Linux, nfs on macOS)
        #[arg(long, default_value_t = MountBackend::default())]
        backend: MountBackend,
    },
    /// Mount a SecAFS filesystem using FUSE (or list mounts if no args)
    Mount {
        /// PostgreSQL connection URL (if omitted, lists current mounts)
        #[arg(value_name = "POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: Option<String>,

        /// Mount point directory
        #[arg(value_name = "MOUNTPOINT", add = ArgValueCompleter::new(PathCompleter::dir()))]
        mountpoint: Option<PathBuf>,

        /// Automatically unmount on exit
        #[arg(short = 'a', long)]
        auto_unmount: bool,

        /// Allow root user to access filesystem
        #[arg(long)]
        allow_root: bool,

        /// Allow other system users to access this mount (requires /etc/fuse.conf
        /// user_allow_other; use cautiously)
        #[arg(long = "system")]
        system: bool,

        /// Run in foreground (don't daemonize)
        #[arg(short = 'f', long)]
        foreground: bool,

        /// User ID to report for all files (defaults to current user)
        #[arg(long)]
        uid: Option<u32>,

        /// Group ID to report for all files (defaults to current group)
        #[arg(long)]
        gid: Option<u32>,

        /// Backend to use for mounting
        #[arg(long, default_value_t = MountBackend::default())]
        backend: MountBackend,
    },
    /// Show differences between base filesystem and delta (overlay mode only)
    Diff {
        /// PostgreSQL connection URL
        #[arg(value_name = "POSTGRES_URL", env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,
    },
    /// Display agent action timeline from tool call audit log
    Timeline {
        /// PostgreSQL connection URL
        #[arg(env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,

        /// Limit number of entries to display
        #[arg(long, default_value = "100")]
        limit: i64,

        /// Filter by tool name
        #[arg(long)]
        filter: Option<String>,

        /// Filter by status (pending/success/error)
        #[arg(long, value_parser = ["pending", "success", "error"])]
        status: Option<String>,

        /// Output format
        #[arg(long, default_value = "table", value_parser = ["table", "json"])]
        format: String,
    },
    /// Start an NFS server to export a SecAFS filesystem over the network
    /// (deprecated: use `secafs serve nfs` instead)
    #[cfg(unix)]
    Nfs {
        /// PostgreSQL connection URL
        #[arg(value_name = "POSTGRES_URL", env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,

        /// IP address to bind to
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        /// Port to listen on
        #[arg(long, default_value = "11111")]
        port: u32,
    },

    /// Start an MCP server exposing filesystem and key-value store tools
    /// (deprecated: use `secafs serve mcp` instead)
    McpServer {
        /// PostgreSQL connection URL
        #[arg(value_name = "POSTGRES_URL", env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,

        /// Tools to expose (comma-separated). If not provided, all tools are exposed.
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<String>>,
    },

    /// Serve a SecAFS filesystem via different protocols
    Serve {
        #[command(subcommand)]
        command: ServeCommand,
    },
    /// List active SecAFS run sessions
    Ps,
    /// Prune unused resources
    Prune {
        #[command(subcommand)]
        command: PruneCommand,
    },
    /// Migrate database schema to the current version
    Migrate {
        /// PostgreSQL connection URL
        #[arg(env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,

        /// Preview migration without applying changes
        #[arg(long)]
        dry_run: bool,

        /// Downgrade to a specific schema version (e.g. "0.5")
        #[arg(long = "target-version")]
        target_version: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum FsCommand {
    /// List files in the filesystem
    Ls {
        /// Path to list (default: /)
        #[arg(default_value = "/")]
        fs_path: String,
    },
    /// Display file contents
    Cat {
        /// Path to the file in the filesystem
        file_path: String,
    },
    /// Write file content
    Write {
        /// Path to the file in the filesystem
        file_path: String,

        /// Content of the file
        content: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ServeCommand {
    /// Start an NFS server to export a SecAFS filesystem over the network
    #[cfg(unix)]
    Nfs {
        /// PostgreSQL connection URL
        #[arg(value_name = "POSTGRES_URL", env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,

        /// IP address to bind to
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,

        /// Port to listen on
        #[arg(long, default_value = "11111")]
        port: u32,
    },

    /// Start an MCP server exposing filesystem and key-value store tools
    Mcp {
        /// PostgreSQL connection URL
        #[arg(value_name = "POSTGRES_URL", env = "SECAFS_POSTGRES_URL", add = ArgValueCompleter::new(id_or_path_completer))]
        id_or_path: String,

        /// Tools to expose (comma-separated). If not provided, all tools are exposed.
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<String>>,
    },

    /// Start the JSON-RPC daemon managing per-volume FUSE mounts
    #[cfg(target_os = "linux")]
    Api {
        /// Unix socket path (default: $XDG_RUNTIME_DIR/secafs/secafs.sock)
        #[arg(long)]
        socket: Option<std::path::PathBuf>,

        /// Postgres connection URL (default: $SECAFS_POSTGRES_URL env)
        #[arg(long, env = "SECAFS_POSTGRES_URL")]
        pg_url: String,

        /// Host-side directory where per-conversation mount points are created
        /// (default: $XDG_STATE_HOME/secafs/mounts)
        #[arg(long)]
        mount_root: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum PruneCommand {
    /// Unmount unused SecAFS mount points
    Mounts {
        /// Skip confirmation prompt and unmount immediately
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug, Clone, Copy)]
pub enum CompletionsCommand {
    /// Install shell completions to your shell rc file
    Install {
        /// Shell to install completions for (defaults to current shell)
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
    /// Uninstall shell completions from your shell rc file
    Uninstall {
        /// Shell to uninstall completions for (defaults to current shell)
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
    /// Print instructions for manual installation
    Show,
}

fn id_or_path_completer(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let mut completions = vec![];

    let path_completer = PathCompleter::any();
    let mut path_completions = path_completer.complete(current);
    completions.append(&mut path_completions);

    completions
}
