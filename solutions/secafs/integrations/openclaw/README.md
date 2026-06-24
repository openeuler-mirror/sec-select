# SecAFS ↔ OpenClaw integration

Run a full SecAFS-backed AI chat where **each conversation gets its own
openGauss/PostgreSQL-backed, FUSE-mounted workspace**, and the agent operates
*inside* that mount. The OpenClaw checkout stays **pristine upstream — zero
changes** — so it can track upstream releases. Everything SecAFS-specific lives
here, under `secafs/integrations/openclaw/`.

```
 Browser (SecAFS Console)
        │  plain WebSocket
        ▼
 bridge.mjs  ──────────────  holds the write-scoped device credential
        │  gateway protocol (operator, write)
        ▼
 OpenClaw gateway  ────────  PRISTINE upstream v2026.6.8-alpha.1
        │  loads `secafs-chat` as an EXTERNAL plugin (via plugins.load.paths)
        ▼
 secafs-chat plugin  ──────  secafs.* gateway methods, Path C cwd redirect,
        │  Unix-socket JSON-RPC      mount-keeper, rollback, export/import
        ▼
 secafs serve api (Rust daemon)
        │
        ▼
 openGauss / PostgreSQL  +  per-conversation FUSE mounts
```

This directory:

| Path | What |
|---|---|
| `plugin/` | The external `secafs-chat` OpenClaw plugin (TypeScript). |
| `frontend/` | `SecAFS Console` — a dependency-free single-page gateway client. |
| `bridge/` | `run-stack.sh` (daemon + gateway in one userns), `bridge.mjs` (browser↔gateway proxy + static server), `secafs-ns.sh` (inspect FUSE mounts from the host). |
| `INTEGRATION.md` | Architecture notes (gateway-client form; Path C). |
| `RUNBOOK.md` | Copy-paste command sequence (defers concepts to this README). |
| `docs/secafs-rollback.md` | Rollback design notes. |

---

## Why a bridge?

The gateway binds operator **write** scopes to device-signature auth (Ed25519
challenge); a browser bearer token only ever gets read-only. `bridge.mjs` holds
the local paired-device credential (via the SDK's `callGatewayFromCli`, the same
path the CLI uses) and proxies the browser's gateway frames with write scopes.
The browser needs no crypto — it talks plain WebSocket to the bridge.

---

## Prerequisites

- **Linux** with `/dev/fuse` present and **user namespaces enabled**. No `sudo`,
  no `fusermount3` — mounts happen inside an `unshare --user --mount` namespace.
- **Node 22+** and **pnpm** (for OpenClaw), plus **npm** (for the plugin build).
- **Rust** toolchain (for the daemon).
- **Docker** (for openGauss).
- A **MiniMax API key** (this setup defaults to MiniMax M3; any
  OpenClaw-supported provider works — adjust the config in step 4).

Recommended sibling layout (the scripts default to these paths):

```
<workspace>/
├── openclaw/     # pristine upstream checkout (step 1)
└── secafs/       # this repo
    └── integrations/openclaw/   # you are here
```

---

## 1. Install OpenClaw — pristine, zero changes

The integration loads from *outside* the OpenClaw tree, so you never edit it.

```bash
cd <workspace>
git clone https://github.com/openclaw/openclaw.git
cd openclaw
git checkout v2026.6.8-alpha.1     # the version the plugin targets
pnpm install
pnpm build                          # REQUIRED: the bridge imports
                                    # dist/plugin-sdk/gateway-runtime.js
```

That is the whole OpenClaw install. Do **not** copy any SecAFS files into it;
the plugin is wired in by config (step 4), not by modifying the checkout. You
can confirm it stays clean at any time with `git -C <workspace>/openclaw status`
(should report a clean working tree).

> Using a different OpenClaw version? The plugin pins `openclaw >=2026.6.8-alpha.1`
> as a peer dependency and relies on the public `openclaw/plugin-sdk/*` surface
> plus the gateway protocol. Newer tags should work; if a method moves, rebuild
> the plugin and re-run its tests (step 3).

---

## 2. Build the secafs daemon

```bash
# one-time: provide the liblzma dev symlink the linker wants (only .so.5 ships)
mkdir -p ~/.local/secafs-buildlibs
ln -sf /usr/lib/x86_64-linux-gnu/liblzma.so.5 ~/.local/secafs-buildlibs/liblzma.so

cd <workspace>/secafs/cli          # NB: the Cargo workspace root is cli/, not the repo root
LIBRARY_PATH=~/.local/secafs-buildlibs cargo build -p secafs --no-default-features
```

