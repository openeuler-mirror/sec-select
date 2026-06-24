import { EventEmitter } from "node:events";
import { describe, expect, it, vi } from "vitest";
import { createDaemonSupervisor } from "./daemon-supervisor.js";

class FakeChild extends EventEmitter {
  pid: number;
  killSignals: string[] = [];
  constructor(pid: number) {
    super();
    this.pid = pid;
  }
  kill(signal?: string): boolean {
    this.killSignals.push(signal ?? "SIGTERM");
    queueMicrotask(() => this.emit("exit", 0));
    return true;
  }
}

class FakeChildNoKillExit extends EventEmitter {
  pid = 1;
  kill(): boolean {
    return true;
  }
}

function fakeSpawn(calls: string[]) {
  return ((bin: string, args: readonly string[]) => {
    calls.push(`${bin} ${args.join(" ")}`);
    return new FakeChild(4242);
  }) as unknown as typeof import("node:child_process").spawn;
}

describe("daemon supervisor", () => {
  it("does not spawn when manageDaemon=false", async () => {
    const calls: string[] = [];
    const s = createDaemonSupervisor({
      manageDaemon: false,
      binary: "secafs",
      args: ["serve", "api"],
      spawn: fakeSpawn(calls),
    });
    await s.start();
    expect(calls).toEqual([]);
    expect(s.isRunning()).toBe(false);
  });

  it("spawns secafs serve api when manageDaemon=true", async () => {
    const calls: string[] = [];
    const s = createDaemonSupervisor({
      manageDaemon: true,
      binary: "secafs",
      args: ["serve", "api", "--socket", "/tmp/x.sock", "--pg-url", "postgres://p"],
      spawn: fakeSpawn(calls),
    });
    await s.start();
    expect(calls).toEqual(["secafs serve api --socket /tmp/x.sock --pg-url postgres://p"]);
    expect(s.isRunning()).toBe(true);
  });

  it("start is idempotent (no second spawn)", async () => {
    const calls: string[] = [];
    const s = createDaemonSupervisor({
      manageDaemon: true,
      binary: "secafs",
      args: ["serve", "api"],
      spawn: fakeSpawn(calls),
    });
    await s.start();
    await s.start();
    expect(calls.length).toBe(1);
  });

  it("stop signals SIGTERM and resolves on exit", async () => {
    const calls: string[] = [];
    const s = createDaemonSupervisor({
      manageDaemon: true,
      binary: "secafs",
      args: ["serve", "api"],
      spawn: fakeSpawn(calls),
    });
    await s.start();
    expect(s.isRunning()).toBe(true);
    await s.stop();
    expect(s.isRunning()).toBe(false);
  });

  it("onExit fires when child exits unprompted", async () => {
    const onExit = vi.fn();
    let emittedChild: FakeChildNoKillExit | null = null;
    const fakeSpawnCapture = ((_bin: string, _args: readonly string[]) => {
      const child = new FakeChildNoKillExit();
      emittedChild = child;
      return child;
    }) as unknown as typeof import("node:child_process").spawn;

    const s = createDaemonSupervisor({
      manageDaemon: true,
      binary: "secafs",
      args: ["serve", "api"],
      spawn: fakeSpawnCapture,
      onExit,
    });
    await s.start();
    emittedChild!.emit("exit", 137);
    expect(onExit).toHaveBeenCalledWith(137);
    expect(s.isRunning()).toBe(false);
  });
});
