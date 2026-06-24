import { createReadStream } from "node:fs";
import fs from "node:fs/promises";
import readline from "node:readline";
import type { SecafsRollbackState } from "./rollback-types.js";

export interface SnapshotOnTurnEndDeps {
  rpc: {
    snapshotCommit(p: { conversationId: string; label?: string }): Promise<{
      snapId: number;
      committedAt: string;
      label: string | null;
    }>;
  };
  sessionStore: {
    load(key: string): Promise<{ secafsRollback?: SecafsRollbackState } | null>;
    patch(key: string, patch: Record<string, unknown>): Promise<void>;
  };
  findLastAssistantMessageId: (sessionFile: string) => Promise<string | null>;
  events: (event: { type: string; sessionKey: string; [k: string]: unknown }) => void;
  sessionFileFor: (sessionKey: string) => string;
  extractConvId: (sessionKey: string) => string;
  logger?: { info?: (msg: string) => void; warn?: (msg: string) => void };
}

export function createSnapshotOnTurnEnd(deps: SnapshotOnTurnEndDeps) {
  return async function snapshotOnTurnEnd(ctx: { sessionKey?: string }): Promise<void> {
    if (!ctx.sessionKey) {
      return;
    }
    const entry = await deps.sessionStore.load(ctx.sessionKey);
    const rb: SecafsRollbackState = entry?.secafsRollback ?? { enabled: false };
    if (!rb.enabled) {
      return;
    }
    if (rb.inProgress) {
      return;
    }

    const lastMsgId = await deps.findLastAssistantMessageId(deps.sessionFileFor(ctx.sessionKey));
    if (!lastMsgId) {
      return;
    }
    if (rb.lastSnapshotMessageId === lastMsgId) {
      return;
    }

    try {
      const result = await deps.rpc.snapshotCommit({
        conversationId: deps.extractConvId(ctx.sessionKey),
        label: lastMsgId,
      });
      await deps.sessionStore.patch(ctx.sessionKey, {
        secafsRollback: { ...rb, lastSnapshotMessageId: lastMsgId },
      });
      deps.events({
        type: "secafs.rollback.snapshotCommitted",
        sessionKey: ctx.sessionKey,
        snapId: result.snapId,
        messageId: lastMsgId,
        committedAt: result.committedAt,
      });
    } catch (e) {
      deps.logger?.warn?.(
        `[secafs-chat] snapshot.commit failed for ${ctx.sessionKey}: ${String(e)}`,
      );
    }
  };
}

/**
 * Default findLastAssistantMessageId implementation.
 *
 * Streams the JSONL file looking for the last assistant message and returns
 * its id. Handles the openclaw chat JSONL schema where each event is shaped
 * `{"type":"message","id":"...","message":{"role":"assistant",...}}`. Also
 * accepts top-level `role`/`type` fallbacks so other producers continue to work.
 */
export async function findLastAssistantMessageId(file: string): Promise<string | null> {
  let stat;
  try {
    stat = await fs.stat(file);
  } catch {
    return null;
  }
  if (stat.size === 0) {
    return null;
  }
  const stream = createReadStream(file, { encoding: "utf8" });
  const rl = readline.createInterface({ input: stream, crlfDelay: Infinity });
  let lastId: string | null = null;
  for await (const line of rl) {
    if (!line) {
      continue;
    }
    let obj: {
      id?: unknown;
      role?: unknown;
      type?: unknown;
      message?: { role?: unknown };
    };
    try {
      obj = JSON.parse(line) as typeof obj;
    } catch {
      continue;
    }
    if (typeof obj.id !== "string") {
      continue;
    }
    const nestedRole =
      obj.message && typeof obj.message === "object" ? obj.message.role : undefined;
    const isAssistant =
      // openclaw chat schema: {"type":"message","message":{"role":"assistant"}}
      (obj.type === "message" && nestedRole === "assistant") ||
      // fallback shapes used by other producers / older formats
      obj.role === "assistant" ||
      obj.type === "assistant_message" ||
      obj.type === "assistant";
    if (isAssistant) {
      lastId = obj.id;
    }
  }
  return lastId;
}
