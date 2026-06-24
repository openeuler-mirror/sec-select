<p align="center">
  <h1 align="center">SecAFS</h1>
</p>

<p align="center">
  Secure Agent Filesystem — a transactional, auditable, rollback-capable filesystem for AI agents, backed by openGauss (PostgreSQL-compatible).
</p>

---

> **⚠️ Warning:** This software is in BETA. It may still contain bugs and unexpected behavior. Use caution with production data and ensure you have backups.

## 🎯 What is SecAFS?

SecAFS is a filesystem explicitly designed for AI agents, backed by **openGauss** — a PostgreSQL-compatible database that is the default backend. Just as traditional filesystems provide file and directory abstractions for applications, SecAFS provides the storage abstractions that AI agents need — with the transactional guarantees, auditability, and rollback that only a mature database can offer. Because openGauss speaks the PostgreSQL wire protocol, stock PostgreSQL works too — pass a `postgres://` URL anywhere you would pass `opengauss://`.

The SecAFS repository consists of:

* **[OpenClaw integration](integrations/openclaw)** — the flagship way to use SecAFS: a drop-in agent-gateway integration that gives every conversation its own SecAFS-backed, FUSE-mounted workspace (see below).
* **SDK** — [TypeScript](sdk/typescript), [Python](sdk/python), and [Rust](sdk/rust) libraries for programmatic filesystem access.
* **[CLI](MANUAL.md)** — mount SecAFS (FUSE on Linux, NFS on macOS), run sandboxed copy-on-write commands, and access files from a terminal.

## 🔌 Use SecAFS with OpenClaw (and other agent gateways)

SecAFS is built to sit *underneath* an agent runtime as its working filesystem. The first integration ships for **[OpenClaw](integrations/openclaw)**, and the same design extends to other agent gateways (e.g. a Hermes-style gateway) without touching the SecAFS core.

**What the integration delivers**

* **A real workspace per conversation.** Each chat session gets its own openGauss-backed, FUSE-mounted directory, and the agent runs *inside* that mount — so everything it reads and writes is captured, transactional, and auditable, with nothing escaping to the host.
* **Rollback in the loop.** Per-message and per-turn checkpoints let you rewind a conversation's filesystem to any earlier point — undo an agent's mistakes without losing the rest of the session.
* **Session export / import.** Snapshot an entire conversation (workspace + history) to a portable archive and restore it elsewhere.

**Why the plugin approach matters**

* **Zero changes to the host gateway.** SecAFS ships as an *external plugin* loaded through the gateway's public plugin interface. The OpenClaw checkout stays **pristine upstream** — it can track new releases with no merge conflicts, because all SecAFS-specific code lives in [`integrations/openclaw/`](integrations/openclaw).
* **Rides stable public contracts only.** The integration depends on just two things: the gateway's wire protocol and its plugin SDK. The agent's working directory is redirected into the SecAFS mount through an upstream-supported mechanism — no core patches, no private hooks.
* **Gateway-agnostic by construction.** Because the coupling is the standard gateway protocol + plugin SDK (not a fork), the same SecAFS daemon and backend can power additional agent gateways and runtimes as they emerge — OpenClaw today, others (Hermes-style gateways and beyond) tomorrow.