- `--no-default-features` drops the ptrace sandbox crate (needs system
  `libunwind-ptrace`); FUSE and the daemon are unaffected.
- Binary lands at `secafs/cli/target/debug/secafs`. `run-stack.sh` puts it on
  `PATH` automatically.

---

## 3. Build & install the plugin

The plugin ships compiled JS. Its manifest already declares
`activation.onStartup: true` — **required**, or the gateway discovers the plugin
but never runs `register()` and every `secafs.*` call returns "unknown method".

```bash
cd <workspace>/secafs/integrations/openclaw/plugin
npm install                         # esbuild + vitest + types
npm run build                       # → dist/index.js
```

"Installing" the plugin into the gateway = pointing the gateway's config at this
directory (step 4, `plugins.load.paths`). We deliberately **do not** use
`openclaw plugins install`: on this version it hangs ("Terminated") before
writing the install record, because the plugin's background timers keep the
short-lived install process alive. `plugins.load.paths` is the reliable path and
keeps the plugin fully external.

Optional checks:

```bash
npm test                            # 116 unit tests
npm run typecheck                   # needs node_modules/openclaw → the checkout
```

> **Gotcha:** `npm install` overwrites the `node_modules/openclaw` symlink the
> typecheck/tests resolve against. `npm run build` is unaffected (it bundles with
> `--packages=external`), but before `npm test`/`typecheck` re-link it:
> `ln -sfn <workspace>/openclaw plugin/node_modules/openclaw`.

---

## 4. Configure `~/.openclaw/openclaw.json`

Minimal working config (replace the API key; keep the gateway token in sync with
what you pass the bridge in step 7):

```json5
{
  gateway: { mode: "local", auth: { mode: "token", token: "<GATEWAY_TOKEN>" } },
  env: { MINIMAX_API_KEY: "sk-..." },
  agents: {
    defaults: { model: { primary: "minimax/MiniMax-M3" } },
    list: [{ id: "main", default: true }],
  },
  models: { mode: "merge", providers: { minimax: {
    baseUrl: "https://api.minimaxi.com/anthropic",   // CN key; api.minimax.io for global
    apiKey: "${MINIMAX_API_KEY}", api: "anthropic-messages",
    models: [{ id: "MiniMax-M3", name: "MiniMax M3", reasoning: true,
               input: ["text","image"], contextWindow: 1000000, maxTokens: 131072 }],
  } } },

  // Load the external plugin from this repo (NO copy into the openclaw tree):
  plugins: {
    load: { paths: ["<workspace>/secafs/integrations/openclaw/plugin"] },
    entries: { "secafs-chat": { enabled: true, config: {
      manageDaemon: false,                             // run-stack.sh runs the daemon
      socketPath: "/home/<you>/.secafs/run/secafs.sock",
      mountRoot: "/home/<you>/.secafs/mounts",
    } } },
  },
}
```

Notes:
- `manageDaemon: false` because `run-stack.sh` starts the daemon itself, in the
  **same userns** as the gateway, so FUSE mounts are visible to the agent's
  tools. (`manageDaemon: true` would have the gateway spawn the daemon and then
  requires `postgresUrl` in the plugin config — not used in this setup.)
- Runtime paths live under `~/.secafs/` on purpose — **not `/tmp`**, which is
  tmpfs + subject to `systemd-tmpfiles` aging and would wipe the socket /
  mountpoints during long-running sessions.
- Plugin config knobs (all optional except as noted):

  | key | default | meaning |
  |---|---|---|
  | `socketPath` | `$XDG_RUNTIME_DIR/secafs/secafs.sock` | daemon Unix socket |
  | `mountRoot` | `$XDG_STATE_HOME/secafs/mounts` | per-conversation mount parent |
  | `manageDaemon` | `false` | gateway spawns the daemon (needs `postgresUrl`) |
  | `idleScanSeconds` | `2` | mount-keeper / idle-scan tick (also the self-heal tick) |
  | `idleUnmountSeconds` | `0` (off) | idle auto-unmount; off by default (see RUNBOOK) |
  | `enableRollbackUI` | `true` | register `secafs.rollback.*` + snapshot on turns |

### One-time gateway auth bootstrap

Pair this device and set the gateway token:

```bash
cd <workspace>/openclaw
pnpm openclaw onboard --non-interactive --accept-risk --mode local \
  --flow quickstart --auth-choice skip --gateway-auth token --gateway-bind loopback
```

A paired device starts with `operator.pairing` only; the `secafs.*` methods need
`operator.admin`. The scope-upgrade approval can deadlock on a hand-bootstrapped
gateway, so seed the device scopes directly:

