import fs from "node:fs/promises";
import path from "node:path";

/**
 * File read/write for the standalone SecAFS-first frontend.
 *
 * The secafs daemon RPC only manages FUSE *mounts* (mount/unmount/list/destroy);
 * file contents live on the host FUSE mount at `<mountRoot>/<conversationId>`.
 * The plugin runs inside the gateway ON THAT HOST, so it can serve file
 * contents to a remote frontend with plain `node:fs` against the mount path.
 * These back the `secafs.fs.read` / `secafs.fs.write` gateway methods.
 */

/**
 * Resolve a per-conversation relative path to an absolute path under the
 * mounted volume root, rejecting anything that escapes the mount (`..`,
 * absolute-looking inputs). Returns the safe absolute path or throws.
 */
export function resolveMountedPath(mountRoot: string, sid: string, relPath: string): string {
  if (!sid) {
    throw new Error("empty conversation id");
  }
  const root = path.resolve(mountRoot, sid);
  // Treat input as relative to the volume root; strip leading separators so an
  // absolute-looking path cannot jump out of the root.
  const cleaned = String(relPath ?? "").replace(/^[/\\]+/, "");
  const abs = path.resolve(root, cleaned);
  const rootWithSep = root.endsWith(path.sep) ? root : root + path.sep;
  if (abs !== root && !abs.startsWith(rootWithSep)) {
    throw new Error(`path escapes volume: ${relPath}`);
  }
  return abs;
}

export interface FsReadResult {
  path: string;
  encoding: "utf8" | "base64";
  content: string;
  size: number;
}

const DEFAULT_MAX_BYTES = 1_048_576; // 1 MiB
const MAX_MAX_BYTES = 16_777_216; // 16 MiB hard cap

export async function readSecafsFile(
  mountRoot: string,
  sid: string,
  relPath: string,
  opts: { maxBytes?: number; encoding?: "utf8" | "base64" } = {},
): Promise<FsReadResult> {
  const abs = resolveMountedPath(mountRoot, sid, relPath);
  const maxBytes = Math.max(1, Math.min(opts.maxBytes ?? DEFAULT_MAX_BYTES, MAX_MAX_BYTES));
  const buf = await fs.readFile(abs);
  if (buf.byteLength > maxBytes) {
    throw new Error(`file too large: ${buf.byteLength} > ${maxBytes} bytes`);
  }
  const encoding = opts.encoding ?? "utf8";
  return { path: relPath, encoding, content: buf.toString(encoding), size: buf.byteLength };
}

export async function writeSecafsFile(
  mountRoot: string,
  sid: string,
  relPath: string,
  content: string,
  opts: { encoding?: "utf8" | "base64" } = {},
): Promise<{ path: string; bytesWritten: number }> {
  const abs = resolveMountedPath(mountRoot, sid, relPath);
  await fs.mkdir(path.dirname(abs), { recursive: true });
  const buf = Buffer.from(content ?? "", opts.encoding ?? "utf8");
  await fs.writeFile(abs, buf);
  return { path: relPath, bytesWritten: buf.byteLength };
}
