import { describe, expect, it, vi } from "vitest";
import { reconcile } from "./reconcile.js";

describe("reconcile in-progress rollback", () => {
  it("finishes a rollback whose fsRestored=true but jsonlTruncated=false", async () => {
    const session = {
      secafsRollback: {
        enabled: true,
        inProgress: {
          targetSnapId: 3,
          targetMessageId: "m3",
          fsRestored: true,
          jsonlTruncated: false,
          startedAt: 0,
        },
      },
    };
    const sessions = {
      load: vi.fn(async (k: string) => (k === "main:abc" ? session : null)),
      keys: vi.fn(async () => ["main:abc"]),
      patch: vi.fn(async () => undefined),
    };
    const rpc = {
      list: vi.fn(async () => ({ mounts: [] })),
      mount: vi.fn(async () => ({ hostPath: "/m", mounted: true })),
      unmount: vi.fn(async () => ({ unmounted: true })),
      snapshotRestore: vi.fn(async () => ({
        restored: true,
        prunedSnapshots: 0,
        prunedUndoRows: 0,
      })),
    };
    const truncate = vi.fn(async () => ({ truncated: true }));
    const result = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id: string) => `main:${id}`,
      truncateJsonlAfterMessage: truncate,
      sessionFileFor: () => "/p/s.jsonl",
      trajectoryFor: () => "/p/s.tj.jsonl",
      workspaceFor: () => "/m",
      extractConvId: (k: string) => k.split(":")[1] ?? k,
    });
    // fsRestored already true → snapshotRestore should NOT be called.
    expect(rpc.snapshotRestore).not.toHaveBeenCalled();
    // jsonlTruncated false → truncate called twice (jsonl + trajectory).
    expect(truncate).toHaveBeenCalledTimes(2);
    // After resume: mount called.
    expect(rpc.mount).toHaveBeenCalled();
    expect(result.rollbacksResumed).toBe(1);
  });

  it("does nothing for sessions without inProgress", async () => {
    const sessions = {
      load: vi.fn(async () => ({ secafsRollback: { enabled: true } })),
      keys: vi.fn(async () => ["main:abc"]),
      patch: vi.fn(),
    };
    const rpc = {
      list: vi.fn(async () => ({ mounts: [] })),
      mount: vi.fn(),
      unmount: vi.fn(),
      snapshotRestore: vi.fn(),
    };
    const result = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id: string) => `main:${id}`,
      truncateJsonlAfterMessage: vi.fn(),
      sessionFileFor: () => "x",
      trajectoryFor: () => "x",
      workspaceFor: () => "x",
      extractConvId: (k: string) => k,
    });
    expect(rpc.snapshotRestore).not.toHaveBeenCalled();
    expect(rpc.mount).not.toHaveBeenCalled();
    expect(result.rollbacksResumed).toBe(0);
  });

  it("starts rollback from scratch when fsRestored=false", async () => {
    const session = {
      secafsRollback: {
        enabled: true,
        inProgress: {
          targetSnapId: 1,
          targetMessageId: "m1",
          fsRestored: false,
          jsonlTruncated: false,
          startedAt: 0,
        },
      },
    };
    const sessions = {
      load: vi.fn(async () => session),
      keys: vi.fn(async () => ["main:xyz"]),
      patch: vi.fn(),
    };
    const rpc = {
      list: vi.fn(async () => ({ mounts: [] })),
      mount: vi.fn(async () => ({ hostPath: "/m", mounted: true })),
      unmount: vi.fn(async () => ({ unmounted: true })),
      snapshotRestore: vi.fn(async () => ({
        restored: true,
        prunedSnapshots: 1,
        prunedUndoRows: 5,
      })),
    };
    const truncate = vi.fn(async () => ({ truncated: true }));
    const result = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id: string) => `main:${id}`,
      truncateJsonlAfterMessage: truncate,
      sessionFileFor: () => "x",
      trajectoryFor: () => "x",
      workspaceFor: () => "x",
      extractConvId: (k: string) => k.split(":")[1] ?? k,
    });
    expect(rpc.unmount).toHaveBeenCalled();
    expect(rpc.snapshotRestore).toHaveBeenCalledWith({ conversationId: "xyz", snapId: 1 });
    expect(truncate).toHaveBeenCalledTimes(2);
    expect(rpc.mount).toHaveBeenCalled();
    expect(result.rollbacksResumed).toBe(1);
  });
});