```bash
node -e '
const fs=require("fs"),os=require("os");const f=os.homedir()+"/.openclaw/devices/paired.json";
const d=JSON.parse(fs.readFileSync(f,"utf8"));
const want=["operator.pairing","operator.read","operator.write","operator.admin","operator.approvals"];
for(const id in d){const e=d[id];e.scopes=[...want];e.approvedScopes=[...want];if(e.tokens?.operator)e.tokens.operator.scopes=[...want];}
fs.writeFileSync(f,JSON.stringify(d,null,2));console.log("seeded device scopes");'
```

---

## 5. Start openGauss

```bash
cd <workspace>/secafs
docker compose -f docker-compose.dev.yml --profile opengauss up -d opengauss
#   ↳ or: ./scripts/dev-up.sh
```

- Listens on **:5433**; DSN `opengauss://secafs:Secafs!123@localhost:5433/secafs`.
- Data persists in `secafs/.dev-ogdata` (survives restarts). The container is
  `restart: unless-stopped`.
- `scripts/opengauss-init.sql` makes the `secafs` role **SYSADMIN** and creates
  the `secafs` db, so schema init works on first use with no extra grants.
- **Optional hardening:** openGauss defaults to `session_timeout=10min`, which
  closes the daemon's idle connections. The daemon now **self-heals** (rebuilds
  its pool on a dead connection), so this is no longer required, but to avoid the
  reconnect entirely:
  ```bash
  docker exec secafs-opengauss-1 bash -lc \
    "su - omm -c 'gs_guc reload -D /var/lib/opengauss/data -c \"session_timeout=0\"'"
  ```

---

## 6. Bring up daemon + gateway

```bash
cd <workspace>/secafs/integrations/openclaw/bridge
bash run-stack.sh          # daemon + gateway inside one `unshare --user --map-root-user --mount`
```

`run-stack.sh` env overrides (defaults shown): `SECAFS_BIN_DIR`,
`OPENCLAW_DIR`, `PG_URL`, `SOCK=~/.secafs/run/secafs.sock`,
`MOUNT_ROOT=~/.secafs/mounts`. It **supervises the daemon** (respawns on crash;
sweeps stale FUSE mountpoints first), so a daemon death self-recovers.

Wait for `[secafs-chat] plugin registered` and `gateway ready` in the output.

---

## 7. Start the bridge + open the console

```bash
cd <workspace>/secafs/integrations/openclaw/bridge
OPENCLAW_DIR=<workspace>/openclaw \
GATEWAY_TOKEN=$(node -e "console.log(require(require('os').homedir()+'/.openclaw/openclaw.json').gateway.auth.token)") \
PORT=8090 node bridge.mjs
```

Open **http://127.0.0.1:8090**. URL is prefilled `ws://127.0.0.1:8090`, token
blank → **Connect** → status should read `connected · read+write`.

---

## Using the console

- **＋ New** (with optional alias) creates a per-conversation FUSE volume; the
  file tree appears. Rollback snapshots are auto-enabled for new sessions.
- **Sessions list** (left): 🟢 mounted / ⚪ saved-in-DB. Click a row to open
  (mounts on demand). Per-row actions: ✏️ rename · ⬇ export · ⏏ close (unmount,
  keep data) · 🗑 destroy (permanently deletes files + chat + transcripts).
- **Chat** (center): the agent runs with cwd **inside the FUSE mount** (Path C),
  so files it creates appear in the tree and persist in openGauss.
- **Per-message rollback**: hover any agent reply → **⏪** rolls workspace files
  *and* chat history back to right after that reply. **🕘** opens the restore-
  point timeline for older points.
- **Files / Editor** (right): click a file to open, edit, **Save to SecAFS**.
- **Export / Import**: ⬇ downloads a session as `.tar.gz` (manifest + workspace
  + chat); **⬆ Import** restores one as a *fresh copy* (new id) — also works
  across machines / openGauss instances.

---

## Stop everything

```bash
# bridge + gateway by port, daemon by name (do NOT `pkill -f "gateway run"`,
# which self-matches the killing command's own line):
for port in 8090 18789; do
  P=$(ss -ltnp 2>/dev/null | grep ":$port" | grep -oP 'pid=\K[0-9]+' | head -1)
  [ -n "$P" ] && kill "$P"
done
pkill -x secafs 2>/dev/null
cd <workspace>/secafs && docker compose -f docker-compose.dev.yml stop opengauss
```

Data is safe: openGauss in `.dev-ogdata`, sessions in `~/.openclaw/agents/main/sessions/`.

---

