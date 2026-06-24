import { type Socket, connect } from "node:net";

export class SecafsRpcError extends Error {
  constructor(
    public readonly code: number,
    message: string,
    public readonly data?: unknown,
  ) {
    super(message);
    this.name = "SecafsRpcError";
  }
}

export interface PingResult {
  version: string;
  pgConnected: boolean;
  mountCount: number;
}

export interface MountResult {
  hostPath: string;
  mounted: boolean;
}

export interface UnmountResult {
  unmounted: boolean;
}

export interface DestroyResult {
  destroyed: boolean;
}

export interface ListEntry {
  conversationId: string;
  hostPath: string;
  since: string;
}

export interface ListResult {
  mounts: ListEntry[];
}

export interface SnapshotInfo {
  snapId: number;
  committedAt: string;
  label: string | null;
}

export interface SnapshotEnableResult {
  enabled: boolean;
  currentSnapId: number;
}

export interface SnapshotDisableResult {
  disabled: boolean;
  purgedSnapshots: number;
  purgedUndoRows: number;
}

export interface SnapshotCommitResult {
  snapId: number;
  committedAt: string;
  label: string | null;
}

export interface SnapshotListResult {
  snapshots: SnapshotInfo[];
}

export interface SnapshotRestoreResult {
  restored: boolean;
  prunedSnapshots: number;
  prunedUndoRows: number;
}

export interface SecafsRpcClient {
  call<T = unknown>(method: string, params: Record<string, unknown>): Promise<T>;
  ping(): Promise<PingResult>;
  mount(p: { conversationId: string; hostPath?: string }): Promise<MountResult>;
  unmount(p: { conversationId: string }): Promise<UnmountResult>;
  list(): Promise<ListResult>;
  destroy(p: { conversationId: string }): Promise<DestroyResult>;
  snapshotEnable(p: { conversationId: string }): Promise<SnapshotEnableResult>;
  snapshotDisable(p: { conversationId: string }): Promise<SnapshotDisableResult>;
  snapshotCommit(p: { conversationId: string; label?: string }): Promise<SnapshotCommitResult>;
  snapshotList(p: { conversationId: string }): Promise<SnapshotListResult>;
  snapshotRestore(p: { conversationId: string; snapId: number }): Promise<SnapshotRestoreResult>;
  close(): void;
}

export interface CreateSecafsRpcClientOpts {
  socketPath: string;
  /** Per-call response deadline. */
  timeoutMs?: number;
  /** Backoff delays (ms) for retrying socket connect on transient failures
   *  such as a daemon restart. Default: `[0, 200, 500, 1500]` (~2.2s window).
   *  Pass `[0]` to disable retries (single attempt). */
  connectBackoffsMs?: readonly number[];
}

