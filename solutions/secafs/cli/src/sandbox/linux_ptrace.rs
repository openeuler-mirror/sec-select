//! Ptrace-based sandbox implementation using reverie.
//!
//! This module provides syscall interception via ptrace for filesystem
//! virtualization. This is experimental and requires root or CAP_SYS_PTRACE.
//!
//! NOTE: The ptrace sandbox currently requires the secafs_sandbox crate
//! which needs to be updated for PostgreSQL. For now, it will report
//! that it is not yet available with PostgreSQL-only mode.

use secafs_sandbox::{
    init_fd_tables, init_mount_table, init_strace, MountTable, Sandbox,
};
use reverie_process::Command;
use reverie_ptrace::TracerBuilder;
use std::path::PathBuf;

/// Run a command using the experimental ptrace-based syscall interception sandbox.
pub async fn run_cmd(strace: bool, command: PathBuf, args: Vec<String>) {
    eprintln!("Welcome to SecAFS!");
    eprintln!();
    eprintln!("Warning: The experimental ptrace sandbox is not yet available");
    eprintln!("with PostgreSQL-only mode. Please use the default sandbox instead.");
    eprintln!();

    let mount_table = MountTable::new();

    init_mount_table(mount_table);
    init_fd_tables();
    init_strace(strace);

    let mut cmd = Command::new(command);
    for arg in args {
        cmd.arg(arg);
    }

    let tracer = TracerBuilder::<Sandbox>::new(cmd).spawn().await.unwrap();

    let (status, _) = tracer.wait().await.unwrap();
    status.raise_or_exit()
}
