import { describe, expect, it, vi } from "vitest";
import { createHandleRestore } from "./rollback-orchestration.js";

function makeFakes() {
  const rpc = {
    snapshotList: vi.fn(async (_p) => ({
      snapshots: [{ snapId: 5, label: "msg-5", committedAt: "now" }],
    })),
    snapshotRestore: vi.fn(async (_p) => ({
      restored: true,
      prunedSnapshots: 1,
      prunedUndoRows: 3,
    })),
    unmount: vi.fn(async (_p) => ({ unmounted: true })),
    mount: vi.fn(async (_p) => ({ hostPath: "/mnt", mounted: true })),
  };
  const sessionStore = {
    load: vi.fn(async (_k) => ({ secafsRollback: { enabled: true } })),
    patch: vi.fn(async (_k, _p) => undefined),
  };
  const truncate = vi.fn(async (_file, _id) => ({
    fileExisted: true,
    truncated: true,
    bytesAfter: 100,
  }));
  const events = vi.fn();
  return { rpc, sessionStore, truncate, events };
}

describe("handleRestore", () => {
  it("runs all phases in order and emits completed event", async () => {
    const { rpc, sessionStore, truncate, events } = makeFakes();
    const handle = createHandleRestore({
      rpc,
      sessionStore,
      truncate,
      events,
      sessionFileFor: () => "/p/s.jsonl",
      trajectoryFor: () => "/p/s.trajectory.jsonl",
      workspaceFor: () => "/mnt",
      extractConvId: (k) => k.split(":")[1] ?? k,
    });
    const r = await handle({ sessionKey: "main:abc", snapId: 5 });
    expect(r.restored).toBe(true);
    expect(r.restoredMessageId).toBe("msg-5");
    expect(rpc.unmount).toHaveBeenCalled();
    expect(rpc.snapshotRestore).toHaveBeenCalled();
    expect(truncate).toHaveBeenCalledTimes(2); // jsonl + trajectory
    // Trajectory uses a different id space; truncate must be lenient.
    expect(truncate.mock.calls[1][2]).toEqual(
      expect.objectContaining({ missingOk: true, idNotFoundOk: true }),
    );
    expect(rpc.mount).toHaveBeenCalled();
    expect(events).toHaveBeenCalledWith(
      expect.objectContaining({ type: "secafs.rollback.completed" }),
    );
  });

  it("rejects when another restore is already in progress", async () => {
    const fakes = makeFakes();
    fakes.sessionStore.load = vi.fn(async () => ({
      secafsRollback: {
        enabled: true,
        inProgress: {
          targetSnapId: 9,
          targetMessageId: "x",
          fsRestored: false,
          jsonlTruncated: false,
          startedAt: 0,
        },
      },
    }));
    const handle = createHandleRestore({
      ...fakes,
      sessionFileFor: () => "x",
      trajectoryFor: () => "x",
      workspaceFor: () => "x",
      extractConvId: () => "v",
    });
    await expect(handle({ sessionKey: "k", snapId: 5 })).rejects.toThrow(/in progress/i);
  });

  it("throws when snapId is unknown", async () => {
    const fakes = makeFakes();
    fakes.rpc.snapshotList = vi.fn(async () => ({ snapshots: [] }));
    const handle = createHandleRestore({
      ...fakes,
      sessionFileFor: () => "x",
      trajectoryFor: () => "x",
      workspaceFor: () => "x",
      extractConvId: () => "v",
    });
    await expect(handle({ sessionKey: "k", snapId: 99 })).rejects.toThrow(/not found/);
  });
});
