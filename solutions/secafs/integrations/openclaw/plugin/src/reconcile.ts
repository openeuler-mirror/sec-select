import type { SecafsRollbackState } from "./rollback-types.js";

export interface ReconcileDeps {
  rpc: {
    list(): Promise<{ mounts: Array<{ conversationId: string; hostPath: string; since: string }> }>;
    unmount(p: { conversationId: string }): Promise<{ unmounted: boolean }>;
    mount(p: {
      conversationId: string;
      hostPath?: string;
    }): Promise<{ hostPath: string; mounted: boolean }>;
    snapshotRestore(p: { conversationId: string; snapId: number }): Promise<{
      restored: boolean;
      prunedSnapshots: number;
      prunedUndoRows: number;
    }>;
  };
  sessions: {
    load(key: string): Promise<unknown>;
    keys(): Promise<string[]>;
    patch(key: string, p: Record<string, unknown>): Promise<void>;
  };
  sessionKeyFor(conversationId: string): string;
  truncateJsonlAfterMessage(
    file: string,
    messageId: string,
    opts?: { missingOk?: boolean },
  ): Promise<{ truncated: boolean }>;
  sessionFileFor(sessionKey: string): string;
  trajectoryFor(sessionKey: string): string;
  workspaceFor(sessionKey: string): string;
  extractConvId(sessionKey: string): string;
  logger?: {
    info?: (msg: string) => void;
    warn?: (msg: string) => void;
    error?: (msg: string) => void;
  };
}

/**
 * On plugin startup, ask the daemon what mounts it has and unmount any
 * that no longer correspond to a live session record (orphans from crashes
 * or out-of-band session deletions).
 *
 * Also resumes any in-progress rollbacks idempotently, picking up from
 * whatever phase was last completed.
 */
export async function reconcile(deps: ReconcileDeps): Promise<{
  unmounted: number;
  rollbacksResumed: number;
}> {
  let unmounted = 0;
  let rollbacksResumed = 0;

  // Existing orphan unmount path.
  let listResult;
  try {
    listResult = await deps.rpc.list();
  } catch (e) {
    deps.logger?.warn?.(`[secafs-chat] reconcile: rpc.list failed: ${String(e)}`);
    return { unmounted: 0, rollbacksResumed: 0 };
  }
  for (const m of listResult.mounts) {
    const key = deps.sessionKeyFor(m.conversationId);
    let rec;
    try {
      rec = await deps.sessions.load(key);
    } catch (e) {
      deps.logger?.warn?.(`[secafs-chat] reconcile: sessions.load failed for ${key}: ${String(e)}`);
      continue;
    }
    if (!rec) {
      try {
        await deps.rpc.unmount({ conversationId: m.conversationId });
        unmounted += 1;
        deps.logger?.info?.(`[secafs-chat] reconcile: unmounted orphan ${m.conversationId}`);
      } catch (e) {
        deps.logger?.warn?.(
          `[secafs-chat] reconcile: unmount failed for ${m.conversationId}: ${String(e)}`,
        );
      }
    }
  }

  // New: resume any in-progress rollbacks.
  let keys: string[] = [];
  try {
    keys = await deps.sessions.keys();
  } catch (e) {
    deps.logger?.warn?.(`[secafs-chat] reconcile: sessions.keys failed: ${String(e)}`);
    return { unmounted, rollbacksResumed };
  }

  for (const key of keys) {
    let entry;
    try {
      entry = (await deps.sessions.load(key)) as { secafsRollback?: SecafsRollbackState } | null;
    } catch {
      continue;
    }
    const rb = entry?.secafsRollback;
    const ip = rb?.inProgress;
    if (!ip || !rb) {
      continue;
    }

    deps.logger?.info?.(
      `[secafs-chat] reconcile: resuming rollback for ${key} → snap ${ip.targetSnapId}`,
    );
    try {
      const conversationId = deps.extractConvId(key);
      let phase = ip;

      if (!phase.fsRestored) {
        try {
          await deps.rpc.unmount({ conversationId });
        } catch {
          // mount may not exist; safe to ignore
        }
        await deps.rpc.snapshotRestore({ conversationId, snapId: phase.targetSnapId });
        phase = { ...phase, fsRestored: true };
        await deps.sessions.patch(key, {
          secafsRollback: { ...rb, inProgress: phase },
        });
      }

      if (!phase.jsonlTruncated) {
        await deps.truncateJsonlAfterMessage(deps.sessionFileFor(key), phase.targetMessageId, {
          missingOk: true,
        });
        await deps.truncateJsonlAfterMessage(deps.trajectoryFor(key), phase.targetMessageId, {
          missingOk: true,
        });
        phase = { ...phase, jsonlTruncated: true };
        await deps.sessions.patch(key, {
          secafsRollback: { ...rb, inProgress: phase },
        });
      }

      try {
        await deps.rpc.mount({ conversationId, hostPath: deps.workspaceFor(key) });
      } catch (e) {
        deps.logger?.warn?.(`[secafs-chat] reconcile: remount failed for ${key}: ${String(e)}`);
      }

      await deps.sessions.patch(key, {
        secafsRollback: {
          enabled: true,
          lastSnapshotMessageId: phase.targetMessageId,
        },
      });
      rollbacksResumed += 1;
      deps.logger?.info?.(`[secafs-chat] reconcile: rollback for ${key} finished`);
    } catch (e) {
      deps.logger?.error?.(
        `[secafs-chat] reconcile: rollback resume failed for ${key}: ${String(e)}`,
      );
    }
  }

  return { unmounted, rollbacksResumed };
}
