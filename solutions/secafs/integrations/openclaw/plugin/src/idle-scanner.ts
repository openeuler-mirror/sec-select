import path from "node:path";
import type { SecafsRollbackState } from "./rollback-types.js";

/**
 * Minimal projection of SessionEntry fields the scanner reads. Keeping the
 * type local avoids deep-importing core types into the plugin.
 */
export interface SessionEntryView {
  sessionId?: string;
  updatedAt?: number;
  kind?: "secafs";
  mountState?: "mounted" | "unmounted" | "stuck";
  secafsRollback?: SecafsRollbackState;
}

export interface IdleScannerDeps {
  /** Returns the current session-store snapshot. */
  loadStore: () => Record<string, SessionEntryView>;
  /** Patch a single session entry (used to set mountState=unmounted). */
  patchSession: (key: string, patch: Record<string, unknown>) => Promise<void>;
  /** Daemon RPC for the mount-keeper and idle-unmount duties. */
  rpc: {
    list: () => Promise<{ mounts: Array<{ conversationId: string }> }>;
    mount: (p: {
      conversationId: string;
      hostPath?: string;
    }) => Promise<{ hostPath: string; mounted: boolean }>;
    unmount: (p: { conversationId: string }) => Promise<{ unmounted: boolean }>;
  };
  /** Path C cwd redirect — re-establish spawned* fields on remount. */
  workspace: {
    set: (key: string, spec: { path: string }) => Promise<void>;
  };
  mountRoot: string;
  /** Mount liveness probe (overridable for tests). Defaults to a st_dev
   *  comparison between the mountpoint and the mount root. */
  probeMount?: (sid: string) => Promise<boolean | null>;
  /** Time source (overridable for tests). */
  now?: () => number;
  logger?: { info?: (m: string) => void; warn?: (m: string) => void };
}

export interface IdleScannerOptions {
  /**
   * Seconds since `updatedAt` after which a mounted volume is unmounted.
   * `0` disables the idle-unmount duty (the keeper still runs).
   */
  idleUnmountSeconds: number;
  /** Seconds between scan ticks. `0` disables the scanner entirely. */
  idleScanSeconds: number;
}

/**
 * Periodic volume scanner with two duties:
 *
 * 1. MOUNT-KEEPER (always on): remount sessions whose store entry says
 *    `mountState: "mounted"` but whose volume the daemon does not actually
 *    have — the daemon dying (crash + supervisor respawn) takes every FUSE
 *    mount with it while the store still says mounted. Without the keeper,
 *    the next direct chat to such a session aborts with upstream's
 *    WorkspaceVanishedError (its attestation check runs BEFORE plugin hooks,
 *    so the before_prompt_build auto-mount never gets a chance).
 *
 * 2. IDLE-UNMOUNT (opt-in via idleUnmountSeconds > 0): unmount volumes idle
 *    longer than the threshold. NOTE this re-creates the vanished-workspace
 *    trap for directly-addressed sessions (see above) — the lazy remount via
 *    before_prompt_build fires too late on the embedded-run path. Only
 *    enable it for deployments that reopen sessions explicitly.
 *
 * Returns a `stop()` handle that clears the interval.
 */
export function startIdleScanner(
  deps: IdleScannerDeps,
  opts: IdleScannerOptions,
): { stop: () => void } {
  if (opts.idleScanSeconds <= 0) {
    return { stop: () => {} };
  }
  const idleMs = opts.idleUnmountSeconds * 1000;
  const intervalMs = opts.idleScanSeconds * 1000;
  const now = deps.now ?? (() => Date.now());
  const handle = setInterval(() => {
    void scanOnce(deps, idleMs, now);
  }, intervalMs);
  // Don't keep the process alive just for this scanner.
  if (typeof handle === "object" && handle !== null && "unref" in handle) {
    (handle as unknown as { unref: () => void }).unref();
  }
  return {
    stop: () => clearInterval(handle),
  };
}

/**
 * Single-pass scan. Exposed for tests. `idleMs <= 0` runs only the
 * mount-keeper duty.
 */
