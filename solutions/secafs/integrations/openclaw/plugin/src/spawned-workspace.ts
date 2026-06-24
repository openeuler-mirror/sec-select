/**
 * Path C cwd redirect (upstream-only mechanism).
 *
 * Replaces the fork-local `SessionWorkspaceApi`
 * (`openclaw/plugin-sdk/session-workspace`), which does NOT exist in upstream
 * OpenClaw. Instead of writing a fork-only `workspace.path` override, this
 * writes the upstream-honored spawn-lineage fields onto the session entry.
 * For each agent run OpenClaw resolves (see `gateway/server-methods/agent.ts`,
 * gated on `spawnedBy` being truthy):
 *   - the process working directory `cwd`  ← `sessionEntry.spawnedCwd`
 *     (`resolveSessionRuntimeCwd`) — this is what bash/exec tools run in;
 *   - the agent `workspaceDir`             ← `sessionEntry.spawnedWorkspaceDir`
 *     (`resolveIngressWorkspaceOverrideForSpawnedRun`).
 * So to land the agent INSIDE the FUSE mount we must set BOTH `spawnedCwd` and
 * `spawnedWorkspaceDir` to the mount path — `spawnedWorkspaceDir` alone leaves
 * the real cwd untouched.
 *
 * Exposes the same `{ set, clear }` shape the rest of the plugin already wires,
 * so gateway-methods and the auto-mount hook are unchanged.
 *
 * - `set(key, { path })` enables the redirect: `spawnedCwd = spawnedWorkspaceDir
 *   = path`, `spawnedBy = pluginId`.
 * - `clear(key)` disables it by unsetting `spawnedBy`. `spawnedCwd` /
 *   `spawnedWorkspaceDir` are left intact on purpose — upstream treats them as
 *   write-once and the resolvers ignore them whenever `spawnedBy` is falsy.
 *
 * NOTE (pending live spike): writing these fields directly to the session store
 * bypasses the gateway `sessions.patch` validation that restricts the spawn
 * fields to `subagent:*` / `acp:*` keys, so this works for the existing
 * `main:*` / `agent:*` key forms. The spike must confirm that setting
 * `spawnedBy` on such sessions has no unwanted side effects (e.g. session
 * visibility/grouping); if it does, move SecAFS conversations onto `acp:*`
 * sessions driven by the standalone frontend.
 */
export interface SpawnedWorkspaceStore {
  patch(key: string, patch: Record<string, unknown>): Promise<void>;
}

export interface SpawnedWorkspaceRedirect {
  set(key: string, spec: { path: string }): Promise<void>;
  clear(key: string): Promise<void>;
}

export function createSpawnedWorkspaceRedirect(
  store: SpawnedWorkspaceStore,
  pluginId: string,
): SpawnedWorkspaceRedirect {
  return {
    async set(key, spec) {
      await store.patch(key, {
        spawnedCwd: spec.path,
        spawnedWorkspaceDir: spec.path,
        spawnedBy: pluginId,
      });
    },
    async clear(key) {
      await store.patch(key, { spawnedBy: undefined });
    },
  };
}
