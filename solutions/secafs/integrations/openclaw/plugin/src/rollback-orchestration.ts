import type { SecafsRollbackState } from "./rollback-types.js";

export interface HandleRestoreDeps {
  rpc: {
    unmount(p: { conversationId: string }): Promise<{ unmounted: boolean }>;
    mount(p: {
      conversationId: string;
      hostPath?: string;
    }): Promise<{ hostPath: string; mounted: boolean }>;
    snapshotList(p: { conversationId: string }): Promise<{
      snapshots: Array<{ snapId: number; label: string | null; committedAt: string }>;
    }>;
    snapshotRestore(p: { conversationId: string; snapId: number }): Promise<{
      restored: boolean;
      prunedSnapshots: number;
      prunedUndoRows: number;
    }>;
  };
  sessionStore: {
    load(key: string): Promise<{ secafsRollback?: SecafsRollbackState } | null>;
    patch(key: string, patch: Record<string, unknown>): Promise<void>;
  };
  truncate: (
    file: string,
    messageId: string,
    opts?: { missingOk?: boolean; idNotFoundOk?: boolean },
  ) => Promise<{ truncated: boolean }>;
  events: (event: { type: string; sessionKey: string; [k: string]: unknown }) => void;
  sessionFileFor: (sessionKey: string) => string;
  trajectoryFor: (sessionKey: string) => string;
  workspaceFor: (sessionKey: string) => string;
  extractConvId: (sessionKey: string) => string;
  logger?: { info?: (msg: string) => void; warn?: (msg: string) => void };
}

export interface HandleRestoreArgs {
  sessionKey: string;
  snapId: number;
}
export interface HandleRestoreResult {
  restored: true;
  restoredMessageId: string;
  prunedSnapshots: number;
  prunedUndoRows: number;
}

export function createHandleRestore(deps: HandleRestoreDeps) {
  return async function handleRestore(args: HandleRestoreArgs): Promise<HandleRestoreResult> {
    const { sessionKey, snapId } = args;
    const conversationId = deps.extractConvId(sessionKey);

    const entry = await deps.sessionStore.load(sessionKey);
    const rb: SecafsRollbackState = entry?.secafsRollback ?? { enabled: false };
    if (rb.inProgress) {
      const e: Error & { code?: string } = new Error("rollback already in progress");
      e.code = "RESTORE_IN_PROGRESS";
      throw e;
    }

    const list = await deps.rpc.snapshotList({ conversationId });
    const target = list.snapshots.find((s) => s.snapId === snapId);
    if (!target) {
      throw new Error(`snapshot ${snapId} not found`);
    }
    const targetMessageId = target.label;
    if (!targetMessageId) {
      throw new Error(`snapshot ${snapId} has no label`);
    }

    // Phase 1: mark inProgress.
    const inProgress = {
      targetSnapId: snapId,
      targetMessageId,
      fsRestored: false,
      jsonlTruncated: false,
      startedAt: Date.now(),
    };
    await deps.sessionStore.patch(sessionKey, {
      secafsRollback: { ...rb, inProgress },
    });

    // Phase 2: unmount.
    await deps.rpc.unmount({ conversationId });

    // Phase 3: daemon restore.
    const outcome = await deps.rpc.snapshotRestore({ conversationId, snapId });
    await deps.sessionStore.patch(sessionKey, {
      secafsRollback: { ...rb, inProgress: { ...inProgress, fsRestored: true } },
    });

    // Phase 4: truncate JSONL + trajectory (atomic via tempfile + rename inside the helper).
    await deps.truncate(deps.sessionFileFor(sessionKey), targetMessageId);
    // Trajectory is an auxiliary trace with its own id space (seq-based), so chat
    // message ids will not appear in it — tolerate id-not-found as a no-op.
    await deps.truncate(deps.trajectoryFor(sessionKey), targetMessageId, {
      missingOk: true,
      idNotFoundOk: true,
    });
    await deps.sessionStore.patch(sessionKey, {
      secafsRollback: {
        ...rb,
        inProgress: { ...inProgress, fsRestored: true, jsonlTruncated: true },
      },
    });

    // Phase 5: remount.
    await deps.rpc.mount({ conversationId, hostPath: deps.workspaceFor(sessionKey) });

    // Phase 6: clear marker, finalize.
    await deps.sessionStore.patch(sessionKey, {
      secafsRollback: { enabled: true, lastSnapshotMessageId: targetMessageId },
    });

    deps.events({
      type: "secafs.rollback.completed",
      sessionKey,
      restoredSnapId: snapId,
      restoredMessageId: targetMessageId,
      prunedSnapshots: outcome.prunedSnapshots,
      prunedUndoRows: outcome.prunedUndoRows,
    });

    return {
      restored: true,
      restoredMessageId: targetMessageId,
      prunedSnapshots: outcome.prunedSnapshots,
      prunedUndoRows: outcome.prunedUndoRows,
    };
  };
}
