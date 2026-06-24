import { createReadStream, createWriteStream } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";

export interface TruncateOptions {
  /** When true, treat ENOENT as "already truncated" and return non-throwing result. */
  missingOk?: boolean;
  /**
   * When true, treat "message id not found in file" as a no-op rather than
   * throwing. Useful for derived/auxiliary files (like trajectory.jsonl) whose
   * id space does not match the chat session.jsonl.
   */
  idNotFoundOk?: boolean;
}

export interface TruncateResult {
  fileExisted: boolean;
  truncated: boolean;
  bytesAfter: number;
  idFound: boolean;
}

/**
 * Truncate a JSONL file in place so that everything after the *last* line
 * whose JSON object has `id === messageId` is removed. The line containing
 * the message id is preserved.
 *
 * Atomic on POSIX via tempfile + rename.
 */
export async function truncateJsonlAfterMessage(
  file: string,
  messageId: string,
  opts: TruncateOptions = {},
): Promise<TruncateResult> {
  let stat;
  try {
    stat = await fs.stat(file);
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT" && opts.missingOk) {
      return { fileExisted: false, truncated: false, bytesAfter: 0, idFound: false };
    }
    throw e;
  }

  const totalSize = stat.size;
  const offset = await findLastMessageOffset(file, messageId);
  if (offset === null) {
    if (opts.idNotFoundOk) {
      return { fileExisted: true, truncated: false, bytesAfter: totalSize, idFound: false };
    }
    throw new Error(`message id "${messageId}" not found in ${file}`);
  }

  if (offset >= totalSize) {
    return { fileExisted: true, truncated: false, bytesAfter: totalSize, idFound: true };
  }

  // Atomic write: tempfile + rename.
  const tempPath = path.join(path.dirname(file), `.${path.basename(file)}.${process.pid}.tmp`);
  await new Promise<void>((resolve, reject) => {
    const reader = createReadStream(file, { start: 0, end: offset - 1 });
    const writer = createWriteStream(tempPath);
    reader.on("error", reject);
    writer.on("error", reject);
    writer.on("close", resolve);
    reader.pipe(writer);
  });
  await fs.rename(tempPath, file);
  return { fileExisted: true, truncated: true, bytesAfter: offset, idFound: true };
}

async function findLastMessageOffset(file: string, messageId: string): Promise<number | null> {
  // Read the entire file as a buffer so we can do exact byte-level scanning.
  // This avoids all readline/encoding edge cases with trailing newlines.
  const buf = await fs.readFile(file);
  const totalSize = buf.length;

  let lastEnd: number | null = null;
  let lineStart = 0;

  while (lineStart <= totalSize) {
    // Find the next \n
    let newlineIdx = buf.indexOf(0x0a, lineStart); // 0x0a === '\n'
    let lineEnd: number; // exclusive index of the byte after the line (including \n if present)

    if (newlineIdx === -1) {
      // No more newlines — this is the last line (possibly empty if lineStart === totalSize)
      if (lineStart === totalSize) {
        break;
      } // nothing left to parse
      newlineIdx = totalSize; // treat end-of-file as line boundary
      lineEnd = totalSize;
    } else {
      lineEnd = newlineIdx + 1; // include the \n byte
    }

    const lineBytes = buf.slice(lineStart, newlineIdx); // excludes \n
    if (lineBytes.length > 0) {
      try {
        const obj = JSON.parse(lineBytes.toString("utf8")) as { id?: unknown };
        if (typeof obj.id === "string" && obj.id === messageId) {
          lastEnd = lineEnd;
        }
      } catch {
        // skip non-JSON lines
      }
    }

    lineStart = lineEnd;
    if (newlineIdx === totalSize) {
      break;
    } // was last line with no \n
  }

  return lastEnd;
}