## Inspecting FUSE mounts from the host

The mounts live inside `run-stack.sh`'s private mount namespace, so a plain host
`ls ~/.secafs/mounts/<id>` shows an **empty** directory (by design) — even for
root. To look inside, enter the daemon's namespace:

```bash
cd <workspace>/secafs/integrations/openclaw/bridge
./secafs-ns.sh ls -la ~/.secafs/mounts/<id>/
./secafs-ns.sh                 # no args → interactive shell in the namespace
```

For ground truth on *what is actually mounted*, prefer the daemon's own view:
`secafs.status`'s `mountCount`, or `cat /proc/<daemon-pid>/mounts` (note: dead
mount carcasses can linger there after a daemon crash — the daemon's RPC list is
authoritative).

---

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `secafs.* → "unknown method"` | Gateway didn't register the plugin. Confirm `activation.onStartup: true` in `plugin/openclaw.plugin.json`, `plugins.load.paths` points at the plugin dir, and you restarted the gateway after `npm run build`. |
| `missing scope: operator.admin` | Bridge connection lacks admin. Re-run the device-scope seeding (step 4). |
| `WorkspaceVanishedError … workspace appears to have disappeared` | The volume was unmounted out-of-band and OpenClaw's attestation check fired before the auto-mount hook. The mount-keeper (`idleScanSeconds`) remounts within a tick; the console also auto-reopens + retries. If persistent, click the session to reopen it. |
| Agent writes land in `~/.openclaw/workspace`, not the mount | Path C didn't apply — the canonical session entry must exist before the workspace override, and `spawnedCwd` must be written to all key forms. Rebuild the plugin (`npm run build`) and restart the gateway. |
| `mount failed: volume::ensure failed` (repeatedly) | The daemon lost its DB connection (e.g. openGauss restart). It self-heals on the next op now; if it doesn't, restart `run-stack.sh`. See the optional `session_timeout=0` hardening (step 5). |
| `mkdir failed: File exists` on remount | A dead daemon left a disconnected FUSE mountpoint. The daemon lazy-detaches these on mount and `run-stack.sh` sweeps them on respawn; if stuck, `./secafs-ns.sh umount -l ~/.secafs/mounts/<id>`. |
| Long-running processes die when a shell exits | Launch detached (`nohup … &`). `run-stack.sh` keeps daemon+gateway together. |
| `GS_PASSWORD has expired or does not meet complexity requirements` (openGauss) | openGauss enforces a password policy: at least 8 chars, with upper + lower + digit + a special char (`!@#$`). Note that `_` is **not** counted as a special character in 6.0. Pick a compliant password (e.g. `Secafs!123`). |
| `permission denied for schema public` (openGauss) | The `secafs` role lacks rights on the `public` schema. As a superuser (`gaussdb`), run `GRANT ALL ON SCHEMA public TO secafs;`. The bundled `scripts/opengauss-init.sql` does this automatically on first init. |
| Database connection failure | Check the container is up (`docker ps | grep -E "postgres|opengauss"`), the port is listening (`ss -tlnp | grep 5433`), and test the DSN directly: `psql "opengauss://secafs:Secafs!123@localhost:5433/secafs" -c "SELECT version();"`. |
| Port already in use | `5433` (database) → change the published port in the compose file; `18789` (gateway) / `8090` (bridge) → set `OPENCLAW_GATEWAY_PORT` / the bridge `PORT` env var. |
| Docker build/run behind an intranet proxy | Export `HTTP_PROXY` / `HTTPS_PROXY` and `NO_PROXY=localhost,127.0.0.1,db` before `docker compose`; Compose passes these through to the build and containers. |

See `RUNBOOK.md` for the bare command sequence and `INTEGRATION.md` for the
architecture notes.

---

## Environment variables

Quick reference for the variables this stack reads. The host-native flow above
sets most of these for you; they matter mainly for the containerized deployment.

**Database**

| Variable | Default | Meaning |
|---|---|---|
| `POSTGRES_USER` / `GS_USERNAME` | `secafs` | DB application role (Postgres / openGauss image) |
| `POSTGRES_PASSWORD` / `GS_PASSWORD` | `secafs` / `Secafs!123` | role password (openGauss has a complexity policy — see Troubleshooting) |
| `POSTGRES_DB` | `secafs` | application database name |

**OpenClaw / bridge**