The **[Getting Started](#-getting-started)** section below shows how to bring this up; **[`integrations/openclaw/README.md`](integrations/openclaw/README.md)** has the full architecture and configuration.

## 🧑‍💻 Getting Started

### In OpenClaw (agent chat)

Run a full SecAFS-backed chat where each conversation gets its own FUSE-mounted, openGauss-backed workspace and the agent works *inside* it. SecAFS loads into a **pristine** OpenClaw as an external plugin — you never modify the OpenClaw checkout.

Use the sibling layout `<workspace>/{openclaw, secafs}`, then:

```bash
WS=<workspace>        # the directory containing openclaw/ and secafs/

# 1. Pristine OpenClaw — zero changes (one-time)
git -C "$WS" clone https://github.com/openclaw/openclaw.git
git -C "$WS/openclaw" checkout v2026.6.8-alpha.1
( cd "$WS/openclaw" && pnpm install && pnpm build )

# 2. Build the SecAFS daemon and the external plugin
( cd "$WS/secafs/cli" && cargo build -p secafs --no-default-features )
( cd "$WS/secafs/integrations/openclaw/plugin" && npm install && npm run build )

# 3. openGauss + the stack (daemon + gateway in one userns) + the browser bridge
( cd "$WS/secafs" && docker compose -f docker-compose.dev.yml --profile opengauss up -d opengauss )
( cd "$WS/secafs/integrations/openclaw/bridge" && bash run-stack.sh & )
( cd "$WS/secafs/integrations/openclaw/bridge" && \
  OPENCLAW_DIR="$WS/openclaw" \
  GATEWAY_TOKEN=$(node -e "console.log(require(require('os').homedir()+'/.openclaw/openclaw.json').gateway.auth.token)") \
  PORT=8090 node bridge.mjs & )

# 4. open the SecAFS Console, start a session, and chat
echo "open http://127.0.0.1:8090"
```

Before step 3 you configure `~/.openclaw/openclaw.json` once — the model provider plus the external plugin path (`plugins.load.paths`). The full config, prerequisites (Linux user namespaces, the one-time daemon build symlink), and a troubleshooting guide are in **[`integrations/openclaw/README.md`](integrations/openclaw/README.md)**; the bare copy-paste sequence is in **[`integrations/openclaw/RUNBOOK.md`](integrations/openclaw/RUNBOOK.md)**.

### Using the CLI

Initialize a SecAFS database in openGauss:

```bash
$ secafs init opengauss://user:pass@localhost/my_agent
Created agent filesystem in openGauss
Database: opengauss://user:pass@localhost/my_agent
```

> Prefer stock PostgreSQL? Swap the scheme: `secafs init postgres://user:pass@localhost/my_agent`.

Inspect the agent filesystem:

```bash
$ secafs fs opengauss://localhost/my_agent ls
f hello.txt

$ secafs fs opengauss://localhost/my_agent cat hello.txt
hello from agent
```

View the agent's action timeline:

```bash
$ secafs timeline opengauss://localhost/my_agent
ID   TOOL                 STATUS       DURATION STARTED
4    execute_code         pending            -- 2024-01-05 09:44:20
3    api_call             error           300ms 2024-01-05 09:44:15
2    read_file            success          50ms 2024-01-05 09:44:10
1    web_search           success        1200ms 2024-01-05 09:43:45
```

Mount a SecAFS filesystem using FUSE (Linux) or NFS (macOS):

```bash
$ secafs mount opengauss://localhost/my_agent ./mnt
$ echo "hello" > ./mnt/hello.txt
$ cat ./mnt/hello.txt
hello
```

Run a program in a sandbox with copy-on-write overlay:

```bash
$ secafs run bash
Welcome to SecAFS!

The following directories are writable:
  - /home/user/project (copy-on-write)

Everything else is read-only.
```

Read the **[User Manual](MANUAL.md)** for complete documentation.

### Using the SDK

Install the SDK in your project:

```bash
npm install secafs-sdk
```

Use it in your agent code:

```typescript
import { SecAFS } from 'secafs-sdk';

// Connect to openGauss (a postgres:// URL works too)
const agent = await SecAFS.open({ postgresUrl: 'opengauss://localhost/my_agent' });

// Key-Value operations
await agent.kv.set('user:preferences', { theme: 'dark' });
const prefs = await agent.kv.get('user:preferences');

// Filesystem operations
await agent.fs.writeFile('/output/report.pdf', pdfBuffer);
const files = await agent.fs.readdir('/output');

// Tool call tracking
await agent.tools.record(
  'web_search',
  Date.now() / 1000,
  Date.now() / 1000 + 1.5,
  { query: 'AI' },
  { results: [...] }
);
```

## 💡 Why SecAFS?

SecAFS provides the following benefits for agent state management:

* **Sandbox Isolation**: All file operations are confined within an openGauss database, isolated from the host filesystem. External contamination cannot enter, and dangerous operations cannot spread.
* **Auditability**: Every file operation, tool call, and state change is recorded. Query your agent's complete history with SQL to debug issues, analyze behavior, or meet compliance requirements.
* **Agent Operation Rollback**: Treat multiple agent operations as a single transaction. Intermediate changes are invisible until the task completes. Supports step-by-step checkpointing (`SAVEPOINT`), snapshots, and rollback.
* **Collaboration Isolation**: File modifications are transactional. Multiple agents operating concurrently on the filesystem are isolated from each other via openGauss MVCC.
* **Remote Deployment**: Leverage mature, managed openGauss (or any PostgreSQL-compatible) infrastructure for efficient remote deployment of agent environments.
* **Portability**: Export an entire agent's state as a portable archive for backup, snapshot recovery, and distribution.

## 🔧 How SecAFS Works

SecAFS is an agent filesystem accessible through an SDK that provides three essential interfaces for agent state management:

* **Filesystem:** a POSIX-like filesystem for files and directories.
* **Key-Value:** a key-value store for agent state and context.
* **Toolcall:** a tool-call audit trail for debugging and analysis.

At the heart of SecAFS is an openGauss-backed storage engine. Everything an agent does — every file it creates, every piece of state it stores, every tool it invokes — lives in an openGauss database with full transactional guarantees. The FUSE/NFS layer projects that database as an ordinary directory tree, so existing tools and agents work unchanged while every change stays captured and reversible.

## Dev database + daemon

For local development, bring up an openGauss instance with the bundled docker-compose:

```bash
./scripts/dev-up.sh                  # openGauss (default)
export SECAFS_POSTGRES_URL='opengauss://secafs:Secafs%21123@localhost:5433/secafs'
cargo run -p secafs -- serve api
```

Prefer stock PostgreSQL instead?

```bash
./scripts/dev-up.sh postgres
export SECAFS_POSTGRES_URL=postgres://secafs:secafs@localhost:5433/secafs
```

Stop:

```bash
./scripts/dev-down.sh
```

For production deployments, supply your own openGauss (or PostgreSQL) via `--pg-url` or `$SECAFS_POSTGRES_URL`.

## 🌱 Origins & what's different

SecAFS's architecture descends from **agentfs**, an open-source agent-filesystem framework that pioneered the idea of exposing a database as an agent's filesystem. SecAFS keeps that core idea but is a substantially reworked system, redesigned for production agent infrastructure:

* **Industrial-grade database instead of an embedded one.** SecAFS runs on **openGauss / PostgreSQL** over the wire protocol rather than an embedded SQLite engine — bringing MVCC concurrency, managed and remote deployment, role-based access, and compliance-grade durability to the agent filesystem.
* **Rollback and checkpointing as first-class features.** Transactional agent-operation rollback, snapshots, and per-message/per-turn checkpoints let you rewind an agent's filesystem to any earlier state.
* **A different target scenario.** SecAFS is designed around multi-conversation, multi-agent infrastructure — per-conversation isolated workspaces, concurrent-agent isolation via MVCC, and direct integration with agent gateways (see [Use SecAFS with OpenClaw](#-use-secafs-with-openclaw-and-other-agent-gateways)).
* **Operational hardening.** Session export/import, a mount-keeper that heals lost mounts, and a self-healing database connection pool make long-running deployments robust.

## 📚 Learn More

- **[OpenClaw integration](integrations/openclaw/README.md)** — run a full SecAFS-backed agent chat.
- **[User Manual](MANUAL.md)** — complete guide to using the SecAFS CLI and SDK.
- **[SPEC.md](SPEC.md)** — the on-database schema and data model.

## 📝 License

Mulan Permissive Software License, Version 2 (MulanPSL-2.0) — see http://license.coscl.org.cn/MulanPSL2
