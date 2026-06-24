import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import {
  type RegisterGatewayMethodsOpts,
  type SessionArchive,
  registerGatewayMethods,
} from "./gateway-methods.js";

function makeHarness(extra: Partial<RegisterGatewayMethodsOpts> = {}) {
  const rpc = {
    ping: vi.fn().mockResolvedValue({ version: "0.0", pgConnected: true, mountCount: 0 }),
    list: vi.fn().mockResolvedValue({ mounts: [] }),
    mount: vi.fn().mockResolvedValue({ hostPath: "/tmp/mnt/xyz", mounted: true }),
    unmount: vi.fn().mockResolvedValue({ unmounted: true }),
    destroy: vi.fn().mockResolvedValue({ destroyed: true }),
    snapshotEnable: vi.fn().mockResolvedValue({ enabled: true, currentSnapId: 1 }),
    snapshotDisable: vi
      .fn()
      .mockResolvedValue({ disabled: true, purgedSnapshots: 0, purgedUndoRows: 0 }),
    snapshotCommit: vi.fn(),
    snapshotList: vi.fn().mockResolvedValue({ snapshots: [] }),
    snapshotRestore: vi.fn(),
  };
  const sessions = {
    create: vi.fn().mockResolvedValue({ sessionKey: "main:abc" }),
    patch: vi.fn().mockResolvedValue(undefined),
    load: vi.fn().mockResolvedValue({ sessionId: "abc" }),
    delete: vi.fn().mockResolvedValue(undefined),
    entries: vi.fn().mockResolvedValue({}),
  };
  const workspace = {
    set: vi.fn().mockResolvedValue(undefined),
    clear: vi.fn().mockResolvedValue(undefined),
  };
  const registered = new Map<string, (params: unknown) => Promise<unknown>>();
  registerGatewayMethods({
    registerGatewayMethod: (m, h) => {
      registered.set(m, h);
    },
    rpc,
    sessions,
    workspace,
    mountRoot: "/tmp/mnt",
    handleRestore: vi.fn().mockResolvedValue({
      restored: true as const,
      restoredMessageId: "m1",
      prunedSnapshots: 0,
      prunedUndoRows: 0,
    }),
    ...extra,
  });
  return { rpc, sessions, workspace, registered };
}