| Variable | Default | Meaning |
|---|---|---|
| `OPENCLAW_GATEWAY_TOKEN` | (generated) | gateway access token; must match the bridge's `GATEWAY_TOKEN` |
| `OPENCLAW_GATEWAY_PORT` | `18789` | gateway HTTP port |
| `OPENCLAW_GATEWAY_BIND` | `loopback` | `loopback` (127.0.0.1) or `lan` (0.0.0.0) |
| `OPENCLAW_CONFIG_DIR` | `~/.openclaw` | config + paired-device directory |
| `PG_URL` (run-stack.sh) | dev DSN | DSN the daemon connects to |
| `PORT` (bridge.mjs) | `8090` | bridge HTTP/WebSocket port |

---

## Production deployment (all-in-one docker-compose)

The host-native flow above (userns + `run-stack.sh`) is what's verified end-to-end
and is the recommended way to run this, because the FUSE mounts must be visible to
the agent's tools in the **same** namespace as the gateway. The Compose templates
below are a convenience for standing up the **database + gateway** together; they
are examples you save yourself — the repo only ships `docker-compose.dev.yml`.

> **FUSE caveat:** the `secafs` binary must be on the gateway container's `PATH`
> for the plugin to mount sessions, so you'd need to bake the compiled `secafs`
> into the OpenClaw image (or bind-mount it in) and give the container the
> privileges FUSE needs. For most setups running the gateway + daemon on the host
> is simpler and is the supported path.

### PostgreSQL

Save as `docker-compose.full.yml` in the directory **above** `openclaw/` and
`secafs/`, then `OPENCLAW_GATEWAY_TOKEN=$(openssl rand -hex 32) docker compose -f docker-compose.full.yml up -d`:

```yaml
# docker-compose.full.yml
services:
  db:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: secafs
      POSTGRES_PASSWORD: secafs
      POSTGRES_DB: secafs
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "secafs"]
      interval: 3s
      timeout: 3s
      retries: 10

  openclaw:
    image: ${OPENCLAW_IMAGE:-openclaw:local}
    environment:
      HOME: /home/node
      OPENCLAW_GATEWAY_TOKEN: ${OPENCLAW_GATEWAY_TOKEN:?set OPENCLAW_GATEWAY_TOKEN}
      OPENCLAW_GATEWAY_BIND: lan
      TZ: ${TZ:-UTC}
    volumes:
      - ${OPENCLAW_CONFIG_DIR:-~/.openclaw}:/home/node/.openclaw
      - ${OPENCLAW_WORKSPACE_DIR:-~/.openclaw/workspace}:/home/node/.openclaw/workspace
    ports:
      - "18789:18789"
      - "18790:18790"
    command: ["node", "dist/index.js", "gateway", "--bind", "lan", "--port", "18789"]
    healthcheck:
      test: ["CMD", "node", "-e", "fetch('http://127.0.0.1:18789/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"]
      interval: 30s
      timeout: 5s
      retries: 5
      start_period: 20s
```

### openGauss

Same idea with the openGauss image. Save as `docker-compose.full-opengauss.yml`:

```yaml
# docker-compose.full-opengauss.yml
services:
  db:
    image: enmotech/opengauss:6.0.0
    privileged: true
    environment:
      GS_USERNAME: secafs
      GS_PASSWORD: "Secafs!123"
    volumes:
      - ./secafs/scripts/opengauss-init.sql:/docker-entrypoint-initdb.d/init.sql:ro
    healthcheck:
      test: ["CMD-SHELL", "export LD_LIBRARY_PATH=/usr/local/opengauss/lib && /usr/local/opengauss/bin/gsql -d postgres -p 5432 -U secafs -W Secafs!123 -c 'SELECT 1' > /dev/null 2>&1 || exit 1"]
      interval: 5s
      timeout: 10s
      retries: 15

  openclaw:
    image: ${OPENCLAW_IMAGE:-openclaw:local}
    environment:
      HOME: /home/node
      OPENCLAW_GATEWAY_TOKEN: ${OPENCLAW_GATEWAY_TOKEN:?set OPENCLAW_GATEWAY_TOKEN}
      OPENCLAW_GATEWAY_BIND: lan
      TZ: ${TZ:-UTC}
    volumes:
      - ${OPENCLAW_CONFIG_DIR:-~/.openclaw}:/home/node/.openclaw
      - ${OPENCLAW_WORKSPACE_DIR:-~/.openclaw/workspace}:/home/node/.openclaw/workspace
    ports:
      - "18789:18789"
      - "18790:18790"
    command: ["node", "dist/index.js", "gateway", "--bind", "lan", "--port", "18789"]
```

When the plugin connects to openGauss via `postgresUrl`, URL-encode the `!` in
the password as `%21` (e.g. `postgres://secafs:Secafs%21123@db:5432/secafs`).
