import { describe, expect, it, vi } from "vitest";
import { handleSessionEnd } from "./session-bindings.js";

function makeDeps() {
  return {
    rpc: { unmount: vi.fn().mockResolvedValue({ unmounted: true }) },
    sessions: { patch: vi.fn().mockResolvedValue(undefined) },
  };
}

describe("handleSessionEnd", () => {
  it("unmounts sessions managed by secafs-chat", async () => {
    const deps = makeDeps();
    await handleSessionEnd(
      { sessionKey: "main:s1", workspace: { managedBy: "secafs-chat" } },
      deps,
    );
    expect(deps.rpc.unmount).toHaveBeenCalledWith({ conversationId: "s1" });
    expect(deps.sessions.patch).toHaveBeenCalledWith("main:s1", { mountState: "unmounted" });
  });

  it("ignores sessions with no workspace override", async () => {
    const deps = makeDeps();
    await handleSessionEnd({ sessionKey: "main:s2" }, deps);
    expect(deps.rpc.unmount).not.toHaveBeenCalled();
    expect(deps.sessions.patch).not.toHaveBeenCalled();
  });

  it("ignores sessions managed by other plugins", async () => {
    const deps = makeDeps();
    await handleSessionEnd(
      { sessionKey: "main:s3", workspace: { managedBy: "other-plugin" } },
      deps,
    );
    expect(deps.rpc.unmount).not.toHaveBeenCalled();
  });

  it("swallows unmount errors but still patches state", async () => {
    const deps = makeDeps();
    deps.rpc.unmount.mockRejectedValueOnce(new Error("boom"));
    await handleSessionEnd(
      { sessionKey: "main:s4", workspace: { managedBy: "secafs-chat" } },
      deps,
    );
    expect(deps.sessions.patch).toHaveBeenCalledWith("main:s4", { mountState: "unmounted" });
  });
});