describe("secafs.* gateway methods", () => {
  it("status delegates to rpc.ping when daemon reachable", async () => {
    const h = makeHarness();
    const out = await h.registered.get("secafs.status")!({});
    expect(out).toMatchObject({ daemonReachable: true, pgConnected: true, mountCount: 0 });
    expect(h.rpc.ping).toHaveBeenCalled();
  });

  it("status reports unreachable when ping throws", async () => {
    const h = makeHarness();
    h.rpc.ping.mockRejectedValueOnce(new Error("no connection"));
    const out = await h.registered.get("secafs.status")!({});
    expect(out).toEqual({ daemonReachable: false, pgConnected: false, mountCount: 0 });
  });

  it("session.create creates session, mounts, sets workspace, patches state", async () => {
    const h = makeHarness();
    const out = await h.registered.get("secafs.session.create")!({});
    expect(h.sessions.create).toHaveBeenCalledWith({ kind: "secafs" });
    expect(h.rpc.mount).toHaveBeenCalledWith({ conversationId: "abc", hostPath: "/tmp/mnt/abc" });
    expect(h.workspace.set).toHaveBeenCalledWith("main:abc", { path: "/tmp/mnt/xyz" });
    expect(h.sessions.patch).toHaveBeenCalledWith("main:abc", { mountState: "mounted" });
    expect(out).toMatchObject({ sessionKey: "main:abc", hostPath: "/tmp/mnt/xyz" });
  });

  it("session.open remounts and re-sets workspace", async () => {
    const h = makeHarness();
    const out = await h.registered.get("secafs.session.open")!({ sessionKey: "main:abc" });
    expect(h.rpc.mount).toHaveBeenCalledWith({ conversationId: "abc", hostPath: "/tmp/mnt/abc" });
    expect(h.workspace.set).toHaveBeenCalledWith("main:abc", { path: "/tmp/mnt/xyz" });
    expect(out).toEqual({ hostPath: "/tmp/mnt/xyz" });
  });

  it("session.close unmounts, clears workspace override, and patches state but preserves data", async () => {
    const h = makeHarness();
    const out = await h.registered.get("secafs.session.close")!({ sessionKey: "main:abc" });
    expect(h.rpc.unmount).toHaveBeenCalledWith({ conversationId: "abc" });
    expect(h.workspace.clear).toHaveBeenCalledWith("main:abc");
    expect(h.sessions.patch).toHaveBeenCalledWith("main:abc", { mountState: "unmounted" });
    expect(h.sessions.delete).not.toHaveBeenCalled();
    expect(out).toEqual({ unmounted: true });
  });

  it("session.destroy unmounts, destroys, clears workspace, deletes session", async () => {
    const h = makeHarness();
    const out = await h.registered.get("secafs.session.destroy")!({ sessionKey: "main:abc" });
    expect(h.rpc.destroy).toHaveBeenCalledWith({ conversationId: "abc" });
    expect(h.workspace.clear).toHaveBeenCalledWith("main:abc");
    expect(h.sessions.delete).toHaveBeenCalledWith("main:abc");
    expect(out).toEqual({ destroyed: true });
  });

  it("session.destroy deletes chat transcripts before removing store entries", async () => {
    const calls: string[] = [];
    const rpc = {
      ping: vi.fn(),
      list: vi.fn().mockResolvedValue({ mounts: [] }),
      mount: vi.fn(),
      unmount: vi.fn().mockResolvedValue({ unmounted: true }),
      destroy: vi.fn().mockResolvedValue({ destroyed: true }),
      snapshotEnable: vi.fn(),
      snapshotDisable: vi.fn(),
      snapshotCommit: vi.fn(),
      snapshotList: vi.fn(),
      snapshotRestore: vi.fn(),
    };
    const sessions = {
      create: vi.fn(),
      patch: vi.fn(),
      load: vi.fn(),
      delete: vi.fn().mockImplementation(async (k: string) => {
        calls.push(`delete:${k}`);
      }),
      entries: vi.fn().mockResolvedValue({}),
    };
    const registered = new Map<string, (params: unknown) => Promise<unknown>>();
    registerGatewayMethods({
      registerGatewayMethod: (m, h) => {
        registered.set(m, h);
      },
      rpc,
      sessions,
      workspace: { set: vi.fn(), clear: vi.fn() },
      mountRoot: "/tmp/mnt",
      handleRestore: vi.fn(),
      deleteSessionArtifacts: vi.fn().mockImplementation(async (sid: string) => {
        calls.push(`artifacts:${sid}`);
      }),
    });
    await registered.get("secafs.session.destroy")!({ sessionKey: "main:abc" });
    // transcript paths come from entry.sessionFile, so artifacts must be
    // collected/deleted BEFORE the store entries go away
    expect(calls[0]).toBe("artifacts:abc");
    expect(calls.slice(1)).toContain("delete:main:abc");
  });

  it("session.destroy ignores unmount errors (session may already be unmounted)", async () => {
    const h = makeHarness();
    h.rpc.unmount.mockRejectedValueOnce(new Error("not mounted"));
    const out = await h.registered.get("secafs.session.destroy")!({ sessionKey: "main:abc" });
    expect(h.rpc.destroy).toHaveBeenCalledWith({ conversationId: "abc" });
    expect(out).toEqual({ destroyed: true });
  });

  it("session.export collects workspace files + transcripts + manifest", async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "secafs-export-"));
    await fs.mkdir(path.join(root, "abc", "sub"), { recursive: true });
    await fs.writeFile(path.join(root, "abc", "AGENTS.md"), "hello agents");
    await fs.writeFile(path.join(root, "abc", "sub", "data.bin"), Buffer.from([0, 1, 2, 255]));
    const h = makeHarness({
      mountRoot: root,
      readTranscripts: vi.fn().mockResolvedValue({
        sessionJsonl: Buffer.from('{"type":"session"}\n').toString("base64"),
      }),
    });
    h.sessions.entries.mockResolvedValue({
      "main:abc": { alias: "my-sess", secafsRollback: { enabled: true } },
    });
    const out = (await h.registered.get("secafs.session.export")!({
      sessionKey: "main:abc",
    })) as SessionArchive;
    expect(h.rpc.mount).toHaveBeenCalledWith({
      conversationId: "abc",
      hostPath: path.join(root, "abc"),
    });
    expect(out.manifest).toMatchObject({
      schemaVersion: 1,
      sid: "abc",
      alias: "my-sess",
      rollbackEnabled: true,
      fileCount: 2,
    });
    const byPath = new Map(out.files.map((f) => [f.path, f.content]));
    expect(Buffer.from(byPath.get("AGENTS.md")!, "base64").toString()).toBe("hello agents");
    expect(Buffer.from(byPath.get("sub/data.bin")!, "base64")).toEqual(
      Buffer.from([0, 1, 2, 255]),
    );
    expect(out.chat.sessionJsonl).toBeDefined();
    await fs.rm(root, { recursive: true, force: true });
  });

  it("session.import creates a fresh session, writes files + transcripts, no seeding", async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "secafs-import-"));
    const writeTranscripts = vi.fn().mockResolvedValue(undefined);
    const ensureSessionEntry = vi.fn().mockResolvedValue(undefined);
    const h = makeHarness({
      mountRoot: root,
      writeTranscripts,
      ensureSessionEntry,
      // import must NOT seed from the default workspace
      defaultWorkspaceDir: "/nonexistent-should-not-be-read",
    });
    const archive: SessionArchive = {
      manifest: {
        schemaVersion: 1,
        sid: "old-sid",
        alias: "imported-alias",
        exportedAt: "2026-06-11T00:00:00Z",
        rollbackEnabled: true,
        fileCount: 1,
        totalBytes: 5,
      },
      files: [{ path: "notes/x.txt", content: Buffer.from("hello").toString("base64") }],
      chat: { sessionJsonl: Buffer.from("{}\n").toString("base64") },
    };
    const out = (await h.registered.get("secafs.session.import")!({ archive })) as {
      sessionKey: string;
      filesWritten: number;
      alias: string | null;
    };
    expect(out.sessionKey).toBe("main:abc"); // fresh session from sessions.create
    expect(out.filesWritten).toBe(1);
    expect(out.alias).toBe("imported-alias");
    const written = await fs.readFile(path.join(root, "abc", "notes", "x.txt"), "utf8");
    expect(written).toBe("hello");
    expect(writeTranscripts).toHaveBeenCalledWith("abc", archive.chat);
    // canonical pre-seed before workspace.set, marker on both forms
    expect(ensureSessionEntry).toHaveBeenCalled();
    expect(h.sessions.patch).toHaveBeenCalledWith(
      "main:abc",
      expect.objectContaining({ kind: "secafs", alias: "imported-alias" }),
    );
    expect(h.sessions.patch).toHaveBeenCalledWith(
      "agent:main:main:abc",
      expect.objectContaining({ kind: "secafs", alias: "imported-alias" }),
    );
    await fs.rm(root, { recursive: true, force: true });
  });

  it("session.import rejects path escapes and malformed archives", async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "secafs-import-evil-"));
    const h = makeHarness({ mountRoot: root });
    const evil: SessionArchive = {
      manifest: {
        schemaVersion: 1,
        sid: "x",
        alias: null,
        exportedAt: "",
        rollbackEnabled: false,
        fileCount: 1,
        totalBytes: 1,
      },
      files: [{ path: "../../escape.txt", content: Buffer.from("x").toString("base64") }],
      chat: {},
    };
    await expect(h.registered.get("secafs.session.import")!({ archive: evil })).rejects.toThrow();
    await expect(h.registered.get("secafs.session.import")!({ archive: { nope: 1 } })).rejects.toThrow(
      /invalid archive/,
    );
    await fs.rm(root, { recursive: true, force: true });
  });

  it("rollback.snapshot delegates to snapshotNow and is absent without the hook", async () => {
    const snapshotNow = vi
      .fn()
      .mockResolvedValue({ enabled: true, committed: true, snapId: 7, messageId: "m1" });
    const h = makeHarness({ snapshotNow });
    const out = await h.registered.get("secafs.rollback.snapshot")!({ sessionKey: "main:abc" });
    expect(snapshotNow).toHaveBeenCalledWith({ sessionKey: "main:abc" });
    expect(out).toEqual({ enabled: true, committed: true, snapId: 7, messageId: "m1" });
    const bare = makeHarness();
    expect(bare.registered.has("secafs.rollback.snapshot")).toBe(false);
  });

  it("session.create and session.open pre-seed the canonical entry BEFORE workspace.set", async () => {
    // Session patches are update-only: spawned* fields written by
    // workspace.set only land on the canonical key if its entry already
    // exists. Wrong order = the agent run silently falls back to the default
    // workspace instead of the FUSE mount (regression caught live).
    const order: string[] = [];
    const h = makeHarness({
      ensureSessionEntry: vi.fn().mockImplementation(async (key: string) => {
        order.push(`ensure:${key}`);
      }),
    });
    h.workspace.set.mockImplementation(async (key: string) => {
      order.push(`wsset:${key}`);
    });
    await h.registered.get("secafs.session.create")!({});
    expect(order.indexOf("ensure:agent:main:main:abc")).toBeGreaterThanOrEqual(0);
    expect(order.indexOf("ensure:agent:main:main:abc")).toBeLessThan(order.indexOf("wsset:main:abc"));
    order.length = 0;
    await h.registered.get("secafs.session.open")!({ sessionKey: "main:abc" });
    expect(order.indexOf("ensure:agent:main:main:abc")).toBeGreaterThanOrEqual(0);
    expect(order.indexOf("ensure:agent:main:main:abc")).toBeLessThan(order.indexOf("wsset:main:abc"));
  });

  it("session.create with alias patches kind+alias onto bare AND canonical keys", async () => {
    const h = makeHarness();
    const out = await h.registered.get("secafs.session.create")!({ alias: "  my project  " });
    expect(out).toMatchObject({ sessionKey: "main:abc", alias: "my project" });
    expect(h.sessions.patch).toHaveBeenCalledWith("main:abc", {
      kind: "secafs",
      alias: "my project",
    });
    expect(h.sessions.patch).toHaveBeenCalledWith("agent:main:main:abc", {
      kind: "secafs",
      alias: "my project",
    });
  });

  it("session.list merges store entries with daemon mount state and dedupes key forms", async () => {
    const h = makeHarness();
    h.sessions.entries.mockResolvedValueOnce({
      "main:s1": { sessionId: "s1", kind: "secafs", alias: "alpha", updatedAt: 100 },
      // canonical twin of s1 (post agent-run migration; marker lost, spawnedBy survives)
      "agent:main:main:s1": { spawnedBy: "agent:main:secafs-chat", lastInteractionAt: 300 },
      // canonical-only secafs session detected via spawnedWorkspaceDir under mountRoot
      "agent:main:main:s2": { spawnedWorkspaceDir: "/tmp/mnt/s2", updatedAt: 200 },
      // unrelated non-secafs session must be excluded
      "agent:main:main:other": { sessionId: "other", updatedAt: 999 },
    });
    h.rpc.list.mockResolvedValueOnce({
      mounts: [{ conversationId: "s1", hostPath: "/tmp/mnt/s1", since: "now" }],
    });
    const out = (await h.registered.get("secafs.session.list")!({})) as {
      daemonReachable: boolean;
      sessions: Array<Record<string, unknown>>;
    };
    expect(out.daemonReachable).toBe(true);
    expect(out.sessions).toHaveLength(2);
    expect(out.sessions[0]).toMatchObject({
      sid: "s1",
      sessionKey: "main:s1",
      alias: "alpha",
      mounted: true,
      hostPath: "/tmp/mnt/s1",
    });
    expect(out.sessions[1]).toMatchObject({ sid: "s2", alias: null, mounted: false });
  });

  it("session.list reports daemonReachable=false but still lists sessions when rpc.list fails", async () => {
    const h = makeHarness();
    h.sessions.entries.mockResolvedValueOnce({
      "main:s1": { sessionId: "s1", kind: "secafs" },
    });
    h.rpc.list.mockRejectedValueOnce(new Error("daemon down"));
    const out = (await h.registered.get("secafs.session.list")!({})) as {
      daemonReachable: boolean;
      sessions: Array<{ mounted: boolean }>;
    };
    expect(out.daemonReachable).toBe(false);
    expect(out.sessions).toHaveLength(1);
    expect(out.sessions[0].mounted).toBe(false);
  });

  it("session.rename patches alias onto every key form; empty alias tombstones with null", async () => {
    const h = makeHarness();
    await h.registered.get("secafs.session.rename")!({ sessionKey: "main:abc", alias: "newname" });
    expect(h.sessions.patch).toHaveBeenCalledWith("main:abc", { alias: "newname" });
    expect(h.sessions.patch).toHaveBeenCalledWith("agent:main:main:abc", { alias: "newname" });
    h.sessions.patch.mockClear();
    await h.registered.get("secafs.session.rename")!({ sessionKey: "main:abc", alias: "   " });
    expect(h.sessions.patch).toHaveBeenCalledWith("main:abc", { alias: null });
  });

  it("session.open resolves the same conversationId for bare and canonical keys", async () => {
    const h = makeHarness();
    await h.registered.get("secafs.session.open")!({ sessionKey: "main:abc" });
    await h.registered.get("secafs.session.open")!({ sessionKey: "agent:main:main:abc" });
    const calls = h.rpc.mount.mock.calls.map((c) => c[0]);
    expect(calls).toEqual([
      { conversationId: "abc", hostPath: "/tmp/mnt/abc" },
      { conversationId: "abc", hostPath: "/tmp/mnt/abc" },
    ]);
  });

  it("session.close resolves the same conversationId for bare and canonical keys", async () => {
    const h = makeHarness();
    await h.registered.get("secafs.session.close")!({ sessionKey: "main:abc" });
    await h.registered.get("secafs.session.close")!({ sessionKey: "agent:main:main:abc" });
    const calls = h.rpc.unmount.mock.calls.map((c) => c[0]);
    expect(calls).toEqual([{ conversationId: "abc" }, { conversationId: "abc" }]);
  });

  it("session.destroy resolves the same conversationId for bare and canonical keys", async () => {
    const h = makeHarness();
    await h.registered.get("secafs.session.destroy")!({ sessionKey: "agent:main:main:xyz" });
    expect(h.rpc.unmount).toHaveBeenCalledWith({ conversationId: "xyz" });
    expect(h.rpc.destroy).toHaveBeenCalledWith({ conversationId: "xyz" });
  });

  it("session.destroy uses configured mainKey/defaultAgentId for non-default setups", async () => {
    const rpc = {
      ping: vi.fn(),
      mount: vi.fn(),
      unmount: vi.fn().mockResolvedValue({ unmounted: true }),
      destroy: vi.fn().mockResolvedValue({ destroyed: true }),
      snapshotEnable: vi.fn(),
      snapshotDisable: vi.fn(),
      snapshotCommit: vi.fn(),
      snapshotList: vi.fn(),
      snapshotRestore: vi.fn(),
    };
    const sessions = {
      create: vi.fn(),
      patch: vi.fn().mockResolvedValue(undefined),
      load: vi.fn(),
      delete: vi.fn().mockResolvedValue(undefined),
    };
    const workspace = {
      set: vi.fn().mockResolvedValue(undefined),
      clear: vi.fn().mockResolvedValue(undefined),
    };
    const registered = new Map<string, (params: unknown) => Promise<unknown>>();
    registerGatewayMethods({
      registerGatewayMethod: (m, h) => {
        registered.set(m, h);
      },
      rpc,
      sessions,
      workspace,
      mountRoot: "/tmp/mnt",
      mainKey: "primary",
      defaultAgentId: "work",
      handleRestore: vi.fn(),
    });
    await registered.get("secafs.session.destroy")!({ sessionKey: "agent:work:primary:abc" });
    // Both forms should be cleaned up using the configured prefixes.
    expect(workspace.clear).toHaveBeenCalledWith("primary:abc");
    expect(workspace.clear).toHaveBeenCalledWith("agent:work:primary:abc");
    expect(sessions.delete).toHaveBeenCalledWith("primary:abc");
    expect(sessions.delete).toHaveBeenCalledWith("agent:work:primary:abc");
  });
});

