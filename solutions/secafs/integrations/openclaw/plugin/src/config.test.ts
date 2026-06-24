import { describe, expect, it } from "vitest";
import { resolveSecafsConfig } from "./config.js";

describe("resolveSecafsConfig", () => {
  it("defaults manageDaemon=false and uses XDG_RUNTIME_DIR socket", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: {} }, { XDG_RUNTIME_DIR: "/run/user/1001" });
    expect(cfg.manageDaemon).toBe(false);
    expect(cfg.socketPath).toBe("/run/user/1001/secafs/secafs.sock");
  });

  it("requires postgresUrl when manageDaemon=true", () => {
    expect(() => resolveSecafsConfig({ pluginConfig: { manageDaemon: true } }, {})).toThrow(
      /postgresUrl/,
    );
    expect(() =>
      resolveSecafsConfig(
        { pluginConfig: { manageDaemon: true, postgresUrl: "postgres://x" } },
        {},
      ),
    ).not.toThrow();
  });

  it("does not require postgresUrl when manageDaemon=false (default)", () => {
    expect(() => resolveSecafsConfig({ pluginConfig: {} }, {})).not.toThrow();
  });

  it("accepts explicit socketPath override", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: { socketPath: "/tmp/custom.sock" } }, {});
    expect(cfg.socketPath).toBe("/tmp/custom.sock");
  });

  it("defaults mountRoot under XDG_STATE_HOME when set", () => {
    const cfg = resolveSecafsConfig(
      { pluginConfig: {} },
      { XDG_STATE_HOME: "/home/kou/.local/state" },
    );
    expect(cfg.mountRoot).toBe("/home/kou/.local/state/secafs/mounts");
  });

  it("falls back to $HOME/.local/state/secafs/mounts when XDG_STATE_HOME absent", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: {} }, { HOME: "/home/kou" });
    expect(cfg.mountRoot).toBe("/home/kou/.local/state/secafs/mounts");
  });

  it("accepts mountRoot override", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: { mountRoot: "/var/secafs/mounts" } }, {});
    expect(cfg.mountRoot).toBe("/var/secafs/mounts");
  });

  it("defaults idle-unmount OFF (vanished-workspace trap) but keeps the keeper scanning", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: {} }, {});
    expect(cfg.idleUnmountSeconds).toBe(0);
    expect(cfg.idleScanSeconds).toBe(2);
  });

  it("accepts numeric overrides for idle knobs", () => {
    const cfg = resolveSecafsConfig(
      { pluginConfig: { idleUnmountSeconds: 300, idleScanSeconds: 30 } },
      {},
    );
    expect(cfg.idleUnmountSeconds).toBe(300);
    expect(cfg.idleScanSeconds).toBe(30);
  });

  it("allows 0 to disable idle handling", () => {
    const cfg = resolveSecafsConfig(
      { pluginConfig: { idleUnmountSeconds: 0, idleScanSeconds: 0 } },
      {},
    );
    expect(cfg.idleUnmountSeconds).toBe(0);
    expect(cfg.idleScanSeconds).toBe(0);
  });

  it("rejects negative or non-finite idle knobs and falls back to defaults", () => {
    const cfg = resolveSecafsConfig(
      { pluginConfig: { idleUnmountSeconds: -1, idleScanSeconds: Number.NaN } },
      {},
    );
    expect(cfg.idleUnmountSeconds).toBe(0);
    expect(cfg.idleScanSeconds).toBe(2);
  });

  it("defaults enableRollbackUI to true", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: {} }, {});
    expect(cfg.enableRollbackUI).toBe(true);
  });

  it("respects enableRollbackUI=false from plugin config", () => {
    const cfg = resolveSecafsConfig({ pluginConfig: { enableRollbackUI: false } }, {});
    expect(cfg.enableRollbackUI).toBe(false);
  });
});
