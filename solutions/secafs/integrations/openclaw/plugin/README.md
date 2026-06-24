# secafs-chat — OpenClaw plugin

An **external, third-party** OpenClaw plugin that gives every conversation its
own openGauss/PostgreSQL-backed, FUSE-mounted workspace, with the agent running
*inside* that mount. It imports only the public `openclaw/plugin-sdk/*` surface
and the gateway protocol, so the OpenClaw checkout stays pristine.

Full deploy guide: **[../README.md](../README.md)**.

## Build

```bash
npm install        # esbuild, vitest, types
npm run build      # → dist/index.js (bundled, ESM, packages external)
npm test           # 116 unit tests  (re-link node_modules/openclaw first — see below)
npm run typecheck  # tsc against the openclaw SDK types
```

> `npm install` overwrites the `node_modules/openclaw` symlink used by
> typecheck/tests. `npm run build` doesn't need it; before `npm test`/`typecheck`
> re-link: `ln -sfn ../../../../openclaw node_modules/openclaw`.

## Install into a gateway

Point the gateway config at this directory — do **not** copy files into the
OpenClaw tree, and do **not** use `openclaw plugins install` (it hangs on this
version before recording the install). In `~/.openclaw/openclaw.json`:

```json5
plugins: {
  load: { paths: ["<abs>/secafs/integrations/openclaw/plugin"] },
  entries: { "secafs-chat": { enabled: true, config: {
    manageDaemon: false,
    socketPath: "/home/<you>/.secafs/run/secafs.sock",
    mountRoot: "/home/<you>/.secafs/mounts",
  } } },
}
```

`openclaw.plugin.json` declares `activation.onStartup: true` — **required**, or
the gateway discovers the plugin but never calls `register()` and `secafs.*`
methods return "unknown method".

## Config (`plugins.entries.secafs-chat.config`)

| key | default | meaning |
|---|---|---|
| `socketPath` | `$XDG_RUNTIME_DIR/secafs/secafs.sock` | daemon Unix socket |
| `mountRoot` | `$XDG_STATE_HOME/secafs/mounts` | per-conversation mount parent |
| `manageDaemon` | `false` | gateway spawns `secafs serve api` itself (then `postgresUrl` is required) |
| `postgresUrl` | — | DSN, only when `manageDaemon: true` |
| `idleScanSeconds` | `2` | mount-keeper / idle-scan tick (also the recovery tick) |
| `idleUnmountSeconds` | `0` (off) | idle auto-unmount; off by default (avoids the vanished-workspace trap) |
| `enableRollbackUI` | `true` | register `secafs.rollback.*` and snapshot at turn boundaries |

## Gateway methods

- **Status:** `secafs.status`
- **Lifecycle:** `secafs.session.create` (alias) · `.open` · `.close` (unmount,
  keep data) · `.destroy` (delete volume + chat + transcripts) · `.list` · `.rename`
- **Files:** `secafs.tree` · `secafs.fs.read` · `secafs.fs.write`
- **Rollback:** `secafs.rollback.setEnabled` · `.list` · `.restore` · `.snapshot`
  (eager per-turn snapshot for the per-message ⏪ button)
- **Portability:** `secafs.session.export` · `secafs.session.import`
  (always a fresh copy; cross-instance capable)

## Requirements

- A reachable `secafs serve api` daemon (the `secafs` Rust binary on `PATH`;
  built from the sibling [SecAFS repo](../../../) `cli/` workspace).
- openGauss / PostgreSQL reachable by that daemon.
- Linux with `/dev/fuse`; in this setup mounts run inside a user namespace (no
  `sudo` / `fusermount3`) — see `../bridge/run-stack.sh`.

## How it works

- **Path C** anchors the agent's cwd to the FUSE mount using only upstream
  seams: the plugin writes `spawnedCwd` / `spawnedWorkspaceDir` / `spawnedBy` to
  the session entry (all key forms), so the agent run honors them — no OpenClaw
  core change. See [`../INTEGRATION.md`](../INTEGRATION.md).
- **Mount-keeper** (idle scanner): every tick it remounts store-mounted sessions
  the daemon lost (crash/respawn) or whose kernel mount went stale, keeping the
  `<5s` remount SLO and preventing `WorkspaceVanishedError`.
- **Rollback** rides SecAFS's copy-on-write snapshots + undo log; `restore`
  rolls back the volume *and* truncates the chat transcript to that point.
- **Session-store writes** go through upstream's `updateSessionStore` (the same
  writer queue the gateway uses) so plugin and gateway never race.
- All session state the plugin owns (`kind`, `mountState`, `alias`,
  `secafsRollback`) persists under `pluginExtensions["secafs-chat"]`, keeping the
  on-disk `SessionEntry` within the upstream-typed shape.