export async function scanOnce(
  deps: IdleScannerDeps,
  idleMs: number,
  now: () => number,
): Promise<void> {
  let store: Record<string, SessionEntryView>;
  try {
    store = deps.loadStore();
  } catch (err) {
    deps.logger?.warn?.(
      `[secafs-chat] idle scanner: failed to load session store: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
    return;
  }

  // Duty 1: mount-keeper — remount store-mounted sessions the daemon lost.
  let daemonMounts: Set<string> | null = null;
  try {
    const r = await deps.rpc.list();
    daemonMounts = new Set(r.mounts.map((m) => m.conversationId));
  } catch (err) {
    deps.logger?.warn?.(
      `[secafs-chat] mount-keeper: rpc.list failed: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
  }
  if (daemonMounts) {
    // A conversation can appear under several key forms; remount each sid once.
    const lost = new Map<string, string>();
    for (const [key, entry] of Object.entries(store)) {
      if (entry?.kind !== "secafs" || entry.mountState !== "mounted") {
        continue;
      }
      if (entry.secafsRollback?.inProgress) {
        continue;
      }
      const sid = sidFromKey(key);
      if (!sid || lost.has(sid)) {
        continue;
      }
      if (!daemonMounts.has(sid)) {
        // daemon doesn't have it at all (crash + respawn)
        lost.set(sid, key);
        continue;
      }
      // Zombie probe: the daemon claims the mount but the kernel may have
      // lost it (manual umount, FUSE thread death → ENOTCONN). A live FUSE
      // mountpoint sits on a different st_dev than the mount root.
      const live = await (deps.probeMount ?? ((s2: string) => isLiveMount(deps, s2)))(sid);
      if (live === false) {
        lost.set(sid, key);
      }
    }
    for (const [sid, key] of lost) {
      try {
        // unmount first so the daemon drops a stale handle (no-op when absent)
        try {
          await deps.rpc.unmount({ conversationId: sid });
        } catch {
          /* not mounted daemon-side; continue */
        }
        const result = await deps.rpc.mount({
          conversationId: sid,
          hostPath: path.join(deps.mountRoot, sid),
        });
        await deps.workspace.set(key, { path: result.hostPath });
        deps.logger?.info?.(
          `[secafs-chat] mount-keeper: re-mounted ${key} → ${result.hostPath} (mount was lost)`,
        );
      } catch (err) {
        deps.logger?.warn?.(
          `[secafs-chat] mount-keeper: re-mount failed for ${key}: ${
            err instanceof Error ? err.message : String(err)
          }`,
        );
      }
    }
  }

  // Duty 2: idle unmount (opt-in).
  if (idleMs <= 0) {
    return;
  }
  const cutoff = now() - idleMs;
  for (const [key, entry] of Object.entries(store)) {
    if (entry?.kind !== "secafs") {
      continue;
    }
    if (entry.mountState !== "mounted") {
      continue;
    }
    if (entry.secafsRollback?.inProgress) {
      // Don't unmount a volume mid-rollback — restore orchestration owns the mount lifecycle.
      continue;
    }
    const updatedAt = typeof entry.updatedAt === "number" ? entry.updatedAt : 0;
    if (updatedAt > cutoff) {
      continue;
    }
    const sid = sidFromKey(key);
    if (!sid) {
      continue;
    }
    try {
      await deps.rpc.unmount({ conversationId: sid });
      await deps.patchSession(key, { mountState: "unmounted" });
      deps.logger?.info?.(`[secafs-chat] idle-unmounted ${key} (idle ≥ ${idleMs / 1000}s)`);
    } catch (err) {
      deps.logger?.warn?.(
        `[secafs-chat] idle-unmount failed for ${key}: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }
  }
}

/**
 * Probe whether the FUSE mount for `sid` is actually live in the kernel.
 * Returns true (live), false (dead/absent), or null (indeterminate — fail
 * open, do nothing). A mounted FUSE volume sits on a different st_dev than
 * the mount root; a dead FUSE session surfaces as ENOTCONN on stat.
 */
async function isLiveMount(deps: IdleScannerDeps, sid: string): Promise<boolean | null> {
  const fs = await import("node:fs/promises");
  const hostPath = path.join(deps.mountRoot, sid);
  try {
    const [mp, root] = await Promise.all([fs.stat(hostPath), fs.stat(deps.mountRoot)]);
    return mp.dev !== root.dev;
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    if (code === "ENOTCONN") return false; // zombie FUSE session
    if (code === "ENOENT") return false; // mountpoint dir gone entirely
    return null;
  }
}

/** Mirrors gateway-methods sidFromKey: last colon segment for both forms. */
function sidFromKey(sessionKey: string): string {
  const parts = sessionKey.split(":").filter((p) => p.length > 0);
  return parts.length >= 2 ? parts[parts.length - 1] : sessionKey;
}