describe("reconcile", () => {
  const stubNewDeps = {
    mount: vi.fn(async () => ({ hostPath: "/m", mounted: true })),
    snapshotRestore: vi.fn(async () => ({ restored: true, prunedSnapshots: 0, prunedUndoRows: 0 })),
    truncateJsonlAfterMessage: vi.fn(async () => ({ truncated: true })),
    sessionFileFor: () => "x",
    trajectoryFor: () => "x",
    workspaceFor: () => "x",
    extractConvId: (k: string) => k,
  };

  it("unmounts daemon orphans (live mount, no session record)", async () => {
    const rpc = {
      list: vi.fn().mockResolvedValue({
        mounts: [{ conversationId: "orphan", hostPath: "/x", since: "t" }],
      }),
      unmount: vi.fn().mockResolvedValue({ unmounted: true }),
      mount: stubNewDeps.mount,
      snapshotRestore: stubNewDeps.snapshotRestore,
    };
    const sessions = {
      load: vi.fn().mockResolvedValue(null),
      keys: vi.fn().mockResolvedValue([]),
      patch: vi.fn(),
    };
    const out = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id) => `main:${id}`,
      truncateJsonlAfterMessage: stubNewDeps.truncateJsonlAfterMessage,
      sessionFileFor: stubNewDeps.sessionFileFor,
      trajectoryFor: stubNewDeps.trajectoryFor,
      workspaceFor: stubNewDeps.workspaceFor,
      extractConvId: stubNewDeps.extractConvId,
    });
    expect(out.unmounted).toBe(1);
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "orphan" });
  });

  it("leaves mounts that still have a session record", async () => {
    const rpc = {
      list: vi.fn().mockResolvedValue({
        mounts: [{ conversationId: "live", hostPath: "/x", since: "t" }],
      }),
      unmount: vi.fn(),
      mount: stubNewDeps.mount,
      snapshotRestore: stubNewDeps.snapshotRestore,
    };
    const sessions = {
      load: vi.fn().mockResolvedValue({ sessionId: "live" }),
      keys: vi.fn().mockResolvedValue([]),
      patch: vi.fn(),
    };
    const out = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id) => `main:${id}`,
      truncateJsonlAfterMessage: stubNewDeps.truncateJsonlAfterMessage,
      sessionFileFor: stubNewDeps.sessionFileFor,
      trajectoryFor: stubNewDeps.trajectoryFor,
      workspaceFor: stubNewDeps.workspaceFor,
      extractConvId: stubNewDeps.extractConvId,
    });
    expect(out.unmounted).toBe(0);
    expect(rpc.unmount).not.toHaveBeenCalled();
  });

  it("returns zero when rpc.list throws, not rethrowing", async () => {
    const rpc = {
      list: vi.fn().mockRejectedValue(new Error("daemon unreachable")),
      unmount: vi.fn(),
      mount: stubNewDeps.mount,
      snapshotRestore: stubNewDeps.snapshotRestore,
    };
    const sessions = {
      load: vi.fn(),
      keys: vi.fn().mockResolvedValue([]),
      patch: vi.fn(),
    };
    const out = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id) => `main:${id}`,
      truncateJsonlAfterMessage: stubNewDeps.truncateJsonlAfterMessage,
      sessionFileFor: stubNewDeps.sessionFileFor,
      trajectoryFor: stubNewDeps.trajectoryFor,
      workspaceFor: stubNewDeps.workspaceFor,
      extractConvId: stubNewDeps.extractConvId,
    });
    expect(out.unmounted).toBe(0);
    expect(rpc.unmount).not.toHaveBeenCalled();
  });

  it("handles mixed state (one orphan + one live)", async () => {
    const rpc = {
      list: vi.fn().mockResolvedValue({
        mounts: [
          { conversationId: "a", hostPath: "/a", since: "t1" },
          { conversationId: "b", hostPath: "/b", since: "t2" },
        ],
      }),
      unmount: vi.fn().mockResolvedValue({ unmounted: true }),
      mount: stubNewDeps.mount,
      snapshotRestore: stubNewDeps.snapshotRestore,
    };
    const sessions = {
      load: vi
        .fn()
        .mockImplementation(async (key: string) => (key === "main:a" ? null : { sessionId: "b" })),
      keys: vi.fn().mockResolvedValue([]),
      patch: vi.fn(),
    };
    const out = await reconcile({
      rpc,
      sessions,
      sessionKeyFor: (id) => `main:${id}`,
      truncateJsonlAfterMessage: stubNewDeps.truncateJsonlAfterMessage,
      sessionFileFor: stubNewDeps.sessionFileFor,
      trajectoryFor: stubNewDeps.trajectoryFor,
      workspaceFor: stubNewDeps.workspaceFor,
      extractConvId: stubNewDeps.extractConvId,
    });
    expect(out.unmounted).toBe(1);
    expect(rpc.unmount).toHaveBeenCalledTimes(1);
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "a" });
  });
});
