import { describe, expect, it, vi } from "vitest";
import { createSpawnedWorkspaceRedirect } from "./spawned-workspace.js";

describe("createSpawnedWorkspaceRedirect (Path C)", () => {
  it("set writes spawnedCwd + spawnedWorkspaceDir + spawnedBy", async () => {
    const patch = vi.fn().mockResolvedValue(undefined);
    const ws = createSpawnedWorkspaceRedirect({ patch }, "secafs-chat");
    await ws.set("main:abc", { path: "/mnt/abc" });
    // spawnedCwd drives the agent's real process cwd (resolveSessionRuntimeCwd);
    // spawnedWorkspaceDir drives the agent workspaceDir. Both must point at the mount.
    expect(patch).toHaveBeenCalledWith("main:abc", {
      spawnedCwd: "/mnt/abc",
      spawnedWorkspaceDir: "/mnt/abc",
      spawnedBy: "secafs-chat",
    });
  });

  it("clear unsets spawnedBy and never touches spawnedCwd/spawnedWorkspaceDir", async () => {
    const patch = vi.fn().mockResolvedValue(undefined);
    const ws = createSpawnedWorkspaceRedirect({ patch }, "secafs-chat");
    await ws.clear("main:abc");
    expect(patch).toHaveBeenCalledWith("main:abc", { spawnedBy: undefined });
    const [, sentPatch] = patch.mock.calls[0];
    expect("spawnedCwd" in sentPatch).toBe(false);
    expect("spawnedWorkspaceDir" in sentPatch).toBe(false);
  });

  it("stamps the caller's plugin id as the ownership marker", async () => {
    const patch = vi.fn().mockResolvedValue(undefined);
    const ws = createSpawnedWorkspaceRedirect({ patch }, "other-plugin");
    await ws.set("acp:xyz", { path: "/m/xyz" });
    expect(patch).toHaveBeenCalledWith("acp:xyz", {
      spawnedCwd: "/m/xyz",
      spawnedWorkspaceDir: "/m/xyz",
      spawnedBy: "other-plugin",
    });
  });
});
