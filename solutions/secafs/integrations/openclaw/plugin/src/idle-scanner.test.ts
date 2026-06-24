import { describe, expect, it, vi } from "vitest";
import { scanOnce, type SessionEntryView } from "./idle-scanner.js";

function entry(partial: Partial<SessionEntryView>): SessionEntryView {
  return {
    sessionId: "sid",
    updatedAt: 0,
    ...partial,
  };
}

function makeDeps(
  store: Record<string, SessionEntryView>,
  daemonMounts: string[] = [],
) {
  const rpc = {
    unmount: vi.fn().mockResolvedValue({ unmounted: true }),
    list: vi.fn().mockResolvedValue({
      mounts: daemonMounts.map((conversationId) => ({ conversationId })),
    }),
    mount: vi.fn().mockImplementation(async (p: { conversationId: string }) => ({
      hostPath: `/mnt/${p.conversationId}`,
      mounted: true,
    })),
  };
  const patchSession = vi.fn().mockResolvedValue(undefined);
  const workspaceSet = vi.fn().mockResolvedValue(undefined);
  const probeMount = vi.fn().mockResolvedValue(true);
  return {
    rpc,
    patchSession,
    workspaceSet,
    probeMount,
    deps: {
      loadStore: () => store,
      patchSession,
      rpc,
      workspace: { set: workspaceSet },
      mountRoot: "/mnt",
      probeMount,
    },
  };
}

