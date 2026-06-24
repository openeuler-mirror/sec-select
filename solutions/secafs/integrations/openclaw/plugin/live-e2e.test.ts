import { execFile } from "node:child_process";
import { readFile, stat, writeFile } from "node:fs/promises";
import { connect } from "node:net";
import path from "node:path";
import { promisify } from "node:util";
import { describe, expect, it } from "vitest";

const execFileAsync = promisify(execFile);

const LIVE = process.env.OPENCLAW_LIVE_TEST === "1";

interface CreateResult {
  sessionKey: string;
  hostPath: string;
}

async function runSecafsCli(args: string[]): Promise<unknown> {
  const repoRoot = path.resolve(__dirname, "../..");
  const { stdout } = await execFileAsync("node", ["openclaw.mjs", "secafs", ...args], {
    cwd: repoRoot,
  });
  return JSON.parse(stdout);
}

async function runGatewayCall(
  method: string,
  params: Record<string, unknown> = {},
): Promise<unknown> {
  const repoRoot = path.resolve(__dirname, "../..");
  // Unset VITEST from the child process env: the openclaw runtime suppresses
  // stdout writes when VITEST=true is set (to avoid noisy test output), but
  // the gateway call CLI is a real child process that needs its stdout output.
  const { VITEST: _vitest, ...childEnv } = process.env;
  const { stdout } = await execFileAsync(
    "node",
    ["openclaw.mjs", "gateway", "call", "--json", "--params", JSON.stringify(params), method],
    { cwd: repoRoot, env: childEnv },
  );
  // The gateway CLI may emit non-JSON status lines before the JSON; slice from
  // the first `{` to last `}` to isolate the JSON payload.
  const trimmed = stdout.trim();
  try {
    return JSON.parse(trimmed);
  } catch {
    const start = trimmed.indexOf("{");
    const end = trimmed.lastIndexOf("}");
    if (start !== -1 && end !== -1) {
      return JSON.parse(trimmed.slice(start, end + 1));
    }
    throw new Error(`Failed to parse gateway call output: ${trimmed}`);
  }
}

/**
 * Call a method on the SecAFS daemon directly via its Unix socket.
 * Used for snapshot operations (secafs.v1.snapshot.*) which are daemon-internal
 * RPC methods and are not exposed as gateway-level methods.
 */
async function runDaemonRpc(
  method: string,
  params: Record<string, unknown> = {},
  socketPath = `/run/user/${process.getuid?.() ?? 1001}/secafs/secafs.sock`,
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const socket = connect(socketPath);
    let buf = "";
    const timeoutHandle = setTimeout(() => {
      socket.destroy();
      reject(new Error(`Daemon RPC timeout: ${method}`));
    }, 10_000);

    socket.setEncoding("utf8");
    socket.on("connect", () => {
      const req = JSON.stringify({ jsonrpc: "2.0", id: 1, method, params });
      socket.write(req + "\n");
    });
    socket.on("data", (chunk: string) => {
      buf += chunk;
      const nl = buf.indexOf("\n");
      if (nl === -1) {
        return;
      }
      const line = buf.slice(0, nl);
      clearTimeout(timeoutHandle);
      socket.destroy();
      try {
        const msg = JSON.parse(line) as { result?: unknown; error?: { message: string } };
        if (msg.error) {
          reject(new Error(msg.error.message));
        } else {
          resolve(msg.result);
        }
      } catch (e) {
        reject(e);
      }
    });
    socket.on("error", (err) => {
      clearTimeout(timeoutHandle);
      reject(err);
    });
  });
}

