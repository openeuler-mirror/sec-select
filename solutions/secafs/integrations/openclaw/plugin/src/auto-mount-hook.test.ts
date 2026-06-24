import { describe, expect, it, vi } from "vitest";
import { ensureSecafsMountForSession, type SessionEntryView } from "./auto-mount-hook.js";

function entry(partial: Partial<SessionEntryView>): SessionEntryView {
  return {
    sessionId: "sid",
    updatedAt: 0,
    ...partial,
  };
}

function makeDeps(store: Record<string, SessionEntryView>) {
  const rpc = { mount: vi.fn().mockResolvedValue({ hostPath: "/mnt/abc", mounted: true }) };
  const patchSession = vi.fn().mockResolvedValue(undefined);
  const wsSet = vi.fn().mockResolvedValue(undefined);
  return {
    rpc,
    patchSession,
    wsSet,
    deps: {
      loadStore: () => store,
      patchSession,
      workspace: { set: wsSet },
      rpc,
      mountRoot: "/mnt",
    },
  };
}

describe("ensureSecafsMountForSession", () => {
  it("no-ops for empty session key", async () => {
    const { deps, rpc } = makeDeps({});
    await ensureSecafsMountForSession(deps, undefined);
    expect(rpc.mount).not.toHaveBeenCalled();
  });

  it("no-ops for non-secafs sessions", async () => {
    const store = {
      "agent:main:main": entry({ sessionId: "regular", mountState: undefined }),
    };
    const { deps, rpc } = makeDeps(store);
    await ensureSecafsMountForSession(deps, "agent:main:main");
    expect(rpc.mount).not.toHaveBeenCalled();
  });

  it("no-ops when secafs session is already mounted", async () => {
    const store = {
      "main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "mounted",
      }),
    };
    const { deps, rpc, patchSession } = makeDeps(store);
    await ensureSecafsMountForSession(deps, "main:abc");
    expect(rpc.mount).not.toHaveBeenCalled();
    expect(patchSession).not.toHaveBeenCalled();
  });

  it("mounts and re-sets workspace + mountState when bare key is unmounted", async () => {
    const store = {
      "main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "unmounted",
      }),
    };
    const { deps, rpc, patchSession, wsSet } = makeDeps(store);
    await ensureSecafsMountForSession(deps, "main:abc");
    expect(rpc.mount).toHaveBeenCalledWith({
      conversationId: "abc",
      hostPath: "/mnt/abc",
    });
    expect(wsSet).toHaveBeenCalledWith("main:abc", { path: "/mnt/abc" });
    expect(patchSession).toHaveBeenCalledWith("main:abc", { mountState: "mounted" });
  });

  it("resolves SecAFS metadata via bare key when invoked with agent-canonical key", async () => {
    const store = {
      "main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "unmounted",
      }),
      "agent:main:main:abc": entry({ sessionId: "agent-side" }),
    };
    const { deps, rpc, patchSession, wsSet } = makeDeps(store);
    await ensureSecafsMountForSession(deps, "agent:main:main:abc");
    expect(rpc.mount).toHaveBeenCalledWith({
      conversationId: "abc",
      hostPath: "/mnt/abc",
    });
    // Workspace + mountState must land on the bare key (where SecAFS metadata lives).
    expect(wsSet).toHaveBeenCalledWith("main:abc", { path: "/mnt/abc" });
    expect(patchSession).toHaveBeenCalledWith("main:abc", { mountState: "mounted" });
  });

  it("logs and swallows mount errors (does not throw)", async () => {
    const store = {
      "main:abc": entry({
        sessionId: "abc",
        kind: "secafs",
        mountState: "unmounted",
      }),
    };
    const { deps, rpc, patchSession } = makeDeps(store);
    rpc.mount.mockRejectedValueOnce(new Error("daemon unreachable"));
    const warn = vi.fn();
    await expect(
      ensureSecafsMountForSession({ ...deps, logger: { warn, info: vi.fn() } }, "main:abc"),
    ).resolves.toBeUndefined();
    expect(warn).toHaveBeenCalled();
    expect(patchSession).not.toHaveBeenCalled();
  });
});
