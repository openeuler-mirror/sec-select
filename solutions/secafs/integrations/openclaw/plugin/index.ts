import path from "node:path";
import { definePluginEntry, type OpenClawPluginApi } from "./api.js";
import { ensureSecafsMountForSession } from "./src/auto-mount-hook.js";
import { resolveSecafsConfig } from "./src/config.js";
import { registerSecafsCli } from "./src/cli.js";
import { createDaemonSupervisor } from "./src/daemon-supervisor.js";
import { registerGatewayMethods } from "./src/gateway-methods.js";
import { startIdleScanner } from "./src/idle-scanner.js";
import { truncateJsonlAfterMessage } from "./src/jsonl-truncate.js";
import { reconcile } from "./src/reconcile.js";
import { createSnapshotOnTurnEnd, findLastAssistantMessageId } from "./src/rollback-hook.js";
import { createHandleRestore } from "./src/rollback-orchestration.js";
import type { SecafsRollbackState } from "./src/rollback-types.js";
import { createSpawnedWorkspaceRedirect } from "./src/spawned-workspace.js";
import { createSecafsRpcClient } from "./src/rpc-client.js";
import { handleSessionEnd } from "./src/session-bindings.js";
import { foldSecafsExt, hoistSecafsExt, hoistSecafsStore } from "./src/session-ext.js";
import {
  loadSessionStore,
  resolveStorePath,
  updateSessionStore,
} from "openclaw/plugin-sdk/session-store-runtime";
import { resolveAgentWorkspaceDir } from "openclaw/plugin-sdk/health";

/**
 * The plugin persists three custom fields on session entries that upstream's
 * `SessionEntry` type does not declare: `kind` (marks a conversation as
 * SecAFS-owned), `mountState`, and `secafsRollback` (per-volume rollback
 * state). On disk these live under the sanctioned
 * `pluginExtensions["secafs-chat"]` slot (see `session-ext.ts`); the load/save
 * wrappers below hoist them to top level in memory so the rest of the plugin
 * keeps reading `entry.kind` / `entry.mountState` / `entry.secafsRollback`.
 * This type captures that in-memory top-level shape for the typed write sites.
 */
type SecafsSessionFields = { kind?: "secafs"; secafsRollback?: SecafsRollbackState };

