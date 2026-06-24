import path from "node:path";

/**
 * Minimal projection of SessionEntry fields the auto-mount hook reads. The
 * full type lives in core; we keep a local shape here so the plugin source
 * stays decoupled from core's deep-type tree.
 */
export interface SessionEntryView {
  sessionId?: string;
  updatedAt?: number;
  kind?: "secafs";
  mountState?: "mounted" | "unmounted" | "stuck";
  workspace?: { path?: string; managedBy?: string };
}

export interface AutoMountHookDeps {
  /** Returns the current session-store snapshot. */
  loadStore: () => Record<string, SessionEntryView>;
  /** Patch a single session entry (mountState updates). */
  patchSession: (key: string, patch: Record<string, unknown>) => Promise<void>;
  /** Path C cwd redirect — re-establish spawnedWorkspaceDir+spawnedBy on remount. */
  workspace: {
    set: (key: string, spec: { path: string }) => Promise<void>;
  };
  /** Daemon RPC for mounting. */
  rpc: {
    mount: (p: {
      conversationId: string;
      hostPath?: string;
    }) => Promise<{ hostPath: string; mounted: boolean }>;
  };
  mountRoot: string;
  logger?: { info?: (m: string) => void; warn?: (m: string) => void };
}

/**
 * Ensure a SecAFS session's FUSE volume is mounted before the agent reads
 * workspace files for a turn. Idempotent and best-effort: mount errors are
 * logged but do not abort the run (the agent will see whatever state the
 * workspace is currently in and fail naturally if that's broken).
 *
 * Designed to be wired to the `before_prompt_build` hook so the prompt
 * builder reads from the live FUSE mount, not a phantom directory left
 * behind by an idle-unmount.
 */
export async function ensureSecafsMountForSession(
  deps: AutoMountHookDeps,
  sessionKey: string | undefined,
): Promise<void> {
  const key = sessionKey?.trim();
  if (!key) {
    return;
  }
  const store = deps.loadStore();
  const { entry, sourceKey } = resolveSecafsEntry(store, key);
  if (!entry || entry.kind !== "secafs") {
    return;
  }
  if (entry.mountState === "mounted") {
    return;
  }
  const sid = sidFromKey(sourceKey);
  if (!sid) {
    return;
  }
  const hostPath = path.join(deps.mountRoot, sid);
  try {
    const result = await deps.rpc.mount({ conversationId: sid, hostPath });
    await deps.workspace.set(sourceKey, { path: result.hostPath });
    await deps.patchSession(sourceKey, { mountState: "mounted" });
    deps.logger?.info?.(`[secafs-chat] auto-mounted ${sourceKey} → ${result.hostPath}`);
  } catch (err) {
    deps.logger?.warn?.(
      `[secafs-chat] auto-mount failed for ${sourceKey}: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
  }
}

/**
 * Resolve the SecAFS session entry for a key passed by the agent runtime.
 * The runtime references conversations via the agent-canonical form
 * `agent:<id>:main:<uuid>`, but the SecAFS metadata (kind, workspace) lives
 * on the bare `main:<uuid>` entry. We return whichever side carries the
 * `kind: "secafs"` marker so callers patch the correct entry.
 */
function resolveSecafsEntry(
  store: Record<string, SessionEntryView>,
  sessionKey: string,
): { entry: SessionEntryView | undefined; sourceKey: string } {
  const direct = store[sessionKey];
  if (direct?.kind === "secafs") {
    return { entry: direct, sourceKey: sessionKey };
  }
  if (sessionKey.startsWith("agent:")) {
    const parts = sessionKey.split(":").filter((p) => p.length > 0);
    if (parts.length >= 3) {
      const bareKey = parts.slice(2).join(":");
      const bare = store[bareKey];
      if (bare?.kind === "secafs") {
        return { entry: bare, sourceKey: bareKey };
      }
    }
  }
  return { entry: direct, sourceKey: sessionKey };
}

function sidFromKey(sessionKey: string): string {
  const parts = sessionKey.split(":").filter((p) => p.length > 0);
  return parts.length >= 2 ? parts[parts.length - 1] : sessionKey;
}
