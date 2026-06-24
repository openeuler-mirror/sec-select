import { type ChildProcess, spawn as nodeSpawn } from "node:child_process";

export interface DaemonSupervisor {
  start(): Promise<void>;
  stop(): Promise<void>;
  isRunning(): boolean;
}

export interface SupervisorOpts {
  manageDaemon: boolean;
  binary: string;
  args: string[];
  spawn?: typeof nodeSpawn;
  onExit?: (code: number | null) => void;
}

export function createDaemonSupervisor(opts: SupervisorOpts): DaemonSupervisor {
  const spawnFn = opts.spawn ?? nodeSpawn;
  let child: ChildProcess | null = null;

  return {
    async start() {
      if (!opts.manageDaemon) {
        return;
      }
      if (child) {
        return;
      }
      const c = spawnFn(opts.binary, opts.args, { stdio: "inherit" });
      c.on("exit", (code) => {
        if (c === child) {
          child = null;
        }
        opts.onExit?.(code);
      });
      child = c;
    },
    async stop() {
      const c = child;
      if (!c) {
        return;
      }
      child = null;
      c.kill("SIGTERM");
      await new Promise<void>((resolve) => {
        let done = false;
        const finalize = () => {
          if (!done) {
            done = true;
            resolve();
          }
        };
        c.once("exit", finalize);
        // Safety timeout: if daemon ignores SIGTERM, kill -9 after 3s
        const killTimer = setTimeout(() => {
          c.kill("SIGKILL");
        }, 3_000);
        c.once("exit", () => clearTimeout(killTimer));
      });
    },
    isRunning() {
      return child !== null;
    },
  };
}
