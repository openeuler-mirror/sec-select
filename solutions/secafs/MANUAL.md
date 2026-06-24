# SecAFS Reference Guide

Command-line reference for the SecAFS CLI.

## Installation

```bash
curl -fsSL https://secafs.ai/install | bash
```

## Commands

## Database Backend

SecAFS uses **openGauss** as its default database backend. openGauss is
PostgreSQL-compatible, so all commands accept either an `opengauss://` or a
`postgres://` connection URL (e.g. `opengauss://user:pass@host:5432/db` or
`postgres://user:pass@host:5432/db`). The `opengauss://` scheme is normalized to
`postgres://` internally for the database driver — stock PostgreSQL works
unchanged.

You can also set the `SECAFS_POSTGRES_URL` environment variable to avoid passing the
URL on every command.

**Testing with local test1:** To run a quick demo against a local PostgreSQL database named `test1` (e.g. user `secafs` / password `secafs`), use the script:

```bash
# 使用脚本默认 URL: postgres://secafs:secafs@127.0.0.1:5432/test1
./scripts/demo-cli-test1.sh

# 或指定 URL
SECAFS_POSTGRES_URL='postgres://user:pass@127.0.0.1:5432/test1' ./scripts/demo-cli-test1.sh
```

The script runs `migrate`, then `fs ls/write/cat` and `timeline`. If `fs` or `timeline` hang in your environment, use the Python SDK against the same database.

### secafs init

Initialize a new agent filesystem in an openGauss (or PostgreSQL) database.

```
secafs init [OPTIONS] <POSTGRES_URL>
```

**Arguments:**
- `POSTGRES_URL` - PostgreSQL connection URL

**Options:**
- `--base <PATH>` - Base directory for overlay filesystem (copy-on-write)
- `-c, --command <CMD>` - Command to execute after initialization (see below)
- `--backend <BACKEND>` - Mount backend for `-c` option (`fuse` or `nfs`)

**Running a command after init:**

The `-c` option initializes the filesystem, mounts it to a temporary directory, runs the specified command with that directory as the working directory, then automatically unmounts.

```bash
# Initialize and run a command in the new filesystem
secafs init opengauss://localhost/my_agent -c "touch hello.txt && ls -la"

# With overlay filesystem
secafs init opengauss://localhost/my_overlay --base /path/to/project -c "make build"
```

### secafs exec

Execute a command with a SecAFS filesystem mounted (Unix only).

```
secafs exec [OPTIONS] <POSTGRES_URL> <COMMAND> [ARGS]...
```

Mounts the specified SecAFS to a temporary directory, runs the command with that directory as the working directory, then automatically unmounts.

**Arguments:**
- `POSTGRES_URL` - PostgreSQL connection URL
- `COMMAND` - Command to execute
- `ARGS` - Arguments for the command

**Options:**
- `--backend <BACKEND>` - Mount backend (`fuse` on Linux, `nfs` on macOS by default)

**Examples:**

```bash
# Run ls in the SecAFS root
secafs exec opengauss://localhost/my_agent ls -la

# Run a build command
secafs exec opengauss://localhost/my_overlay make build
```

### secafs run

Execute a program in a sandboxed environment with copy-on-write filesystem.

```
secafs run [OPTIONS] <COMMAND> [ARGS]...
```

**Options:**
- `--session <ID>` - Named session for persistence across runs
- `--allow <PATH>` - Allow write access to additional directories (repeatable)
- `--no-default-allows` - Disable default allowed directories
- `--postgres-url <URL>` - openGauss/PostgreSQL URL for the delta layer (default: auto-generated)
- `--experimental-sandbox` - Use ptrace-based syscall interception (Linux only)
- `--strace` - Show intercepted syscalls (requires `--experimental-sandbox`)

**Platform behavior:**

Linux uses FUSE + overlay filesystem with user namespaces. macOS uses NFS + overlay filesystem with Apple's Sandbox.

Default allowed directories (macOS): `~/.config`, `~/.cache`, `~/.local`, `~/.npm`, `/tmp`, plus the config directories of common coding agents.

### secafs mount

