import { mkdtemp, rm } from "node:fs/promises";
import { type Server, type Socket, createServer } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { createSecafsRpcClient } from "./rpc-client.js";

let dir: string;
let socketPath: string;
let server: Server;

beforeAll(async () => {
  dir = await mkdtemp(path.join(tmpdir(), "rpc-"));
  socketPath = path.join(dir, "s.sock");
  server = createServer((sock: Socket) => {
    sock.on("data", (buf) => {
      for (const line of buf.toString().split("\n")) {
        if (!line.trim()) {
          continue;
        }
        const req = JSON.parse(line) as {
          id: number;
          method: string;
          params: Record<string, unknown>;
        };
        let resp: Record<string, unknown>;
        if (req.method === "secafs.v1.ping") {
          resp = {
            jsonrpc: "2.0",
            id: req.id,
            result: { version: "0.0", pgConnected: true, mountCount: 0 },
          };
        } else if (req.method === "secafs.v1.mount") {
          resp = {
            jsonrpc: "2.0",
            id: req.id,
            result: { hostPath: "/tmp/mnt", mounted: true },
          };
        } else {
          resp = {
            jsonrpc: "2.0",
            id: req.id,
            error: { code: -32601, message: "not found" },
          };
        }
        sock.write(JSON.stringify(resp) + "\n");
      }
    });
  });
  await new Promise<void>((resolve) => server.listen(socketPath, () => resolve()));
});

afterAll(async () => {
  await new Promise<void>((resolve) => server.close(() => resolve()));
  await rm(dir, { recursive: true, force: true });
});

describe("secafs RPC client", () => {
  it("ping returns structured result", async () => {
    const client = createSecafsRpcClient({ socketPath });
    try {
      const out = await client.ping();
      expect(out.pgConnected).toBe(true);
      expect(out.mountCount).toBe(0);
    } finally {
      client.close();
    }
  });

  it("mount returns hostPath and mounted=true", async () => {
    const client = createSecafsRpcClient({ socketPath });
    try {
      const out = await client.mount({ conversationId: "a" });
      expect(out.hostPath).toBe("/tmp/mnt");
      expect(out.mounted).toBe(true);
    } finally {
      client.close();
    }
  });

  it("throws SecafsRpcError on unknown method", async () => {
    const client = createSecafsRpcClient({ socketPath });
    try {
      await expect(client.call("secafs.v1.bogus", {})).rejects.toMatchObject({
        name: "SecafsRpcError",
        code: -32601,
      });
    } finally {
      client.close();
    }
  });

  it("retries connect across a transient daemon restart", async () => {
    // Simulate daemon-restart: client is created with no server bound, then
    // the server starts shortly after the call is issued. With backoff
    // retries, the call should succeed instead of failing on first ENOENT.
    const restartDir = await mkdtemp(path.join(tmpdir(), "rpc-restart-"));
    const restartSocket = path.join(restartDir, "s.sock");
    const client = createSecafsRpcClient({
      socketPath: restartSocket,
      timeoutMs: 5_000,
      connectBackoffsMs: [0, 100, 250, 500],
    });
    // Fire the call before the server exists.
    const pingPromise = client.ping();
    // After 200ms (between retry attempts), bring the server up. Wrap the
    // server in a Promise so the type is naturally Server (avoids
    // no-redundant-type-constituents on a `Server | undefined` annotation
    // that the lint config can't fully resolve).
    const serverReady = new Promise<Server>((resolveServer) => {
      setTimeout(() => {
        const s = createServer((sock: Socket) => {
          sock.on("data", (buf) => {
            for (const line of buf.toString().split("\n")) {
              if (!line.trim()) {
                continue;
              }
              const req = JSON.parse(line) as { id: number; method: string };
              if (req.method === "secafs.v1.ping") {
                sock.write(
                  JSON.stringify({
                    jsonrpc: "2.0",
                    id: req.id,
                    result: { version: "0.0", pgConnected: true, mountCount: 0 },
                  }) + "\n",
                );
              }
            }
          });
        });
        s.listen(restartSocket, () => resolveServer(s));
      }, 200);
    });
    try {
      const out = await pingPromise;
      expect(out.pgConnected).toBe(true);
    } finally {
      client.close();
      const restartServer = await serverReady;
      await new Promise<void>((resolve) => restartServer.close(() => resolve()));
      await rm(restartDir, { recursive: true, force: true });
    }
  });

  it("rejects with last connect error after exhausting retries", async () => {
    const missingDir = await mkdtemp(path.join(tmpdir(), "rpc-missing-"));
    const missingSocket = path.join(missingDir, "nope.sock");
    const client = createSecafsRpcClient({
      socketPath: missingSocket,
      timeoutMs: 2_000,
      connectBackoffsMs: [0, 50, 50],
    });
    try {
      await expect(client.ping()).rejects.toThrow();
    } finally {
      client.close();
      await rm(missingDir, { recursive: true, force: true });
    }
  });

  it("reconnects after server-side close mid-session", async () => {
    // Spawn an ephemeral server, make a call, drop the connection from the
    // server side, then make another call and expect it to succeed via a
    // fresh socket.
    const flapDir = await mkdtemp(path.join(tmpdir(), "rpc-flap-"));
    const flapSocket = path.join(flapDir, "s.sock");
    const conns: Socket[] = [];
    const flapServer = createServer((sock: Socket) => {
      conns.push(sock);
      sock.on("data", (buf) => {
        for (const line of buf.toString().split("\n")) {
          if (!line.trim()) {
            continue;
          }
          const req = JSON.parse(line) as { id: number; method: string };
          sock.write(
            JSON.stringify({
              jsonrpc: "2.0",
              id: req.id,
              result: { version: "0.0", pgConnected: true, mountCount: 0 },
            }) + "\n",
          );
        }
      });
    });
    await new Promise<void>((resolve) => flapServer.listen(flapSocket, () => resolve()));
    const client = createSecafsRpcClient({ socketPath: flapSocket, timeoutMs: 2_000 });
    try {
      await client.ping();
      // Server-side drop.
      for (const c of conns) {
        c.destroy();
      }
      // Give the close event a microtask to propagate.
      await new Promise((r) => setTimeout(r, 20));
      const out = await client.ping();
      expect(out.pgConnected).toBe(true);
    } finally {
      client.close();
      await new Promise<void>((resolve) => flapServer.close(() => resolve()));
      await rm(flapDir, { recursive: true, force: true });
    }
  });

  it("timeout rejects if server never responds", async () => {
    // Start a socket that accepts but never replies
    const openSockets: Socket[] = [];
    const silent = createServer((sock: Socket) => {
      openSockets.push(sock);
    });
    const silentSock = path.join(dir, "silent.sock");
    await new Promise<void>((resolve) => silent.listen(silentSock, () => resolve()));
    try {
      const client = createSecafsRpcClient({ socketPath: silentSock, timeoutMs: 100 });
      try {
        await expect(client.call("secafs.v1.ping", {})).rejects.toThrow(/timeout/);
      } finally {
        client.close();
      }
    } finally {
      for (const s of openSockets) {
        s.destroy();
      }
      await new Promise<void>((resolve) => silent.close(() => resolve()));
    }
  });
});

