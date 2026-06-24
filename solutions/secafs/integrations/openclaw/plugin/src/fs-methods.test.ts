import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { readSecafsFile, resolveMountedPath, writeSecafsFile } from "./fs-methods.js";

describe("resolveMountedPath", () => {
  const root = "/mnt/secafs";
  it("resolves a normal relative path under the volume", () => {
    expect(resolveMountedPath(root, "abc", "dir/file.txt")).toBe("/mnt/secafs/abc/dir/file.txt");
  });
  it("strips leading slashes so absolute-looking paths stay inside", () => {
    expect(resolveMountedPath(root, "abc", "/etc/passwd")).toBe("/mnt/secafs/abc/etc/passwd");
  });
  it("rejects ../ traversal that escapes the volume", () => {
    expect(() => resolveMountedPath(root, "abc", "../../etc/passwd")).toThrow(/escapes volume/);
  });
  it("rejects an empty conversation id", () => {
    expect(() => resolveMountedPath(root, "", "x")).toThrow(/empty conversation id/);
  });
  it("allows the volume root itself", () => {
    expect(resolveMountedPath(root, "abc", "")).toBe("/mnt/secafs/abc");
  });
});

describe("read/write round-trip", () => {
  let mountRoot: string;
  beforeAll(async () => {
    mountRoot = await mkdtemp(path.join(tmpdir(), "secafs-fs-test-"));
  });
  afterAll(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(mountRoot, { recursive: true, force: true });
  });

  it("writes then reads a utf8 file (creating parent dirs)", async () => {
    const w = await writeSecafsFile(mountRoot, "vol1", "notes/hello.txt", "hi from secafs");
    expect(w.bytesWritten).toBe(14);
    const r = await readSecafsFile(mountRoot, "vol1", "notes/hello.txt");
    expect(r.content).toBe("hi from secafs");
    expect(r.encoding).toBe("utf8");
    expect(r.size).toBe(14);
    // confirm it actually landed under the volume root
    const onDisk = await readFile(path.join(mountRoot, "vol1", "notes/hello.txt"), "utf8");
    expect(onDisk).toBe("hi from secafs");
  });

  it("supports base64 round-trip", async () => {
    const b64 = Buffer.from([0, 1, 2, 255]).toString("base64");
    await writeSecafsFile(mountRoot, "vol1", "bin.dat", b64, { encoding: "base64" });
    const r = await readSecafsFile(mountRoot, "vol1", "bin.dat", { encoding: "base64" });
    expect(r.content).toBe(b64);
  });

  it("enforces maxBytes on read", async () => {
    await writeSecafsFile(mountRoot, "vol1", "big.txt", "x".repeat(100));
    await expect(readSecafsFile(mountRoot, "vol1", "big.txt", { maxBytes: 10 })).rejects.toThrow(
      /too large/,
    );
  });

  it("refuses to write outside the volume", async () => {
    await expect(writeSecafsFile(mountRoot, "vol1", "../escape.txt", "x")).rejects.toThrow(
      /escapes volume/,
    );
  });
});
