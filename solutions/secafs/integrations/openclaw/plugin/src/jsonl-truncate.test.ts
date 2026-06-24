import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { describe, expect, it, beforeEach, afterEach } from "vitest";
import { truncateJsonlAfterMessage } from "./jsonl-truncate.js";

let tmpDir: string;
beforeEach(async () => {
  tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), "jsonl-trunc-"));
});
afterEach(async () => {
  await fs.rm(tmpDir, { recursive: true, force: true });
});

async function write(file: string, content: string): Promise<void> {
  await fs.writeFile(file, content, "utf8");
}
async function read(file: string): Promise<string> {
  return fs.readFile(file, "utf8");
}

describe("truncateJsonlAfterMessage", () => {
  it("truncates everything after the line containing the target message id", async () => {
    const file = path.join(tmpDir, "s.jsonl");
    const lines = [
      `{"type":"user_message","id":"m1","text":"hi"}`,
      `{"type":"assistant_message","id":"m2","text":"hello"}`,
      `{"type":"user_message","id":"m3","text":"more"}`,
      `{"type":"assistant_message","id":"m4","text":"reply"}`,
    ];
    await write(file, lines.join("\n") + "\n");
    const result = await truncateJsonlAfterMessage(file, "m2");
    expect(result.truncated).toBe(true);
    const after = await read(file);
    expect(after).toBe(lines.slice(0, 2).join("\n") + "\n");
  });

  it("is a no-op when the target id is the last line", async () => {
    const file = path.join(tmpDir, "s.jsonl");
    const lines = [`{"id":"m1"}`, `{"id":"m2"}`];
    await write(file, lines.join("\n") + "\n");
    const result = await truncateJsonlAfterMessage(file, "m2");
    expect(result.truncated).toBe(false);
    const after = await read(file);
    expect(after).toBe(lines.join("\n") + "\n");
  });

  it("throws if the message id is not found", async () => {
    const file = path.join(tmpDir, "s.jsonl");
    await write(file, `{"id":"m1"}\n`);
    await expect(truncateJsonlAfterMessage(file, "missing")).rejects.toThrow(/not found/);
  });

  it("handles a file without trailing newline", async () => {
    const file = path.join(tmpDir, "s.jsonl");
    await write(file, `{"id":"m1"}\n{"id":"m2"}`);
    const result = await truncateJsonlAfterMessage(file, "m1");
    expect(result.truncated).toBe(true);
    expect(await read(file)).toBe(`{"id":"m1"}\n`);
  });

  it("handles missing file by treating message as already truncated", async () => {
    const file = path.join(tmpDir, "missing.jsonl");
    const result = await truncateJsonlAfterMessage(file, "m1", { missingOk: true });
    expect(result.truncated).toBe(false);
    expect(result.fileExisted).toBe(false);
    expect(result.idFound).toBe(false);
  });

  it("treats id-not-found as no-op when idNotFoundOk is true", async () => {
    const file = path.join(tmpDir, "s.jsonl");
    const lines = [`{"id":"m1"}`, `{"id":"m2"}`];
    await write(file, lines.join("\n") + "\n");
    const result = await truncateJsonlAfterMessage(file, "missing", { idNotFoundOk: true });
    expect(result.fileExisted).toBe(true);
    expect(result.truncated).toBe(false);
    expect(result.idFound).toBe(false);
    expect(await read(file)).toBe(lines.join("\n") + "\n");
  });
});
