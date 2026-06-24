# SecAFS ↔ OpenClaw Integration

This directory makes **SecAFS own its OpenClaw integration**, so the OpenClaw
checkout can stay **pristine upstream** (`v2026.6.8-alpha.1`, 0 local changes)
and OpenClaw upgrades never conflict with SecAFS.

## Target architecture (gateway-client form)

```
┌──────────────────────────────┐
│  SecAFS-first frontend (SPA)  │  standalone single-file gateway client
│  primary surface = the FS     │
└───────────────┬──────────────┘
                │ Gateway WebSocket protocol (operator / ACP role, token auth)
                │ calls chat.* + secafs.* + sessions.patch
┌───────────────▼──────────────┐
│  OpenClaw Gateway (PRISTINE)  │  upstream v2026.6.8-alpha.1, 0 changes
│  loads secafs-chat as an      │
│  EXTERNAL third-party plugin  │
└───────────────┬──────────────┘
                │ Unix socket JSON-RPC
┌───────────────▼──────────────┐
│  secafs serve api (Rust)      │  ../../cli — independent component
│  PostgreSQL backend           │
└──────────────────────────────┘
```

The frontend is a **gateway client**, not an OpenClaw "channel" (channels are
backend messaging adapters, not UIs).

## The key that makes OpenClaw 0-change: Path C (cwd redirect via upstream seam)

SecAFS needs the agent's working directory to point into the per-conversation
FUSE mount. Path C achieves this using **only upstream seams**, so no OpenClaw
core change is required.

The plugin writes `spawnedCwd` **+** `spawnedWorkspaceDir` **+** `spawnedBy` to
**all** key forms of the session entry (bare `main:<uuid>` and canonical
`agent:<id>:main:<uuid>` — the first run migrates bare→canonical without merging
fields, so a single-form write would be lost). The agent run then honors
`spawnedCwd` for the bash/file cwd and `spawnedWorkspaceDir`+`spawnedBy` for the
workspace override.

**Mechanism (upstream-only — no core change):**

1. Create each SecAFS conversation as a `main:<uuid>` session.
2. Patch the cwd/workspace fields onto the session over the gateway:
   ```
   gateway.call("sessions.patch", {
     sessionKey: "<sessionKey>",
     patch: {
       spawnedBy: "secafs",
       spawnedCwd: "<FUSE mount path>",
       spawnedWorkspaceDir: "<FUSE mount path>",
     },
   })
   ```
3. The agent run honors it via upstream
   `resolveSessionRuntimeWorkspace` → `resolveIngressWorkspaceOverrideForSpawnedRun`
   (gated on `spawnedBy` being truthy) for the workspace, and
   `resolveSessionRuntimeCwd` (read unconditionally) for the cwd. Bash/file tools
   then run with `cwd = <FUSE mount path>`.

All pieces are present in upstream `v2026.6.8-alpha.1`
(`src/gateway/sessions-patch.ts` handles `spawnedWorkspaceDir`;
`src/gateway/server-methods/agent.ts` `resolveSessionRuntimeWorkspace`).

**Constraints:**
- `spawnedWorkspaceDir` is only allowed on `subagent:*` / `acp:*` session keys.
- It is **write-once** (cannot be cleared once set) → SecAFS per-conversation
  mount path must be deterministic & stable (`mountRoot/<conversationId>`).
- `spawnedBy` must be set (also patchable).

Net result: agent bash/file tools run with cwd inside the per-conversation FUSE
mount, and files persist to the SecAFS backend. See [`README.md`](./README.md)
to reproduce.

## What is in here

| Subdir | Role |
|---|---|
| `plugin/` | The OpenClaw plugin (daemon supervision + `secafs.*` gateway methods + mount/rollback logic). An **external** plugin. |
| `frontend/` | Standalone single-file SecAFS-first gateway client. |
| `bridge/` | Frontend↔gateway proxy and local stack runner. |
| `docs/` | Rollback plugin doc. |

SecAFS depends only on two stable public contracts — the gateway WebSocket
protocol (incl. `sessions.patch`) and `@openclaw/plugin-sdk` — so there is **0
secafs-specific code in the OpenClaw checkout**.

## Running it end-to-end

See [`README.md`](./README.md) and [`RUNBOOK.md`](./RUNBOOK.md) for setup and the
full bring-up.