Mount a SecAFS filesystem or list mounted filesystems.

```
secafs mount [OPTIONS] [POSTGRES_URL] [MOUNT_POINT]
```

Without arguments, lists all mounted SecAFS filesystems.

**Options:**
- `-a, --auto-unmount` - Automatically unmount on exit
- `--allow-root` - Allow root user to access filesystem
- `-f, --foreground` - Run in foreground
- `--uid <UID>` - User ID for all files
- `--gid <GID>` - Group ID for all files

**Unmounting:**
- Linux: `fusermount -u <MOUNT_POINT>`
- macOS: `umount <MOUNT_POINT>`

### secafs serve mcp

Start an MCP (Model Context Protocol) server.

```
secafs serve mcp <POSTGRES_URL> [OPTIONS]
```

**Options:**
- `--tools <TOOLS>` - Comma-separated list of tools to expose (default: all)

**Available tools:**

Filesystem: `read_file`, `write_file`, `readdir`, `mkdir`, `remove`, `rename`, `stat`, `access`

Key-Value: `kv_get`, `kv_set`, `kv_delete`, `kv_list`

### secafs serve nfs

Start an NFS server to export SecAFS over the network.

```
secafs serve nfs <POSTGRES_URL> [OPTIONS]
```

**Options:**
- `--bind <IP>` - IP address to bind (default: `127.0.0.1`)
- `--port <PORT>` - Port to listen on (default: `11111`)

**Mounting from client:**
```bash
mount -t nfs -o vers=3,tcp,port=11111,mountport=11111,nolock <HOST>:/ <MOUNT_POINT>
```

### secafs migrate

Migrate database schema to the current version.

```
secafs migrate [OPTIONS] <POSTGRES_URL>
```

Upgrades a SecAFS database schema to the latest version. This is necessary when using databases created with older versions of SecAFS.

**Arguments:**
- `POSTGRES_URL` - PostgreSQL connection URL

**Options:**
- `--dry-run` - Preview migration without applying changes

**Examples:**

```bash
# Preview pending migrations
secafs migrate opengauss://localhost/my_agent --dry-run

# Apply migrations
secafs migrate opengauss://localhost/my_agent
```

### secafs fs

Filesystem operations on agent databases.

#### secafs fs ls

```
secafs fs <POSTGRES_URL> ls [FS_PATH]
```

List files and directories. Output: `f <name>` for files, `d <name>` for directories.

#### secafs fs cat

```
secafs fs <POSTGRES_URL> cat <FILE_PATH>
```

Display file contents.

#### secafs fs write

```
secafs fs <POSTGRES_URL> write <FILE_PATH> <CONTENT>
```

Write content to a file.

### secafs diff

Show filesystem changes in overlay mode.

```
secafs diff <POSTGRES_URL>
```

### secafs timeline

Display agent action timeline from the tool call audit log.

```
secafs timeline [OPTIONS] <POSTGRES_URL>
```

**Options:**
- `--limit <N>` - Limit entries (default: 100)
- `--filter <TOOL>` - Filter by tool name
- `--status <STATUS>` - Filter by status: `pending`, `success`, `error`
- `--format <FORMAT>` - Output format: `table`, `json` (default: table)

### secafs completions

Manage shell completions.

```
secafs completions install [SHELL]
secafs completions uninstall [SHELL]
secafs completions show
```

Supported shells: `bash`, `zsh`, `fish`, `powershell`

## Environment Variables

**Configuration variables:**

| Variable | Description |
|----------|-------------|
| `SECAFS_POSTGRES_URL` | Default openGauss/PostgreSQL connection URL (accepts `opengauss://` or `postgres://`) |

**Variables set inside the sandbox:**

| Variable | Description |
|----------|-------------|
| `SECAFS` | Set to `1` inside SecAFS sandbox |
| `SECAFS_SANDBOX` | Sandbox type: `macos-sandbox` or `linux-namespace` |
| `SECAFS_SESSION` | Current session ID |

## Files

- `~/.secafs/run/<SESSION_ID>/` - Session run directories

## See Also

- See `SPEC.md` for the data model and `README.md` for an overview.