describe("idle scanner scanOnce", () => {
  it("unmounts secafs sessions older than idleMs", async () => {
    const store = {
      "main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "mounted",
        updatedAt: 1000,
      }),
    };
    const { deps, rpc, patchSession } = makeDeps(store);
    await scanOnce(deps, 5000, () => 10_000); // now=10s, idle ≥5s, last=1s → unmount
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "abc" });
    expect(patchSession).toHaveBeenCalledWith("main:abc", { mountState: "unmounted" });
  });

  it("skips sessions younger than idleMs", async () => {
    const store = {
      "main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "mounted",
        updatedAt: 9_500,
      }),
    };
    const { deps, rpc, patchSession } = makeDeps(store, ["abc"]);
    await scanOnce(deps, 5000, () => 10_000); // now=10s, idle ≥5s, last=9.5s → keep
    expect(rpc.unmount).not.toHaveBeenCalled();
    expect(patchSession).not.toHaveBeenCalled();
  });

  it("ignores non-secafs sessions", async () => {
    const store = {
      "agent:main:main": entry({
        sessionId: "regular",
        mountState: "mounted",
        updatedAt: 1000,
      }),
    };
    const { deps, rpc } = makeDeps(store);
    await scanOnce(deps, 5000, () => 10_000);
    expect(rpc.unmount).not.toHaveBeenCalled();
  });

  it("ignores already-unmounted secafs sessions", async () => {
    const store = {
      "main:xyz": entry({
        sessionId: "xyz",
        kind: "secafs",
        mountState: "unmounted",
        updatedAt: 1000,
      }),
    };
    const { deps, rpc } = makeDeps(store);
    await scanOnce(deps, 5000, () => 10_000);
    expect(rpc.unmount).not.toHaveBeenCalled();
  });

  it("derives conversationId from agent-canonical key form", async () => {
    const store = {
      "agent:main:main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "mounted",
        updatedAt: 1000,
      }),
    };
    const { deps, rpc } = makeDeps(store);
    await scanOnce(deps, 5000, () => 10_000);
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "abc" });
  });

  it("logs and continues when unmount fails for a single session", async () => {
    const store = {
      "main:bad": entry({
        sessionId: "bad",
        kind: "secafs",
        mountState: "mounted",
        updatedAt: 1000,
      }),
      "main:ok": entry({
        sessionId: "ok",
        kind: "secafs",
        mountState: "mounted",
        updatedAt: 1000,
      }),
    };
    const { deps, rpc, patchSession } = makeDeps(store, ["bad", "ok"]);
    rpc.unmount.mockRejectedValueOnce(new Error("boom")).mockResolvedValueOnce({ unmounted: true });
    const warn = vi.fn();
    await scanOnce({ ...deps, logger: { warn, info: vi.fn() } }, 5000, () => 10_000);
    expect(rpc.unmount).toHaveBeenCalledTimes(2);
    // First call failed → no patch for bad; second succeeded → patch for ok.
    expect(patchSession).toHaveBeenCalledWith("main:ok", { mountState: "unmounted" });
    expect(patchSession).not.toHaveBeenCalledWith("main:bad", expect.anything());
    expect(warn).toHaveBeenCalled();
  });

  it("treats missing updatedAt as ancient (definitely idle)", async () => {
    const store = {
      "main:noTs": entry({
        sessionId: "noTs",
        kind: "secafs",
        mountState: "mounted",
      }),
    };
    const { deps, rpc } = makeDeps(store);
    await scanOnce(deps, 5000, () => 10_000);
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "noTs" });
  });

  it("skips secafs sessions whose secafsRollback.inProgress is set", async () => {
    const store = {
      "main:rolling": entry({
        sessionId: "rolling",
        kind: "secafs",
        mountState: "mounted",
        updatedAt: 1000,
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
      }),
    };
    const { deps, rpc, patchSession } = makeDeps(store);
    await scanOnce(deps, 5000, () => 10_000);
    expect(rpc.unmount).not.toHaveBeenCalled();
    expect(patchSession).not.toHaveBeenCalled();
  });

  it("mount-keeper remounts store-mounted sessions the daemon lost", async () => {
    const store = {
      "main:lost1": entry({ sessionId: "lost1", kind: "secafs", mountState: "mounted", updatedAt: Date.now() }),
      // canonical twin of lost1 — must remount the sid only once
      "agent:main:main:lost1": entry({ sessionId: "lost1", kind: "secafs", mountState: "mounted", updatedAt: Date.now() }),
      "main:alive": entry({ sessionId: "alive", kind: "secafs", mountState: "mounted", updatedAt: Date.now() }),
      "main:closed": entry({ sessionId: "closed", kind: "secafs", mountState: "unmounted", updatedAt: Date.now() }),
    };
    const { rpc, workspaceSet, deps } = makeDeps(store, ["alive"]);
    await scanOnce(deps, 0, () => Date.now());
    expect(rpc.mount).toHaveBeenCalledTimes(1);
    expect(rpc.mount).toHaveBeenCalledWith({ conversationId: "lost1", hostPath: "/mnt/lost1" });
    expect(workspaceSet).toHaveBeenCalledWith("main:lost1", { path: "/mnt/lost1" });
    // only the defensive pre-unmount of the lost sid; no idle-unmount duty (idleMs=0)
    expect(rpc.unmount).toHaveBeenCalledTimes(1);
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "lost1" });
  });

  it("mount-keeper remounts zombie mounts (daemon claims it, kernel lost it)", async () => {
    const store = {
      "main:zombie": entry({ sessionId: "zombie", kind: "secafs", mountState: "mounted", updatedAt: Date.now() }),
    };
    const { rpc, probeMount, deps } = makeDeps(store, ["zombie"]);
    probeMount.mockResolvedValueOnce(false);
    await scanOnce(deps, 0, () => Date.now());
    expect(rpc.unmount).toHaveBeenCalledWith({ conversationId: "zombie" });
    expect(rpc.mount).toHaveBeenCalledWith({ conversationId: "zombie", hostPath: "/mnt/zombie" });
  });

  it("mount-keeper skips everything when rpc.list fails (daemon down)", async () => {
    const store = {
      "main:lost1": entry({ sessionId: "lost1", kind: "secafs", mountState: "mounted", updatedAt: Date.now() }),
    };
    const { rpc, deps } = makeDeps(store);
    rpc.list.mockRejectedValueOnce(new Error("daemon down"));
    await scanOnce(deps, 0, () => Date.now());
    expect(rpc.mount).not.toHaveBeenCalled();
  });

});
