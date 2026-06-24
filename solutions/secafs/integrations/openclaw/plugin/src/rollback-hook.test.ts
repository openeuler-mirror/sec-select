import { describe, expect, it, vi } from "vitest";
import { createSnapshotOnTurnEnd } from "./rollback-hook.js";

describe("snapshot-on-turn-end hook", () => {
  it("commits a snapshot labeled with the last assistant message id", async () => {
    const rpc = {
      snapshotCommit: vi.fn(async () => ({ snapId: 3, committedAt: "t", label: "asst-99" })),
    };
    const sessionStore = {
      load: vi.fn(async () => ({ secafsRollback: { enabled: true } })),
      patch: vi.fn(async () => undefined),
    };
    const findLast = vi.fn(async () => "asst-99");
    const events = vi.fn();
    const hook = createSnapshotOnTurnEnd({
      rpc,
      sessionStore,
      findLastAssistantMessageId: findLast,
      events,
      sessionFileFor: () => "/p/s.jsonl",
      extractConvId: () => "vol",
    });
    await hook({ sessionKey: "main:vol" });
    expect(rpc.snapshotCommit).toHaveBeenCalledWith({ conversationId: "vol", label: "asst-99" });
    expect(events).toHaveBeenCalledWith(
      expect.objectContaining({ type: "secafs.rollback.snapshotCommitted", snapId: 3 }),
    );
  });

  it("skips when rollback is not enabled", async () => {
    const rpc = { snapshotCommit: vi.fn() };
    const sessionStore = {
      load: vi.fn(async () => ({ secafsRollback: { enabled: false } })),
      patch: vi.fn(),
    };
    const findLast = vi.fn(async () => "asst-99");
    const hook = createSnapshotOnTurnEnd({
      rpc,
      sessionStore,
      findLastAssistantMessageId: findLast,
      events: vi.fn(),
      sessionFileFor: () => "x",
      extractConvId: () => "v",
    });
    await hook({ sessionKey: "k" });
    expect(rpc.snapshotCommit).not.toHaveBeenCalled();
  });

  it("skips when last assistant message id is the same as last snapshot label", async () => {
    const rpc = { snapshotCommit: vi.fn() };
    const sessionStore = {
      load: vi.fn(async () => ({ secafsRollback: { enabled: true, lastSnapshotMessageId: "m1" } })),
      patch: vi.fn(),
    };
    const findLast = vi.fn(async () => "m1");
    const hook = createSnapshotOnTurnEnd({
      rpc,
      sessionStore,
      findLastAssistantMessageId: findLast,
      events: vi.fn(),
      sessionFileFor: () => "x",
      extractConvId: () => "v",
    });
    await hook({ sessionKey: "k" });
    expect(rpc.snapshotCommit).not.toHaveBeenCalled();
  });

  it("skips when no assistant message exists yet", async () => {
    const rpc = { snapshotCommit: vi.fn() };
    const sessionStore = {
      load: vi.fn(async () => ({ secafsRollback: { enabled: true } })),
      patch: vi.fn(),
    };
    const findLast = vi.fn(async () => null);
    const hook = createSnapshotOnTurnEnd({
      rpc,
      sessionStore,
      findLastAssistantMessageId: findLast,
      events: vi.fn(),
      sessionFileFor: () => "x",
      extractConvId: () => "v",
    });
    await hook({ sessionKey: "k" });
    expect(rpc.snapshotCommit).not.toHaveBeenCalled();
  });

  it("skips when restore is in progress", async () => {
    const rpc = { snapshotCommit: vi.fn() };
    const sessionStore = {
      load: vi.fn(async () => ({
        secafsRollback: {
          enabled: true,
          inProgress: {
            targetSnapId: 1,
            targetMessageId: "x",
            fsRestored: false,
            jsonlTruncated: false,
            startedAt: 0,
          },
        },
      })),
      patch: vi.fn(),
    };
    const findLast = vi.fn(async () => "asst-1");
    const hook = createSnapshotOnTurnEnd({
      rpc,
      sessionStore,
      findLastAssistantMessageId: findLast,
      events: vi.fn(),
      sessionFileFor: () => "x",
      extractConvId: () => "v",
    });
    await hook({ sessionKey: "k" });
    expect(rpc.snapshotCommit).not.toHaveBeenCalled();
  });
});

describe("findLastAssistantMessageId (real openclaw JSONL format)", () => {
  it("finds id from the openclaw nested {type:message, message:{role:assistant}} shape", async () => {
    const fs = await import("node:fs/promises");
    const path = await import("node:path");
    const os = await import("node:os");
    const { findLastAssistantMessageId } = await import("./rollback-hook.js");

    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "rollback-jsonl-"));
    const file = path.join(dir, "session.jsonl");
    const lines = [
      `{"type":"session","version":3,"id":"sess1","timestamp":"2026-01-01"}`,
      `{"type":"model_change","id":"m1","provider":"x"}`,
      `{"type":"message","id":"u-1","message":{"role":"user","content":[]}}`,
      `{"type":"message","id":"a-1","message":{"role":"assistant","content":[]}}`,
      `{"type":"message","id":"u-2","message":{"role":"user","content":[]}}`,
      `{"type":"message","id":"a-2","message":{"role":"assistant","content":[]}}`,
    ];
    await fs.writeFile(file, lines.join("\n") + "\n");
    const id = await findLastAssistantMessageId(file);
    expect(id).toBe("a-2");
    await fs.rm(dir, { recursive: true, force: true });
  });

  it("returns null when no assistant message exists yet (only user + meta events)", async () => {
    const fs = await import("node:fs/promises");
    const path = await import("node:path");
    const os = await import("node:os");
    const { findLastAssistantMessageId } = await import("./rollback-hook.js");

    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "rollback-jsonl-"));
    const file = path.join(dir, "session.jsonl");
    const lines = [
      `{"type":"session","version":3,"id":"sess1"}`,
      `{"type":"message","id":"u-1","message":{"role":"user","content":[]}}`,
    ];
    await fs.writeFile(file, lines.join("\n") + "\n");
    const id = await findLastAssistantMessageId(file);
    expect(id).toBeNull();
    await fs.rm(dir, { recursive: true, force: true });
  });
});
