/**
 * Per-session rollback state, persisted in sessions.json under
 * `secafsRollback`. Old sessions without this key are equivalent to
 * `{ enabled: false }`.
 */
export interface SecafsRollbackState {
  enabled: boolean;
  /** messageId of the most recent committed snapshot label (used to dedup in the snapshot hook). */
  lastSnapshotMessageId?: string;
  /** Two-phase commit marker for restore orchestration. Cleared on completion. */
  inProgress?: SecafsRollbackInProgress;
}

export interface SecafsRollbackInProgress {
  targetSnapId: number;
  targetMessageId: string;
  fsRestored: boolean;
  jsonlTruncated: boolean;
  startedAt: number;
}

export const ROLLBACK_KEY = "secafsRollback" as const;