export default definePluginEntry({
  id: "secafs-chat",
  name: "SecAFS Chat",
  description: "Per-conversation FUSE-mounted workspace backed by SecAFS.",
  register(api: OpenClawPluginApi) {
    // 1. Resolve config
    const cfg = resolveSecafsConfig({ pluginConfig: api.pluginConfig ?? {} }, process.env);

    // 2. Create daemon supervisor
    const supervisor = createDaemonSupervisor({
      manageDaemon: cfg.manageDaemon,
      binary: "secafs",
      args: [
        "serve",
        "api",
        "--socket",
        cfg.socketPath,
        ...(cfg.postgresUrl ? ["--pg-url", cfg.postgresUrl] : []),
        "--mount-root",
        cfg.mountRoot,
      ],
      onExit: (code) => {
        api.logger.warn?.(`[secafs-chat] daemon exited with code ${String(code)}`);
      },
    });

    // 3. Create RPC client
    const rpc = createSecafsRpcClient({ socketPath: cfg.socketPath });

    // 4. Resolve the session-store path + default agent / main-key prefix.
    //    Session-store access uses the PUBLIC openclaw/plugin-sdk/
    //    session-store-runtime API. (api.runtime.agent is NOT populated for
    //    plugins in upstream OpenClaw — the fork-only api.runtime.agent.session
    //    path threw "Cannot read properties of undefined (reading 'session')".)
    const mainKey = (api.config.session?.mainKey?.trim() || "main").toLowerCase();
    const agentsList = api.config.agents?.list ?? [];
    const defaultAgentId =
      agentsList.find((a) => a?.default)?.id?.trim() ?? agentsList[0]?.id?.trim() ?? "main";
    const storePath = resolveStorePath(api.config.session?.store, { agentId: defaultAgentId });

    // Session-store IO:
    // - READS go through loadStore (upstream's cached loadSessionStore +
    //   hoisting the plugin's fields out of pluginExtensions["secafs-chat"]).
    // - WRITES go through updateStore, which wraps upstream's
    //   updateSessionStore: same writer queue the gateway's own
    //   sessions.delete/patch use (lock → reload fresh → mutate → persist).
    //   Direct loadSessionStore+saveSessionStore writes bypass that queue and
    //   race the gateway — deleted entries were observed resurrecting from the
    //   gateway's copy. Never write the store any other way.
    const loadStore = () => hoistSecafsStore(loadSessionStore(storePath));
    type StoreShape = ReturnType<typeof loadStore>;
    const updateStore = async <T>(fn: (store: StoreShape) => T): Promise<T> => {
      return await updateSessionStore(storePath, (raw) => {
        const store = raw as StoreShape;
        for (const k of Object.keys(store)) store[k] = hoistSecafsExt(store[k]);
        const result = fn(store);
        for (const k of Object.keys(store)) store[k] = foldSecafsExt(store[k]);
        return result;
      });
    };

    const sessionStore = {
      async load(sessionKey: string) {
        const store = loadStore();
        return store[sessionKey] ?? null;
      },
      async patch(sessionKey: string, patch: Record<string, unknown>) {
        // Update-only: patching a missing key must NOT create an entry.
        // Multi-form writers (workspace redirect, rollback state) patch every
        // key form; upserting here would resurrect entries destroy just
        // deleted. Creation goes through sessions.create/ensureSessionEntry.
        await updateStore((store) => {
          const existing = store[sessionKey];
          if (!existing) return;
          store[sessionKey] = { ...existing, ...patch } as StoreShape[string];
        });
      },
    };

    // 5. The Path C cwd redirect (`workspace`) is defined just after
    //    collectKeyForms below — it must write spawnedCwd to ALL session-key
    //    forms (bare + canonical), so it needs collectKeyForms first.

    // 6. Build sessions adapter for gateway-methods.
    const sessions = {
      async create(_opts: { kind: "secafs" }): Promise<{ sessionKey: string }> {
        const { randomUUID } = await import("node:crypto");
        const uuid = randomUUID();
        const sessionKey = `${mainKey}:${uuid}`;
        // Persist `kind: "secafs"` so the session stays grouped under the
        // SecAFS Chat tab even after the user clicks Close (which clears
        // the workspace override but leaves kind intact). Only destroy
        // removes the entry.
        await updateStore((store) => {
          store[sessionKey] = {
            sessionId: uuid,
            updatedAt: Date.now(),
            kind: "secafs",
          } as (typeof store)[string];
        });
        return { sessionKey };
      },
      async patch(key: string, patch: Record<string, unknown>): Promise<void> {
        await updateStore((store) => {
          const existing = store[key];
          if (!existing) {
            return;
          }
          store[key] = { ...existing, ...patch } as (typeof store)[string];
        });
      },
      async load(key: string): Promise<{ sessionId: string } | null> {
        const store = loadStore();
        const entry = store[key];
        if (!entry?.sessionId) {
          return null;
        }
        return { sessionId: entry.sessionId };
      },
      async keys(): Promise<string[]> {
        const store = loadStore();
        return Object.keys(store);
      },
      async entries(): Promise<Record<string, Record<string, unknown>>> {
        return loadStore() as unknown as Record<string, Record<string, unknown>>;
      },
      async delete(key: string): Promise<void> {
        await updateStore((store) => {
          delete store[key];
        });
      },
    };

    // 7. Resolve the default agent workspace directory once at register time.
    //    Used to seed freshly-created SecAFS volumes so a new conversation
    //    inherits the agent's identity/memory rather than presenting an
    //    empty BOOTSTRAP-pending workspace.
    let defaultWorkspaceDir: string | undefined;
    try {
      defaultWorkspaceDir = resolveAgentWorkspaceDir(api.config, defaultAgentId);
    } catch (err) {
      api.logger.warn?.(
        `[secafs-chat] could not resolve default workspace dir for seeding: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }

    // 8. Build rollback helpers.
    const sessionFileFor = (sessionKey: string): string => {
      const store = loadStore();
      const entry = store[sessionKey];
      if (entry?.sessionFile) {
        return entry.sessionFile;
      }
      if (entry?.sessionId) {
        const dir = path.dirname(storePath);
        return path.join(dir, `${entry.sessionId}.jsonl`);
      }
      const sid = sessionKey.split(":").findLast(Boolean) ?? sessionKey;
      return path.join(path.dirname(storePath), `${sid}.jsonl`);
    };

    const trajectoryFor = (sessionKey: string): string =>
      sessionFileFor(sessionKey).replace(/\.jsonl$/, ".trajectory.jsonl");

    const workspaceFor = (sessionKey: string): string => {
      const sid = sessionKey.split(":").findLast(Boolean) ?? sessionKey;
      return path.join(cfg.mountRoot, sid);
    };

    const extractConvIdFn = (sessionKey: string): string => {
      const parts = sessionKey.split(":").filter((p) => p.length > 0);
      return parts.length >= 2 ? parts[parts.length - 1] : sessionKey;
    };

    const events = (event: { type: string; sessionKey: string; [k: string]: unknown }) => {
      const broadcast = (
        api as unknown as { broadcast?: (channel: string, payload: unknown) => void }
      ).broadcast;
      if (typeof broadcast === "function") {
        broadcast.call(api, "secafs.rollback", event);
      } else {
        api.logger.info?.(`[secafs-chat] event: ${JSON.stringify(event)}`);
      }
    };

    // Sessions are tracked under multiple key forms — bare ("main:<uuid>"),
    // canonical ("agent:<agentId>:main:<uuid>"), and any caller-supplied form.
    // The bare form is created by secafs.session.create and carries kind=secafs;
    // the canonical form is created by the agent runtime when the conversation
    // first runs. Rollback metadata (secafsRollback) only co-locates cleanly on
    // the bare form because it owns the conversation lifecycle. Reads must
    // tolerate any caller-supplied form; writes must reach all forms that
    // currently exist so future readers under any form see consistent state.
    const collectKeyForms = (sessionKey: string): string[] => {
      const sid = extractConvIdFn(sessionKey);
      const bare = `${mainKey}:${sid}`;
      const canonical = `agent:${defaultAgentId}:${mainKey}:${sid}`;
      return [...new Set([bare, sessionKey, canonical])];
    };

    // Destroy semantics include the chat record: remove the conversation's
    // transcript files. Paths come from entry.sessionFile across all key forms
    // (the agent runtime assigns its own internal sessionId, so the file name
    // is NOT derivable from the sid alone), plus the sid-named pre-seed file.
    const deleteSessionArtifacts = async (sid: string): Promise<void> => {
      const fs = await import("node:fs/promises");
      const store = loadStore();
      const dir = path.dirname(storePath);
      const transcripts = new Set<string>([path.join(dir, `${sid}.jsonl`)]);
      for (const key of collectKeyForms(`${mainKey}:${sid}`)) {
        const sessionFile = (store[key] as { sessionFile?: string } | undefined)?.sessionFile;
        if (sessionFile) transcripts.add(sessionFile);
      }
      for (const file of transcripts) {
        for (const target of [file, file.replace(/\.jsonl$/, ".trajectory.jsonl")]) {
          try {
            await fs.rm(target, { force: true });
          } catch (err) {
            api.logger.warn?.(
              `[secafs-chat] failed to delete transcript ${target}: ${
                err instanceof Error ? err.message : String(err)
              }`,
            );
          }
        }
      }
    };

    // Export/import transcript IO. Read collects from entry.sessionFile across
    // key forms (the runtime may use an internal-uuid file name) with the
    // sid-named file as fallback; write always lands on the sid-named file —
    // the importer pre-seeds the canonical entry's sessionFile to match.
    const readTranscripts = async (
      sid: string,
    ): Promise<{ sessionJsonl?: string; trajectoryJsonl?: string }> => {
      const fs = await import("node:fs/promises");
      const store = loadStore();
      const dir = path.dirname(storePath);
      const candidates = [path.join(dir, `${sid}.jsonl`)];
      for (const key of collectKeyForms(`${mainKey}:${sid}`)) {
        const sessionFile = (store[key] as { sessionFile?: string } | undefined)?.sessionFile;
        if (sessionFile && !candidates.includes(sessionFile)) candidates.push(sessionFile);
      }
      const result: { sessionJsonl?: string; trajectoryJsonl?: string } = {};
      for (const file of candidates) {
        try {
          if (!result.sessionJsonl) {
            result.sessionJsonl = (await fs.readFile(file)).toString("base64");
          }
        } catch {
          /* missing; try next candidate */
        }
        try {
          if (!result.trajectoryJsonl) {
            const traj = file.replace(/\.jsonl$/, ".trajectory.jsonl");
            result.trajectoryJsonl = (await fs.readFile(traj)).toString("base64");
          }
        } catch {
          /* missing; try next candidate */
        }
      }
      return result;
    };

    const writeTranscripts = async (
      sid: string,
      chat: { sessionJsonl?: string; trajectoryJsonl?: string },
    ): Promise<void> => {
      const fs = await import("node:fs/promises");
      const dir = path.dirname(storePath);
      if (chat.sessionJsonl) {
        await fs.writeFile(path.join(dir, `${sid}.jsonl`), Buffer.from(chat.sessionJsonl, "base64"));
      }
      if (chat.trajectoryJsonl) {
        await fs.writeFile(
          path.join(dir, `${sid}.trajectory.jsonl`),
          Buffer.from(chat.trajectoryJsonl, "base64"),
        );
      }
    };

    const rollbackSessionStore = {
      load: async (key: string): Promise<{ secafsRollback?: SecafsRollbackState } | null> => {
        const store = loadStore();
        // Prefer whichever form actually has secafsRollback set. Falls back to
        // the bare form (which carries kind:secafs) when none has rollback yet.
        for (const k of collectKeyForms(key)) {
          const entry = store[k] as (SecafsSessionFields & { sessionId?: string }) | undefined;
          if (entry?.secafsRollback !== undefined) {
            return entry as { secafsRollback?: SecafsRollbackState };
          }
        }
        const sid = extractConvIdFn(key);
        const bare = store[`${mainKey}:${sid}`];
        return bare ? (bare as { secafsRollback?: SecafsRollbackState }) : null;
      },
      patch: async (key: string, patch: Record<string, unknown>) => {
        // Patch every form that currently has an entry; sessionStore.patch
        // returns silently for missing entries so this never errors.
        for (const k of collectKeyForms(key)) {
          await sessionStore.patch(k, patch);
        }
      },
    };

    // Path C cwd redirect. Writes spawnedCwd + spawnedWorkspaceDir + spawnedBy to
    // ALL key forms (bare + canonical) via collectKeyForms, so the agent run —
    // which resolves the canonical "agent:<id>:main:<uuid>" key — runs with cwd
    // inside the FUSE mount even though secafs.session.create used the bare key.
    const workspace = createSpawnedWorkspaceRedirect(
      { patch: (key, patch) => rollbackSessionStore.patch(key, patch) },
      "secafs-chat",
    );

    const snapshotOnTurnEnd = createSnapshotOnTurnEnd({
      rpc,
      sessionStore: rollbackSessionStore,
      findLastAssistantMessageId,
      events,
      sessionFileFor,
      extractConvId: extractConvIdFn,
      logger: api.logger,
    });

    // Eagerly snapshot the just-finished turn (the before_prompt_build hook
    // only commits lazily at the START of the next turn). Used by the
    // frontend's per-message rollback button, which needs a snapId for the
    // reply as soon as it lands. Same dedupe contract as the lazy hook
    // (lastSnapshotMessageId), so the two paths never double-commit.
    const snapshotNow = async ({ sessionKey }: { sessionKey: string }) => {
      const entry = await rollbackSessionStore.load(sessionKey);
      const rb: SecafsRollbackState = entry?.secafsRollback ?? { enabled: false };
      if (!rb.enabled || rb.inProgress) {
        return { enabled: rb.enabled === true, committed: false };
      }
      const messageId = await findLastAssistantMessageId(sessionFileFor(sessionKey));
      if (!messageId) {
        return { enabled: true, committed: false };
      }
      const conversationId = extractConvIdFn(sessionKey);
      if (rb.lastSnapshotMessageId === messageId) {
        // already snapshotted (lazy hook or a prior call) — surface the
        // existing snapId so the caller can still wire its button
        const list = await rpc.snapshotList({ conversationId });
        const hit = [...list.snapshots].reverse().find((s) => s.label === messageId);
        return { enabled: true, committed: false, snapId: hit?.snapId, messageId };
      }
      const r = await rpc.snapshotCommit({ conversationId, label: messageId });
      await rollbackSessionStore.patch(sessionKey, {
        secafsRollback: { ...rb, lastSnapshotMessageId: messageId },
      });
      return { enabled: true, committed: true, snapId: r.snapId, messageId };
    };

    const handleRestoreFn = createHandleRestore({
      rpc,
      sessionStore: rollbackSessionStore,
      truncate: truncateJsonlAfterMessage,
      events,
      sessionFileFor,
      trajectoryFor,
      workspaceFor,
      extractConvId: extractConvIdFn,
      logger: api.logger,
    });

    // 9. Register gateway methods
    registerGatewayMethods({
      registerGatewayMethod: (name, handler) => {
        api.registerGatewayMethod(name, async ({ params, respond }) => {
          try {
            const result = await handler(params);
            respond(true, result);
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            respond(false, undefined, { code: "UNAVAILABLE", message });
          }
        });
      },
      rpc,
      sessions,
      workspace,
      mountRoot: cfg.mountRoot,
      defaultWorkspaceDir,
      mainKey,
      defaultAgentId,
      logger: api.logger,
      handleRestore: handleRestoreFn,
      snapshotNow,
      sessionStore: rollbackSessionStore,
      enableRollbackUI: cfg.enableRollbackUI,
      sessionFileFor: (sid: string) => path.join(path.dirname(storePath), `${sid}.jsonl`),
      deleteSessionArtifacts,
      readTranscripts,
      writeTranscripts,
      ensureSessionEntry: async (key, entry) => {
        await updateStore((store) => {
          if (store[key]) return; // already exists; never overwrite agent runtime fields
          store[key] = { ...entry } as (typeof store)[string];
        });
      },
    });

    // Register the `openclaw secafs ...` CLI (folded in from core, Cat 3).
    // Subcommands are thin gateway clients hitting this plugin's secafs.* methods.
    api.registerCli((ctx) =>
      registerSecafsCli(ctx as unknown as Parameters<typeof registerSecafsCli>[0]),
    );

    // 8. Register session_end hook — unmount managed workspace sessions.
    api.on("session_end", async (event) => {
      const key = event.sessionKey;
      if (!key) {
        return;
      }
      // Load session metadata from store; the hook event does not carry it.
      // Path C: a secafs-managed conversation is marked by kind:"secafs" (set on
      // create) or by our plugin id in spawnedBy (set while the cwd redirect is
      // active), not by the removed workspace.managedBy field.
      const store = loadStore();
      const entry = store[key] as { kind?: string; spawnedBy?: string } | undefined;
      const managed = entry?.kind === "secafs" || entry?.spawnedBy === "secafs-chat";
      await handleSessionEnd(
        { sessionKey: key, workspace: managed ? { managedBy: "secafs-chat" } : undefined },
        { rpc, sessions, logger: api.logger },
      );
    });

    // 10. Register before_prompt_build hook — auto-mount the SecAFS volume
    //    if it was idle-unmounted, so the prompt builder reads from the
    //    live FUSE mount rather than a phantom directory. Then snapshot the
    //    last assistant message so rollback points stay current.
    api.on("before_prompt_build", async (_event, ctx) => {
      await ensureSecafsMountForSession(
        {
          loadStore: () => loadStore(),
          patchSession: (key, patch) => sessionStore.patch(key, patch),
          workspace: { set: (key, spec) => workspace.set(key, spec) },
          rpc,
          mountRoot: cfg.mountRoot,
          logger: api.logger,
        },
        ctx?.sessionKey,
      );
      if (cfg.enableRollbackUI) {
        await snapshotOnTurnEnd({ sessionKey: ctx?.sessionKey });
      }
    });

    // 10. Start the volume scanner: mount-keeper (always) + idle unmount
    //     (opt-in). No-op only when idleScanSeconds is 0.
    const idleScanner = startIdleScanner(
      {
        loadStore: () => loadStore(),
        patchSession: (key, patch) => sessionStore.patch(key, patch),
        rpc,
        workspace,
        mountRoot: cfg.mountRoot,
        logger: api.logger,
      },
      {
        idleUnmountSeconds: cfg.idleUnmountSeconds,
        idleScanSeconds: cfg.idleScanSeconds,
      },
    );

    // 11. Register shutdown via gateway_stop hook
    api.on("gateway_stop", async () => {
      idleScanner.stop();
      try {
        rpc.close();
      } catch {
        // ignore close errors on shutdown
      }
      await supervisor.stop();
    });

    // 12. Kick off async startup (daemon spawn + reconcile). Fire-and-forget so
    //     register() stays synchronous per the plugin contract; errors are logged
    //     but do not block gateway startup.
    void (async () => {
      try {
        await supervisor.start();
        if (cfg.manageDaemon) {
          // Give the daemon a moment to bind its socket before we try to reconcile.
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
        const result = await reconcile({
          rpc,
          sessions: {
            load: async (key: string) => {
              // The first agent run migrates the bare "main:<uuid>" entry to its
              // canonical "agent:<id>:main:<uuid>" key, so an exact-key lookup
              // would misjudge a live session as an orphan and unmount it.
              const store = loadStore();
              for (const k of collectKeyForms(key)) {
                if (store[k]) return store[k];
              }
              return null;
            },
            keys: async () => {
              const store = loadStore();
              return Object.keys(store);
            },
            patch: async (key: string, p: Record<string, unknown>) => sessionStore.patch(key, p),
          },
          sessionKeyFor: (id: string) => `${mainKey}:${id}`,
          logger: api.logger,
          truncateJsonlAfterMessage,
          sessionFileFor,
          trajectoryFor,
          workspaceFor,
          extractConvId: extractConvIdFn,
        });
        if (result.unmounted > 0) {
          api.logger.info?.(`[secafs-chat] reconcile unmounted ${result.unmounted} orphan(s)`);
        }
        if (result.rollbacksResumed > 0) {
          api.logger.info?.(
            `[secafs-chat] reconcile resumed ${result.rollbacksResumed} in-progress rollback(s)`,
          );
        }
      } catch (e) {
        api.logger.warn?.(`[secafs-chat] async startup failed: ${String(e)}`);
      }
    })();

    api.logger.info?.("[secafs-chat] plugin registered");
  },
});