describe("rollback gateway methods", () => {
  function makeOpts(overrides: Partial<RegisterGatewayMethodsOpts> = {}) {
    const registered: Record<string, (params: unknown) => Promise<unknown>> = {};
    const baseRpc = {
      ping: vi.fn(async () => ({ version: "test", pgConnected: true, mountCount: 0 })),
      mount: vi.fn(async () => ({ hostPath: "/m", mounted: true })),
      unmount: vi.fn(async () => ({ unmounted: true })),
      destroy: vi.fn(async () => ({ destroyed: true })),
      snapshotEnable: vi.fn(async () => ({ enabled: true, currentSnapId: 1 })),
      snapshotDisable: vi.fn(async () => ({
        disabled: true,
        purgedSnapshots: 4,
        purgedUndoRows: 12,
      })),
      snapshotCommit: vi.fn(),
      snapshotList: vi.fn(async () => ({
        snapshots: [{ snapId: 1, label: "m1", committedAt: "t" }],
      })),
      snapshotRestore: vi.fn(),
    };
    const opts: RegisterGatewayMethodsOpts = {
      registerGatewayMethod: (name, handler) => {
        registered[name] = handler;
      },
      rpc: baseRpc,
      sessions: {
        create: vi.fn(),
        patch: vi.fn(),
        load: vi.fn(),
        delete: vi.fn(),
      },
      workspace: { set: vi.fn(), clear: vi.fn() },
      mountRoot: "/m",
      handleRestore: vi.fn(async () => ({
        restored: true as const,
        restoredMessageId: "m1",
        prunedSnapshots: 0,
        prunedUndoRows: 0,
      })),
      ...overrides,
    };
    registerGatewayMethods(opts);
    return { registered, opts };
  }

  it("registers the three rollback methods", () => {
    const { registered } = makeOpts();
    expect(Object.keys(registered)).toEqual(
      expect.arrayContaining([
        "secafs.rollback.setEnabled",
        "secafs.rollback.list",
        "secafs.rollback.restore",
      ]),
    );
  });

  it("setEnabled(true) calls snapshotEnable AND persists secafsRollback.enabled to BOTH key forms", async () => {
    const { registered, opts } = makeOpts();
    const r = (await registered["secafs.rollback.setEnabled"]({
      sessionKey: "main:abc",
      enabled: true,
    })) as { enabled: boolean };
    expect(opts.rpc.snapshotEnable).toHaveBeenCalledWith({ conversationId: "abc" });
    expect(r.enabled).toBe(true);
    // Sessions are tracked under bare and canonical key forms; the patch must
    // hit both so the snapshot hook and rollback.list see consistent state
    // regardless of which form the agent runtime / UI provides.
    expect(opts.sessions.patch).toHaveBeenCalledWith("main:abc", {
      secafsRollback: expect.objectContaining({ enabled: true }),
    });
    expect(opts.sessions.patch).toHaveBeenCalledWith("agent:main:main:abc", {
      secafsRollback: expect.objectContaining({ enabled: true }),
    });
  });

  it("setEnabled(false) calls snapshotDisable, surfaces purge counts, and clears flag on BOTH key forms", async () => {
    const { registered, opts } = makeOpts();
    const r = (await registered["secafs.rollback.setEnabled"]({
      sessionKey: "main:abc",
      enabled: false,
    })) as { enabled: boolean; purgedSnapshots: number };
    expect(opts.rpc.snapshotDisable).toHaveBeenCalledWith({ conversationId: "abc" });
    expect(r.enabled).toBe(false);
    expect(r.purgedSnapshots).toBe(4);
    expect(opts.sessions.patch).toHaveBeenCalledWith("main:abc", {
      secafsRollback: { enabled: false },
    });
    expect(opts.sessions.patch).toHaveBeenCalledWith("agent:main:main:abc", {
      secafsRollback: { enabled: false },
    });
  });

  it("list returns snapshots with label remapped to messageId", async () => {
    const { registered } = makeOpts();
    const r = (await registered["secafs.rollback.list"]({ sessionKey: "main:abc" })) as {
      snapshots: Array<{ messageId: string }>;
    };
    expect(r.snapshots).toEqual([{ snapId: 1, messageId: "m1", committedAt: "t" }]);
  });

  it("restore delegates to handleRestore", async () => {
    const { registered, opts } = makeOpts();
    const r = (await registered["secafs.rollback.restore"]({
      sessionKey: "main:abc",
      snapId: 5,
    })) as { restored: boolean };
    expect(opts.handleRestore).toHaveBeenCalledWith({ sessionKey: "main:abc", snapId: 5 });
    expect(r.restored).toBe(true);
  });
});
