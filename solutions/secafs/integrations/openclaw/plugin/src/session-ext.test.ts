import { describe, expect, it } from "vitest";
import {
  SECAFS_PLUGIN_ID,
  foldSecafsExt,
  foldSecafsStore,
  hoistSecafsExt,
  hoistSecafsStore,
} from "./session-ext.js";

describe("session-ext codec", () => {
  it("fold moves custom fields into pluginExtensions and strips top-level", () => {
    const folded = foldSecafsExt({
      sessionId: "s",
      updatedAt: 1,
      spawnedCwd: "/mnt/x", // upstream core field — must stay top-level
      kind: "secafs",
      mountState: "mounted",
      secafsRollback: { enabled: true },
    }) as Record<string, unknown>;
    expect("kind" in folded).toBe(false);
    expect("mountState" in folded).toBe(false);
    expect("secafsRollback" in folded).toBe(false);
    expect(folded.spawnedCwd).toBe("/mnt/x"); // core field untouched
    expect((folded.pluginExtensions as Record<string, unknown>)[SECAFS_PLUGIN_ID]).toEqual({
      kind: "secafs",
      mountState: "mounted",
      secafsRollback: { enabled: true },
    });
  });

  it("hoist surfaces namespaced fields back to top-level", () => {
    const hoisted = hoistSecafsExt({
      sessionId: "s",
      updatedAt: 1,
      pluginExtensions: { [SECAFS_PLUGIN_ID]: { kind: "secafs", mountState: "unmounted" } },
    }) as Record<string, unknown>;
    expect(hoisted.kind).toBe("secafs");
    expect(hoisted.mountState).toBe("unmounted");
  });

  it("round-trips (fold then hoist) preserving values", () => {
    const original = { sessionId: "s", updatedAt: 1, kind: "secafs", secafsRollback: { enabled: false } };
    const back = hoistSecafsExt(foldSecafsExt(original)) as Record<string, unknown>;
    expect(back.kind).toBe("secafs");
    expect(back.secafsRollback).toEqual({ enabled: false });
  });

  it("hoist falls back to legacy top-level fields (pre-migration stores)", () => {
    // No pluginExtensions namespace, but legacy top-level kind present.
    const legacy = { sessionId: "s", updatedAt: 1, kind: "secafs" };
    expect((hoistSecafsExt(legacy) as Record<string, unknown>).kind).toBe("secafs");
  });

  it("fold migrates a legacy top-level store into the namespace", () => {
    const store = { "main:a": { sessionId: "a", updatedAt: 1, kind: "secafs" } };
    const folded = foldSecafsStore(store) as Record<string, Record<string, unknown>>;
    expect("kind" in folded["main:a"]).toBe(false);
    expect((folded["main:a"].pluginExtensions as Record<string, unknown>)[SECAFS_PLUGIN_ID]).toEqual({ kind: "secafs" });
  });

  it("no-op for entries without any secafs state", () => {
    const entry = { sessionId: "s", updatedAt: 1, spawnedCwd: "/x" };
    expect(foldSecafsExt(entry)).toBe(entry);
    expect(hoistSecafsExt(entry)).toBe(entry);
  });

  it("hoistStore is symmetric over multiple entries", () => {
    const store = {
      a: { sessionId: "a", updatedAt: 1, pluginExtensions: { [SECAFS_PLUGIN_ID]: { kind: "secafs" } } },
      b: { sessionId: "b", updatedAt: 1 },
    };
    const h = hoistSecafsStore(store) as Record<string, Record<string, unknown>>;
    expect(h.a.kind).toBe("secafs");
    expect("kind" in h.b).toBe(false);
  });
});
