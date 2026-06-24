import path from "node:path";

export interface SecafsChatConfig {
  postgresUrl?: string;
  socketPath: string;
  manageDaemon: boolean;
  mountRoot: string;
  /**
   * Seconds since last `updatedAt` after which a mounted SecAFS volume is
   * lazily unmounted to free FUSE resources. Default 0 (DISABLED): upstream's
   * workspace attestation check (ensureAgentWorkspace) runs BEFORE plugin
   * hooks on the embedded-run path, so a directly-addressed unmounted session
   * aborts with WorkspaceVanishedError before the before_prompt_build
   * auto-mount can re-mount it. Only enable for deployments where sessions
   * are always reopened explicitly (secafs.session.open) before chatting.
   */
  idleUnmountSeconds: number;
  /**
   * Seconds between scanner ticks. The scanner is also the MOUNT-KEEPER: it
   * remounts store-mounted sessions the daemon lost (crash + respawn), so
   * keep it running even with idle-unmount disabled. `0` disables the
   * scanner entirely.
   */
  idleScanSeconds: number;
  /**
   * Enable the rollback UI (per-conversation Copy-on-Write snapshots).
   * When `false`, the plugin does not register `secafs.rollback.*` gateway
   * methods and does not commit snapshots at turn boundaries. Triggers in
   * Postgres remain dormant for any volume that never enables rollback.
   * Default: `true`.
   */
  enableRollbackUI: boolean;
}

export interface ResolveInput {
  pluginConfig: {
    postgresUrl?: string;
    socketPath?: string;
    manageDaemon?: boolean;
    mountRoot?: string;
    idleUnmountSeconds?: number;
    idleScanSeconds?: number;
    enableRollbackUI?: boolean;
  };
}

export function resolveSecafsConfig(input: ResolveInput, env: NodeJS.ProcessEnv): SecafsChatConfig {
  const manageDaemon = input.pluginConfig.manageDaemon ?? false;
  if (manageDaemon && !input.pluginConfig.postgresUrl) {
    throw new Error("secafs-chat: postgresUrl required when manageDaemon=true");
  }

  const runtimeDir =
    env.XDG_RUNTIME_DIR ??
    (env.UID ? `/run/user/${env.UID}` : path.join("/tmp", `secafs-${process.getuid?.() ?? "x"}`));
  const stateHome =
    env.XDG_STATE_HOME ?? (env.HOME ? path.join(env.HOME, ".local/state") : "/tmp/.secafs-state");

  const socketPath =
    input.pluginConfig.socketPath ?? path.join(runtimeDir, "secafs", "secafs.sock");
  const mountRoot = input.pluginConfig.mountRoot ?? path.join(stateHome, "secafs", "mounts");

  const idleUnmountSeconds = normalizeNonNegativeInt(input.pluginConfig.idleUnmountSeconds, 0);
  // 2s tick keeps mount-loss recovery under the <5s SLO (keeper detects and
  // remounts within one tick; each tick is a cached store read + one cheap
  // unix-socket RPC + one stat per mounted session).
  const idleScanSeconds = normalizeNonNegativeInt(input.pluginConfig.idleScanSeconds, 2);

  const enableRollbackUI = input.pluginConfig.enableRollbackUI ?? true;

  return {
    postgresUrl: input.pluginConfig.postgresUrl,
    socketPath,
    manageDaemon,
    mountRoot,
    idleUnmountSeconds,
    idleScanSeconds,
    enableRollbackUI,
  };
}

function normalizeNonNegativeInt(value: number | undefined, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return fallback;
  }
  return Math.floor(value);
}