export function createSecafsRpcClient(opts: CreateSecafsRpcClientOpts): SecafsRpcClient {
  const timeoutMs = opts.timeoutMs ?? 10_000;
  const connectBackoffsMs = opts.connectBackoffsMs ?? [0, 200, 500, 1500];
  let nextId = 1;
  const pending = new Map<
    number,
    {
      resolve: (v: unknown) => void;
      reject: (e: unknown) => void;
      timer: NodeJS.Timeout;
    }
  >();
  let socket: Socket | null = null;
  let connectingPromise: Promise<Socket> | null = null;
  let buf = "";
  let closed = false;

  function attachHandlers(s: Socket): void {
    s.setEncoding("utf8");
    s.on("data", (chunk: string) => {
      buf += chunk;
      let nl;
      while ((nl = buf.indexOf("\n")) !== -1) {
        const line = buf.slice(0, nl);
        buf = buf.slice(nl + 1);
        if (!line) {
          continue;
        }
        let msg: {
          id?: number;
          result?: unknown;
          error?: { code: number; message: string; data?: unknown };
        };
        try {
          msg = JSON.parse(line);
        } catch {
          continue;
        }
        if (typeof msg.id !== "number") {
          continue;
        }
        const entry = pending.get(msg.id);
        if (!entry) {
          continue;
        }
        pending.delete(msg.id);
        clearTimeout(entry.timer);
        if (msg.error) {
          entry.reject(new SecafsRpcError(msg.error.code, msg.error.message, msg.error.data));
        } else {
          entry.resolve(msg.result);
        }
      }
    });
    s.on("error", () => {
      // Failures are reported per-call via the request timer or via the close
      // handler. Don't double-reject pending here because the close handler
      // (which runs after error) handles it consistently.
    });
    s.on("close", () => {
      if (s === socket) {
        socket = null;
      }
      buf = "";
      // Don't reject pending requests here — the per-call timer will fire
      // and ensureSocket() will reconnect on the next call. Pending writes
      // that the kernel buffered before close will time out naturally.
      // Clearing pending here would race with the connectWithRetry path
      // that holds a reference to in-flight calls across reconnects.
    });
  }

  async function connectOnce(): Promise<Socket> {
    return new Promise<Socket>((resolve, reject) => {
      const s = connect(opts.socketPath);
      const onErr = (e: Error) => {
        s.removeListener("connect", onConnect);
        s.destroy();
        reject(e);
      };
      const onConnect = () => {
        s.removeListener("error", onErr);
        resolve(s);
      };
      s.once("error", onErr);
      s.once("connect", onConnect);
    });
  }

  async function connectWithRetry(): Promise<Socket> {
    let lastErr: unknown;
    for (let i = 0; i < connectBackoffsMs.length; i += 1) {
      if (closed) {
        throw new Error("rpc client closed");
      }
      const wait = connectBackoffsMs[i];
      if (wait > 0) {
        await new Promise<void>((r) => setTimeout(r, wait));
      }
      try {
        return await connectOnce();
      } catch (e) {
        lastErr = e;
      }
    }
    throw lastErr instanceof Error ? lastErr : new Error("rpc connect failed");
  }

  function ensureSocket(): Promise<Socket> {
    if (closed) {
      return Promise.reject(new Error("rpc client closed"));
    }
    if (socket && !socket.destroyed) {
      return Promise.resolve(socket);
    }
    if (connectingPromise) {
      return connectingPromise;
    }
    connectingPromise = connectWithRetry()
      .then((s) => {
        attachHandlers(s);
        socket = s;
        return s;
      })
      .finally(() => {
        connectingPromise = null;
      });
    return connectingPromise;
  }

  async function call<T = unknown>(method: string, params: Record<string, unknown>): Promise<T> {
    const id = nextId++;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`timeout calling ${method}`));
      }, timeoutMs);
      pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        timer,
      });
      ensureSocket().then(
        (s) => {
          if (!pending.has(id)) {
            // Already timed out / cancelled while we were connecting.
            return;
          }
          try {
            s.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
          } catch (e) {
            pending.delete(id);
            clearTimeout(timer);
            reject(e);
          }
        },
        (e) => {
          pending.delete(id);
          clearTimeout(timer);
          reject(e);
        },
      );
    });
  }

  return {
    call,
    ping: () => call<PingResult>("secafs.v1.ping", {}),
    mount: (p) => call<MountResult>("secafs.v1.mount", p),
    unmount: (p) => call<UnmountResult>("secafs.v1.unmount", p),
    list: () => call<ListResult>("secafs.v1.list", {}),
    destroy: (p) => call<DestroyResult>("secafs.v1.destroy", p),
    snapshotEnable: (p) =>
      call<SnapshotEnableResult>(
        "secafs.v1.snapshot.enable",
        p as unknown as Record<string, unknown>,
      ),
    snapshotDisable: (p) =>
      call<SnapshotDisableResult>(
        "secafs.v1.snapshot.disable",
        p as unknown as Record<string, unknown>,
      ),
    snapshotCommit: (p) =>
      call<SnapshotCommitResult>(
        "secafs.v1.snapshot.commit",
        p as unknown as Record<string, unknown>,
      ),
    snapshotList: (p) =>
      call<SnapshotListResult>("secafs.v1.snapshot.list", p as unknown as Record<string, unknown>),
    snapshotRestore: (p) =>
      call<SnapshotRestoreResult>(
        "secafs.v1.snapshot.restore",
        p as unknown as Record<string, unknown>,
      ),
    close: () => {
      closed = true;
      const s = socket;
      if (s) {
        socket = null;
        s.end();
      }
    },
  };
}