describe.runIf(LIVE)("secafs-chat live e2e", () => {
  it("status returns daemonReachable", async () => {
    const status = (await runSecafsCli(["status"])) as { daemonReachable: boolean };
    expect(status.daemonReachable).toBe(true);
  });

  it("create → write file → destroy round trip", async () => {
    const created = (await runSecafsCli(["create"])) as CreateResult;
    expect(created.sessionKey).toMatch(/^main:/);
    expect(created.hostPath).toBeTruthy();

    // Wait briefly for FUSE mount to settle
    await new Promise((r) => setTimeout(r, 500));

    // Write a file inside the mount
    const filePath = path.join(created.hostPath, "hello.txt");
    await writeFile(filePath, "hi from live e2e");
    const back = await readFile(filePath, "utf8");
    expect(back).toBe("hi from live e2e");

    // Destroy
    const destroyed = (await runSecafsCli(["destroy", created.sessionKey])) as {
      destroyed: boolean;
    };
    expect(destroyed.destroyed).toBe(true);

    // Mount path should be unmounted; we can't reliably assert dir state
    // (it may persist as an empty directory). Just verify a fresh write
    // after destroy doesn't see the old file.
    try {
      const post = await stat(filePath);
      // If stat succeeds, the file should NOT have the old content
      // (in case the dir still exists post-unmount).
      expect(post.size).not.toBe(17);
    } catch {
      // ENOENT is the expected path
    }
  });

  it("create → close → reopen preserves data", async () => {
    const created = (await runSecafsCli(["create"])) as CreateResult;
    await new Promise((r) => setTimeout(r, 500));
    const filePath = path.join(created.hostPath, "persist.txt");
    await writeFile(filePath, "preserved");

    await runSecafsCli(["close", created.sessionKey]);
    await new Promise((r) => setTimeout(r, 500));

    const reopened = (await runSecafsCli(["open", created.sessionKey])) as {
      hostPath: string;
    };
    await new Promise((r) => setTimeout(r, 500));
    const back = await readFile(path.join(reopened.hostPath, "persist.txt"), "utf8");
    expect(back).toBe("preserved");

    // Cleanup
    await runSecafsCli(["destroy", created.sessionKey]);
  });

  it("rollback round-trip: enable → snapshot → mutate → restore", async () => {
    // Create a fresh secafs session via existing CLI helper.
    const created = (await runSecafsCli(["create"])) as CreateResult;
    await new Promise((r) => setTimeout(r, 500));

    try {
      // Enable rollback for this session.
      const enable = (await runGatewayCall("secafs.rollback.setEnabled", {
        sessionKey: created.sessionKey,
        enabled: true,
      })) as { enabled: boolean };
      expect(enable.enabled).toBe(true);

      // Write file v1 to the FUSE mount.
      const fileA = path.join(created.hostPath, "rollback-test.txt");
      await writeFile(fileA, "v1");
      expect(await readFile(fileA, "utf8")).toBe("v1");

      // Commit a snapshot via the daemon RPC directly. The gateway exposes
      // secafs.rollback.* but does not expose the raw secafs.v1.snapshot.*
      // daemon methods — those are internal RPC only.
      const conv = created.sessionKey.split(":").findLast(Boolean) ?? "";
      const commit1 = (await runDaemonRpc("secafs.v1.snapshot.commit", {
        conversationId: conv,
        label: "asst-1",
      })) as { snapId: number };
      expect(commit1.snapId).toBeGreaterThan(0);
      const baselineSnapId = commit1.snapId;

      // Mutate: change file content + add a new file.
      await writeFile(fileA, "v2");
      const fileB = path.join(created.hostPath, "rollback-extra.txt");
      await writeFile(fileB, "extra");

      // Commit snapshot 2.
      await runDaemonRpc("secafs.v1.snapshot.commit", {
        conversationId: conv,
        label: "asst-2",
      });

      // List snapshots to verify both are present.
      const list = (await runGatewayCall("secafs.rollback.list", {
        sessionKey: created.sessionKey,
      })) as { snapshots: Array<{ snapId: number; messageId: string }> };
      expect(list.snapshots.length).toBeGreaterThanOrEqual(2);

      // Restore to baseline via daemon RPC directly. The gateway's
      // secafs.rollback.restore also performs JSONL truncation which requires a
      // matching JSONL session file; since this test does not use the chat
      // pipeline, the JSONL path is absent and would cause an error. Calling
      // the daemon's restore RPC directly avoids the JSONL truncation step.
      const restore = (await runDaemonRpc("secafs.v1.snapshot.restore", {
        conversationId: conv,
        snapId: baselineSnapId,
      })) as { restored: boolean };
      expect(restore.restored).toBe(true);

      // The FUSE mount's inode cache may not invalidate automatically without
      // unmount/remount; close + reopen the mount to force re-read.
      await runSecafsCli(["close", created.sessionKey]);
      await new Promise((r) => setTimeout(r, 500));
      const reopened = (await runSecafsCli(["open", created.sessionKey])) as { hostPath: string };
      await new Promise((r) => setTimeout(r, 500));

      // Verify rollback: fileA = "v1", fileB does NOT exist.
      const restoredA = await readFile(path.join(reopened.hostPath, "rollback-test.txt"), "utf8");
      expect(restoredA).toBe("v1");
      let fileBExists = true;
      try {
        await stat(path.join(reopened.hostPath, "rollback-extra.txt"));
      } catch {
        fileBExists = false;
      }
      expect(fileBExists).toBe(false);
    } finally {
      await runSecafsCli(["destroy", created.sessionKey]);
    }
  }, 60_000);
});