describe("rpc-client snapshot extensions", () => {
  it("forwards snapshotCommit args and returns parsed result", async () => {
    // Create a mock server that handles snapshot methods
    const snapshotDir = await mkdtemp(path.join(tmpdir(), "rpc-snapshot-"));
    const snapshotSocket = path.join(snapshotDir, "s.sock");
    const capturedCalls: { method: string; params: Record<string, unknown> }[] = [];
    const snapshotServer = createServer((sock: Socket) => {
      sock.on("data", (buf) => {
        for (const line of buf.toString().split("\n")) {
          if (!line.trim()) {
            continue;
          }
          const req = JSON.parse(line) as {
            id: number;
            method: string;
            params: Record<string, unknown>;
          };
          capturedCalls.push({ method: req.method, params: req.params });
          sock.write(
            JSON.stringify({
              jsonrpc: "2.0",
              id: req.id,
              result: {
                snapId: 7,
                committedAt: "2026-01-01T00:00:00Z",
                label: "msg-7",
              },
            }) + "\n",
          );
        }
      });
    });
    await new Promise<void>((resolve) => snapshotServer.listen(snapshotSocket, () => resolve()));
    try {
      const client = createSecafsRpcClient({ socketPath: snapshotSocket });
      try {
        const r = await client.snapshotCommit({ conversationId: "v", label: "msg-7" });
        expect(capturedCalls.at(-1)).toEqual({
          method: "secafs.v1.snapshot.commit",
          params: { conversationId: "v", label: "msg-7" },
        });
        expect(r.snapId).toBe(7);
        expect(r.committedAt).toBe("2026-01-01T00:00:00Z");
        expect(r.label).toBe("msg-7");
      } finally {
        client.close();
      }
    } finally {
      await new Promise<void>((resolve) => snapshotServer.close(() => resolve()));
      await rm(snapshotDir, { recursive: true, force: true });
    }
  });

  it("exposes snapshotEnable/Disable/List/Restore", () => {
    const client = createSecafsRpcClient({ socketPath });
    expect(typeof client.snapshotEnable).toBe("function");
    expect(typeof client.snapshotDisable).toBe("function");
    expect(typeof client.snapshotList).toBe("function");
    expect(typeof client.snapshotRestore).toBe("function");
    client.close();
  });
});
