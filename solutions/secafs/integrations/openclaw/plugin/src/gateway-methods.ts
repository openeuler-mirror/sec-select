import fs from "node:fs/promises";
import path from "node:path";
import { readSecafsFile, writeSecafsFile } from "./fs-methods.js";
import type { SecafsRollbackState } from "./rollback-types.js";

export interface RegisterGatewayMethodsOpts {
  registerGatewayMethod: (name: string, handler: (params: unknown) => Promise<unknown>) => void;
  rpc: {
    ping(): Promise<{ version: string; pgConnected: boolean; mountCount: number }>;
    list(): Promise<{
      mounts: Array<{ conversationId: string; hostPath: string; since: string }>;
    }>;
    mount(p: {
      conversationId: string;
      hostPath?: string;
    }): Promise<{ hostPath: string; mounted: boolean }>;
    unmount(p: { conversationId: string }): Promise<{ unmounted: boolean }>;
    destroy(p: { conversationId: string }): Promise<{ destroyed: boolean }>;
    snapshotEnable(p: {
      conversationId: string;
    }): Promise<{ enabled: boolean; currentSnapId: number }>;
    snapshotDisable(p: {
      conversationId: string;
    }): Promise<{ disabled: boolean; purgedSnapshots: number; purgedUndoRows: number }>;
    snapshotCommit(p: {
      conversationId: string;
      label?: string;
    }): Promise<{ snapId: number; committedAt: string; label: string | null }>;
    snapshotList(p: { conversationId: string }): Promise<{
      snapshots: Array<{ snapId: number; committedAt: string; label: string | null }>;
    }>;
    snapshotRestore(p: {
      conversationId: string;
      snapId: number;
    }): Promise<{ restored: boolean; prunedSnapshots: number; prunedUndoRows: number }>;
  };
  sessions: {
    create(opts: { kind: "secafs" }): Promise<{ sessionKey: string }>;
    patch(key: string, p: Record<string, unknown>): Promise<void>;
    load(key: string): Promise<{ sessionId: string } | null>;
    delete(key: string): Promise<void>;
    /** Full session store (hoisted shape), for secafs.session.list. */
    entries(): Promise<Record<string, Record<string, unknown>>>;
  };
  workspace: {
    set(key: string, spec: { path: string }): Promise<void>;
    clear(key: string): Promise<void>;
  };
  mountRoot: string;
  /**
   * Optional session store for reading per-session rollback state (e.g. enabled flag).
   * When provided, `secafs.rollback.list` will derive the `enabled` field from the
   * stored `secafsRollback` entry instead of defaulting to `false`.
   */
  sessionStore?: {
    load(key: string): Promise<{ secafsRollback?: SecafsRollbackState } | null>;
  };
  /**
   * Orchestration handler for restore operations. Called by `secafs.rollback.restore`.
   * Provided by the plugin's index.ts (Task 3.9).
   */
  handleRestore: (args: { sessionKey: string; snapId: number }) => Promise<{
    restored: true;
    restoredMessageId: string;
    prunedSnapshots: number;
    prunedUndoRows: number;
  }>;
  /**
   * Eagerly snapshot the just-finished turn and return its snapId (the lazy
   * before_prompt_build hook only commits at the start of the NEXT turn).
   * Backs `secafs.rollback.snapshot`, used by the per-message rollback button.
   */
  snapshotNow?: (args: { sessionKey: string }) => Promise<{
    enabled: boolean;
    committed: boolean;
    snapId?: number;
    messageId?: string;
  }>;
  /**
   * Feature flag for the rollback UI. When `false`, the three
   * `secafs.rollback.*` gateway methods are NOT registered. Default `true`.
   */
  enableRollbackUI?: boolean;
  /**
   * Configurable session-key prefixes. Defaults match OpenClaw's defaults
   * (`mainKey="main"`, `agentId="main"`) so existing setups behave exactly
   * as before. Used to build the bare and canonical key forms during
   * destroy so we delete both halves of a conversation regardless of the
   * deployment's renaming choices.
   */
  mainKey?: string;
  defaultAgentId?: string;
  /**
   * Absolute path of the default agent workspace. When provided, the contents
   * are seeded into a freshly-created SecAFS volume so the new conversation
   * inherits the agent's identity/memory files (AGENTS.md, IDENTITY.md,
   * USER.md, SOUL.md, etc.) instead of presenting an empty BOOTSTRAP-pending
   * workspace. Set to `undefined` to disable seeding.
   */
  defaultWorkspaceDir?: string;
  /**
   * Resolve the absolute path to the chat history JSONL file for a sessionId.
   * Used when pre-seeding the canonical session entry so the agent runtime
   * finds a valid sessionFile before its first prompt.
   */
  sessionFileFor?: (sessionId: string) => string;
  /**
   * Create a session-store entry under `key` if one does not already exist;
   * no-op if it does. Used to pre-seed the canonical-form entry so the agent
   * runtime does not later auto-create it with an invalid sessionId.
   */
  ensureSessionEntry?: (key: string, entry: Record<string, unknown>) => Promise<void>;
  /**
   * Delete the conversation's chat transcript files (.jsonl and
   * .trajectory.jsonl) as part of destroy. Called before the store entries
   * are removed (the paths come from entry.sessionFile). Best-effort.
   */
  deleteSessionArtifacts?: (sid: string) => Promise<void>;
  /**
   * Read the conversation's transcript files for export. Returns base64
   * contents (fields undefined when the file is missing).
   */
  readTranscripts?: (sid: string) => Promise<{
    sessionJsonl?: string;
    trajectoryJsonl?: string;
  }>;
  /**
   * Write imported transcript files for a fresh conversation. Contents are
   * base64; the implementation owns path resolution (sid-named files in the
   * session store directory).
   */
  writeTranscripts?: (
    sid: string,
    chat: { sessionJsonl?: string; trajectoryJsonl?: string },
  ) => Promise<void>;
  logger?: { info?: (msg: string) => void; warn?: (msg: string) => void };
}

/** Export archive shape shared by secafs.session.export/import. */
export interface SessionArchive {
  manifest: {
    schemaVersion: 1;
    sid: string;
    alias: string | null;
    exportedAt: string;
    rollbackEnabled: boolean;
    fileCount: number;
    totalBytes: number;
  };
  files: Array<{ path: string; content: string }>; // content = base64
  chat: { sessionJsonl?: string; trajectoryJsonl?: string }; // base64
}

/**
 * Names that should NOT be copied from the default workspace into a new SecAFS
 * volume. `.git/` is per-workspace and large; `.openclaw/` holds bootstrap
 * state that must be derived from the new volume's own files; transient
 * heartbeat scratch files are session-specific.
 */
const SEED_EXCLUDE_NAMES = new Set([".git", ".openclaw", "node_modules"]);

/** Hard cap for session export/import payloads (raw bytes, pre-base64). */
const MAX_ARCHIVE_BYTES = 50 * 1024 * 1024;
const MAX_ARCHIVE_FILES = 5000;

/**
 * Recursively collect every regular file under `root` as
 * {path: relative, content: base64}. Symlinks and special files are skipped
 * (the agent workspace doesn't use them; SecAFS symlink rows would need a
 * dedicated archive field). Throws when caps are exceeded.
 */
async function collectExportFiles(root: string): Promise<Array<{ path: string; content: string }>> {
  const out: Array<{ path: string; content: string }> = [];
  let totalBytes = 0;
  async function walk(dir: string, rel: string): Promise<void> {
    const entries = await fs.readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
      const abs = path.join(dir, entry.name);
      const relPath = rel ? `${rel}/${entry.name}` : entry.name;
      if (entry.isDirectory()) {
        await walk(abs, relPath);
        continue;
      }
      if (!entry.isFile()) {
        continue; // symlinks/sockets/devices not supported in archives
      }
      const buf = await fs.readFile(abs);
      totalBytes += buf.byteLength;
      if (totalBytes > MAX_ARCHIVE_BYTES) {
        throw new Error(`workspace exceeds export limit of ${MAX_ARCHIVE_BYTES / (1024 * 1024)}MB`);
      }
      out.push({ path: relPath, content: buf.toString("base64") });
      if (out.length > MAX_ARCHIVE_FILES) {
        throw new Error(`workspace exceeds export limit of ${MAX_ARCHIVE_FILES} files`);
      }
    }
  }
  await walk(root, "");
  return out;
}

interface TreeEntry {
  name: string;
  kind: "file" | "dir";
  size?: number;
  children?: TreeEntry[];
  truncated?: boolean;
}

/**
 * Walk `root` and return a depth-limited, entry-capped tree representation.
 * Hidden directories and the `.openclaw/` state dir are skipped to keep the
 * payload small and the rendered tree focused on user-visible content. The
 * traversal is best-effort: unreadable entries are silently dropped rather
 * than aborting the whole tree.
 */
async function readTree(params: {
  root: string;
  maxEntries: number;
  maxDepth: number;
}): Promise<TreeEntry[]> {
  const skipNames = new Set([".git", ".openclaw"]);
  let remaining = params.maxEntries;
  async function walk(dir: string, depth: number): Promise<TreeEntry[]> {
    if (depth > params.maxDepth || remaining <= 0) {
      return [];
    }
    let entries: import("node:fs").Dirent[];
    try {
      entries = await fs.readdir(dir, { withFileTypes: true });
    } catch {
      return [];
    }
    entries.sort((a, b) => {
      if (a.isDirectory() !== b.isDirectory()) {
        return a.isDirectory() ? -1 : 1;
      }
      return a.name.localeCompare(b.name);
    });
    const result: TreeEntry[] = [];
    for (const entry of entries) {
      if (skipNames.has(entry.name)) {
        continue;
      }
      if (remaining <= 0) {
        result.push({ name: "…", kind: "file", truncated: true });
        break;
      }
      remaining -= 1;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        result.push({
          name: entry.name,
          kind: "dir",
          children: await walk(full, depth + 1),
        });
      } else {
        let size: number | undefined;
        try {
          const st = await fs.stat(full);
          size = st.size;
        } catch {
          /* unreadable; keep entry without size */
        }
        result.push({ name: entry.name, kind: "file", size });
      }
    }
    return result;
  }
  return walk(params.root, 0);
}

async function seedFromDefaultWorkspace(params: {
  sourceDir: string;
  destDir: string;
  logger?: { info?: (msg: string) => void; warn?: (msg: string) => void };
}): Promise<void> {
  const { sourceDir, destDir, logger } = params;
  let entries: import("node:fs").Dirent[];
  try {
    entries = await fs.readdir(sourceDir, { withFileTypes: true });
  } catch (err) {
    logger?.warn?.(
      `[secafs-chat] seed skipped: cannot read default workspace ${sourceDir}: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
    return;
  }
  for (const entry of entries) {
    if (SEED_EXCLUDE_NAMES.has(entry.name)) {
      continue;
    }
    const src = path.join(sourceDir, entry.name);
    const dst = path.join(destDir, entry.name);
    try {
      await fs.cp(src, dst, { recursive: true, errorOnExist: false, force: false });
    } catch (err) {
      logger?.warn?.(
        `[secafs-chat] seed: failed to copy ${entry.name}: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }
  }
}

function normalizeAlias(raw: unknown): string | undefined {
  if (typeof raw !== "string") return undefined;
  const alias = raw.trim();
  return alias ? alias.slice(0, 64) : undefined;
}

function sidFromKey(sessionKey: string): string {
  // Session keys come in two forms that must both map to the same SecAFS
  // conversationId so a session created via the bare form can later be
  // opened/closed via its canonical agent form (and vice versa):
  //   - bare:        "main:<uuid>"                  → return "<uuid>"
  //   - canonical:   "agent:<agentId>:main:<uuid>"  → return "<uuid>"
  // The conversationId is the last colon-delimited segment in either case.
  const parts = sessionKey.split(":").filter((p) => p.length > 0);
  return parts.length >= 2 ? parts[parts.length - 1] : sessionKey;
}

export function registerGatewayMethods(opts: RegisterGatewayMethodsOpts): void {
  // Sessions are tracked under two key forms (see destroy below for context):
  //   bare:        `${mainKey}:${conversationId}`
  //   canonical:   `agent:${defaultAgentId}:${mainKey}:${conversationId}`
  // Different code paths reach the plugin with either form, so any state we
  // care about (kind, workspace, secafsRollback) must be written to both forms
  // and read from whichever form has the data.
  const buildKeyForms = (sessionKey: string): string[] => {
    const sid = sidFromKey(sessionKey);
    const mk = opts.mainKey ?? "main";
    const aid = opts.defaultAgentId ?? "main";
    const bare = `${mk}:${sid}`;
    const canonical = `agent:${aid}:${mk}:${sid}`;
    const set = new Set<string>([sessionKey, bare, canonical]);
    return [...set];
  };
  const loadAnyForm = async (
    sessionKey: string,
  ): Promise<{ secafsRollback?: SecafsRollbackState } | null> => {
    if (!opts.sessionStore) return null;
    for (const k of buildKeyForms(sessionKey)) {
      const entry = await opts.sessionStore.load(k);
      if (entry?.secafsRollback !== undefined) return entry;
    }
    return null;
  };
  const patchAllForms = async (
    sessionKey: string,
    patch: Record<string, unknown>,
  ): Promise<void> => {
    for (const k of buildKeyForms(sessionKey)) {
      try {
        await opts.sessions.patch(k, patch);
      } catch {
        /* missing entry; continue */
      }
    }
  };

  opts.registerGatewayMethod("secafs.status", async () => {
    try {
      const p = await opts.rpc.ping();
      return {
        daemonReachable: true,
        pgConnected: p.pgConnected,
        mountCount: p.mountCount,
      };
    } catch {
      return { daemonReachable: false, pgConnected: false, mountCount: 0 };
    }
  });

  opts.registerGatewayMethod("secafs.session.create", async (raw: unknown) => {
    const params = (raw ?? {}) as { alias?: string };
    const alias = normalizeAlias(params.alias);
    const { sessionKey } = await opts.sessions.create({ kind: "secafs" });
    const sid = sidFromKey(sessionKey);
    const hostPath = path.join(opts.mountRoot, sid);
    const result = await opts.rpc.mount({ conversationId: sid, hostPath });
    // Seed the freshly-created volume with the default agent workspace so the
    // new conversation inherits the agent's identity/memory files. The mount
    // is FUSE-backed, so writes flow through to Postgres and persist across
    // close/reopen cycles. Done before workspace.set so the agent's first
    // turn already sees the seeded files.
    if (opts.defaultWorkspaceDir) {
      await seedFromDefaultWorkspace({
        sourceDir: opts.defaultWorkspaceDir,
        destDir: result.hostPath,
        logger: opts.logger,
      });
    }
    // Pre-seed the canonical-form entry (`agent:<aid>:<mainKey>:<sid>`) with
    // the correct sessionId and sessionFile so the agent runtime's chat
    // dispatcher passes validation on the first prompt. Without this, the
    // runtime auto-creates the canonical entry with sessionId set to the
    // full sessionKey (which contains colons), which fails the regex
    // /^[a-z0-9][a-z0-9._-]{0,127}$/i in validateSessionId and the dispatch
    // rejects with "Invalid session ID". opts.ensureSessionEntry is provided
    // by index.ts and creates the entry if missing (no-op if it exists).
    // MUST run BEFORE workspace.set: session patches are update-only, so the
    // Path C spawned* fields only land on the canonical key if its entry
    // already exists — otherwise the agent run (which resolves the canonical
    // key) falls back to the default workspace instead of the FUSE mount.
    const mk = opts.mainKey ?? "main";
    const aid = opts.defaultAgentId ?? "main";
    const canonicalKey = `agent:${aid}:${mk}:${sid}`;
    if (canonicalKey !== sessionKey && opts.ensureSessionEntry) {
      const sessionFile = opts.sessionFileFor?.(sid);
      await opts.ensureSessionEntry(canonicalKey, {
        sessionId: sid,
        ...(sessionFile ? { sessionFile } : {}),
        updatedAt: Date.now(),
      });
    }
    await opts.workspace.set(sessionKey, { path: result.hostPath });
    // mountState must reach ALL key forms: the mount-keeper and the idle
    // scanner read it from whichever form survives the bare->canonical
    // migration; a bare-only write is silently dropped (update-only patch).
    await patchAllForms(sessionKey, { mountState: "mounted" });
    // Write the secafs marker (and alias) to BOTH key forms: the first agent
    // run migrates the bare entry to the canonical key WITHOUT merging fields,
    // so anything present only on the bare entry would be lost.
    const marker = { kind: "secafs", ...(alias ? { alias } : {}) };
    await opts.sessions.patch(sessionKey, marker);
    if (canonicalKey !== sessionKey) {
      await opts.sessions.patch(canonicalKey, marker);
    }
    opts.logger?.info?.(
      `[secafs-chat] session created and mounted: ${sessionKey} → ${result.hostPath}`,
    );
    return { sessionKey, hostPath: result.hostPath, ...(alias ? { alias } : {}) };
  });

  opts.registerGatewayMethod("secafs.session.open", async (raw: unknown) => {
    const params = raw as { sessionKey: string };
    const sid = sidFromKey(params.sessionKey);
    const hostPath = path.join(opts.mountRoot, sid);
    const result = await opts.rpc.mount({ conversationId: sid, hostPath });
    // Same ordering constraint as create: the canonical entry must exist
    // before workspace.set, or the update-only patch skips it and the agent
    // run misses the Path C cwd redirect.
    const canonicalKey = `agent:${opts.defaultAgentId ?? "main"}:${opts.mainKey ?? "main"}:${sid}`;
    if (canonicalKey !== params.sessionKey && opts.ensureSessionEntry) {
      const sessionFile = opts.sessionFileFor?.(sid);
      await opts.ensureSessionEntry(canonicalKey, {
        sessionId: sid,
        ...(sessionFile ? { sessionFile } : {}),
        updatedAt: Date.now(),
      });
    }
    await opts.workspace.set(params.sessionKey, { path: result.hostPath });
    await patchAllForms(params.sessionKey, { mountState: "mounted" });
    return { hostPath: result.hostPath };
  });

  opts.registerGatewayMethod("secafs.session.close", async (raw: unknown) => {
    const params = raw as { sessionKey: string };
    const sid = sidFromKey(params.sessionKey);
    const result = await opts.rpc.unmount({ conversationId: sid });
    // Clear the workspace override so the agent stops writing into the
    // now-unmounted host directory (which would silently land on local disk
    // and be shadowed when the volume is remounted). The agent falls back to
    // the default workspace; reopening the session re-sets the override.
    await opts.workspace.clear(params.sessionKey);
    await patchAllForms(params.sessionKey, { mountState: "unmounted" });
    return { unmounted: result.unmounted };
  });

  opts.registerGatewayMethod("secafs.session.list", async () => {
    const mk = opts.mainKey ?? "main";
    const entries = await opts.sessions.entries();
    let mounts: Array<{ conversationId: string; hostPath: string; since: string }> = [];
    let daemonReachable = true;
    try {
      mounts = (await opts.rpc.list()).mounts;
    } catch {
      daemonReachable = false;
    }
    const mountBySid = new Map(mounts.map((m) => [m.conversationId, m]));
    // A conversation may exist under several key forms (bare + canonical);
    // group them by sid and merge. A group is a SecAFS session if ANY form
    // carries the kind marker, was spawned by this plugin, or points its
    // workspace into our mount root (markers can survive on either form —
    // the first agent run migrates bare→canonical without merging fields).
    const groups = new Map<
      string,
      { alias?: string; isSecafs: boolean; updatedAt: number; lastInteractionAt: number }
    >();
    for (const [key, e] of Object.entries(entries)) {
      const sid = sidFromKey(key);
      if (sid === key) continue;
      const g = groups.get(sid) ?? { isSecafs: false, updatedAt: 0, lastInteractionAt: 0 };
      const entry = e as {
        kind?: string;
        alias?: string;
        spawnedBy?: string;
        spawnedWorkspaceDir?: string;
        updatedAt?: number;
        lastInteractionAt?: number;
      };
      if (
        entry.kind === "secafs" ||
        entry.spawnedBy?.endsWith(":secafs-chat") ||
        entry.spawnedWorkspaceDir?.startsWith(opts.mountRoot + path.sep)
      ) {
        g.isSecafs = true;
      }
      if (entry.alias && !g.alias) g.alias = entry.alias;
      g.updatedAt = Math.max(g.updatedAt, Number(entry.updatedAt ?? 0));
      g.lastInteractionAt = Math.max(g.lastInteractionAt, Number(entry.lastInteractionAt ?? 0));
      groups.set(sid, g);
    }
    const sessions = [];
    for (const [sid, g] of groups) {
      if (!g.isSecafs) continue;
      const m = mountBySid.get(sid);
      sessions.push({
        sid,
        sessionKey: `${mk}:${sid}`,
        alias: g.alias ?? null,
        mounted: Boolean(m),
        hostPath: m?.hostPath ?? path.join(opts.mountRoot, sid),
        updatedAt: g.updatedAt || null,
        lastInteractionAt: g.lastInteractionAt || null,
      });
    }
    sessions.sort(
      (a, b) =>
        Math.max(b.lastInteractionAt ?? 0, b.updatedAt ?? 0) -
        Math.max(a.lastInteractionAt ?? 0, a.updatedAt ?? 0),
    );
    return { daemonReachable, sessions };
  });

  opts.registerGatewayMethod("secafs.session.rename", async (raw: unknown) => {
    const params = raw as { sessionKey: string; alias?: string };
    const sid = sidFromKey(params.sessionKey);
    const alias = normalizeAlias(params.alias);
    const mk = opts.mainKey ?? "main";
    const aid = opts.defaultAgentId ?? "main";
    // Patch every form that may exist so the alias survives the bare→canonical
    // migration of the first agent run. `null` tombstones a cleared alias
    // (undefined would be dropped by the pluginExtensions fold and the old
    // alias would resurface).
    const keys = [...new Set([`${mk}:${sid}`, `agent:${aid}:${mk}:${sid}`, params.sessionKey])];
    for (const key of keys) {
      await opts.sessions.patch(key, { alias: alias ?? null });
    }
    return { sid, alias: alias ?? null };
  });

  opts.registerGatewayMethod("secafs.session.export", async (raw: unknown) => {
    const params = raw as { sessionKey: string };
    const sid = sidFromKey(params.sessionKey);
    const hostPath = path.join(opts.mountRoot, sid);
    // Idempotent ensure-mounted: export must read the live volume, not a
    // phantom mountpoint dir.
    await opts.rpc.mount({ conversationId: sid, hostPath });
    const files = await collectExportFiles(hostPath);
    const chat = (await opts.readTranscripts?.(sid)) ?? {};
    // alias + rollback flag from whichever key form carries them
    let alias: string | null = null;
    let rollbackEnabled = false;
    const entries = await opts.sessions.entries();
    for (const key of buildKeyForms(params.sessionKey)) {
      const e = entries[key] as
        | { alias?: string; secafsRollback?: { enabled?: boolean } }
        | undefined;
      if (e?.alias && !alias) alias = e.alias;
      if (e?.secafsRollback?.enabled) rollbackEnabled = true;
    }
    const totalBytes = files.reduce((n, f) => n + Buffer.byteLength(f.content, "base64"), 0);
    const archive: SessionArchive = {
      manifest: {
        schemaVersion: 1,
        sid,
        alias,
        exportedAt: new Date().toISOString(),
        rollbackEnabled,
        fileCount: files.length,
        totalBytes,
      },
      files,
      chat,
    };
    return archive;
  });

  opts.registerGatewayMethod("secafs.session.import", async (raw: unknown) => {
    const params = raw as { archive?: SessionArchive; alias?: string };
    const archive = params.archive;
    if (!archive || archive.manifest?.schemaVersion !== 1 || !Array.isArray(archive.files)) {
      throw new Error("invalid archive: expected {manifest:{schemaVersion:1}, files:[...]}");
    }
    let importBytes = 0;
    for (const f of archive.files) {
      if (typeof f?.path !== "string" || typeof f?.content !== "string") {
        throw new Error("invalid archive: each file needs {path, content}");
      }
      importBytes += Buffer.byteLength(f.content, "base64");
      if (importBytes > MAX_ARCHIVE_BYTES) {
        throw new Error(`archive exceeds ${MAX_ARCHIVE_BYTES / (1024 * 1024)}MB limit`);
      }
    }
    // Import is always a fresh copy: new sid, never overwrites an existing
    // session. NOTE: unlike secafs.session.create this does NOT seed from the
    // default workspace — the archive is the complete workspace state.
    const { sessionKey } = await opts.sessions.create({ kind: "secafs" });
    const sid = sidFromKey(sessionKey);
    const hostPath = path.join(opts.mountRoot, sid);
    const result = await opts.rpc.mount({ conversationId: sid, hostPath });
    let filesWritten = 0;
    for (const f of archive.files) {
      // writeSecafsFile owns the path-escape guard + mkdir -p
      await writeSecafsFile(opts.mountRoot, sid, f.path, f.content, { encoding: "base64" });
      filesWritten += 1;
    }
    if (opts.writeTranscripts && (archive.chat?.sessionJsonl || archive.chat?.trajectoryJsonl)) {
      await opts.writeTranscripts(sid, archive.chat);
    }
    // Same ordering as create: canonical entry must exist before
    // workspace.set (update-only patches). sessionFile points at the
    // imported transcript so the conversation continues where it left off.
    const mk2 = opts.mainKey ?? "main";
    const aid2 = opts.defaultAgentId ?? "main";
    const canonicalKey = `agent:${aid2}:${mk2}:${sid}`;
    if (canonicalKey !== sessionKey && opts.ensureSessionEntry) {
      const sessionFile = opts.sessionFileFor?.(sid);
      await opts.ensureSessionEntry(canonicalKey, {
        sessionId: sid,
        ...(sessionFile ? { sessionFile } : {}),
        updatedAt: Date.now(),
      });
    }
    await opts.workspace.set(sessionKey, { path: result.hostPath });
    const alias = normalizeAlias(params.alias) ?? normalizeAlias(archive.manifest.alias);
    const marker = {
      kind: "secafs",
      mountState: "mounted",
      ...(alias ? { alias } : {}),
    };
    await opts.sessions.patch(sessionKey, marker);
    if (canonicalKey !== sessionKey) {
      await opts.sessions.patch(canonicalKey, marker);
    }
    opts.logger?.info?.(
      `[secafs-chat] imported session ${sessionKey} (${filesWritten} files) → ${result.hostPath}`,
    );
    return { sessionKey, hostPath: result.hostPath, filesWritten, alias: alias ?? null };
  });

  opts.registerGatewayMethod("secafs.tree", async (raw: unknown) => {
    const params = raw as { sessionKey: string; maxEntries?: number; maxDepth?: number };
    const sid = sidFromKey(params.sessionKey);
    const hostPath = path.join(opts.mountRoot, sid);
    const maxEntries = Math.max(1, Math.min(params.maxEntries ?? 500, 5000));
    const maxDepth = Math.max(1, Math.min(params.maxDepth ?? 6, 12));
    const entries = await readTree({ root: hostPath, maxEntries, maxDepth });
    return { hostPath, entries };
  });

  // Read a file's contents from the mounted volume (host FUSE mount). Used by
  // the standalone frontend to open files. Path is relative to the volume root
  // and is confined to it (see fs-methods.resolveMountedPath).
  opts.registerGatewayMethod("secafs.fs.read", async (raw: unknown) => {
    const p = raw as {
      sessionKey: string;
      path: string;
      encoding?: "utf8" | "base64";
      maxBytes?: number;
    };
    const sid = sidFromKey(p.sessionKey);
    return readSecafsFile(opts.mountRoot, sid, p.path, {
      encoding: p.encoding,
      maxBytes: p.maxBytes,
    });
  });

  // Write a file's contents to the mounted volume (creating parent dirs).
  opts.registerGatewayMethod("secafs.fs.write", async (raw: unknown) => {
    const p = raw as {
      sessionKey: string;
      path: string;
      content: string;
      encoding?: "utf8" | "base64";
    };
    const sid = sidFromKey(p.sessionKey);
    return writeSecafsFile(opts.mountRoot, sid, p.path, p.content, { encoding: p.encoding });
  });

  if (opts.enableRollbackUI !== false) {
    opts.registerGatewayMethod("secafs.rollback.setEnabled", async (raw: unknown) => {
      const p = raw as { sessionKey?: string; enabled?: boolean } | undefined;
      if (!p?.sessionKey || typeof p.enabled !== "boolean") {
        throw new Error("sessionKey and enabled are required");
      }
      const conversationId = sidFromKey(p.sessionKey);
      // Persist the enabled flag so the snapshot hook (rollback-hook.ts) and
      // rollback.list reflect the current state. Write to both key forms.
      const existingEntry = await loadAnyForm(p.sessionKey);
      const existingRollback = existingEntry?.secafsRollback ?? {};
      if (p.enabled) {
        const r = await opts.rpc.snapshotEnable({ conversationId });
        await patchAllForms(p.sessionKey, {
          secafsRollback: { ...existingRollback, enabled: r.enabled },
        });
        return { enabled: r.enabled };
      }
      const r = await opts.rpc.snapshotDisable({ conversationId });
      // Disable purges per D5 — clear lastSnapshotMessageId and any inProgress marker.
      await patchAllForms(p.sessionKey, {
        secafsRollback: { enabled: false },
      });
      return {
        enabled: false,
        purgedSnapshots: r.purgedSnapshots,
        purgedUndoRows: r.purgedUndoRows,
      };
    });

    opts.registerGatewayMethod("secafs.rollback.list", async (raw: unknown) => {
      const p = raw as { sessionKey?: string } | undefined;
      if (!p?.sessionKey) {
        throw new Error("sessionKey is required");
      }
      const conversationId = sidFromKey(p.sessionKey);
      const list = await opts.rpc.snapshotList({ conversationId });
      const entry = await loadAnyForm(p.sessionKey);
      const enabled = entry?.secafsRollback?.enabled === true;
      return {
        enabled,
        snapshots: list.snapshots.map((s) => ({
          snapId: s.snapId,
          messageId: s.label,
          committedAt: s.committedAt,
        })),
      };
    });

    opts.registerGatewayMethod("secafs.rollback.restore", async (raw: unknown) => {
      const p = raw as { sessionKey?: string; snapId?: number } | undefined;
      if (!p?.sessionKey || typeof p.snapId !== "number") {
        throw new Error("sessionKey and snapId are required");
      }
      return await opts.handleRestore({ sessionKey: p.sessionKey, snapId: p.snapId });
    });

    if (opts.snapshotNow) {
      const snapshotNow = opts.snapshotNow;
      opts.registerGatewayMethod("secafs.rollback.snapshot", async (raw: unknown) => {
        const p = raw as { sessionKey?: string } | undefined;
        if (!p?.sessionKey) {
          throw new Error("sessionKey is required");
        }
        return await snapshotNow({ sessionKey: p.sessionKey });
      });
    }
  }

  opts.registerGatewayMethod("secafs.session.destroy", async (raw: unknown) => {
    const params = raw as { sessionKey: string };
    const sid = sidFromKey(params.sessionKey);
    try {
      await opts.rpc.unmount({ conversationId: sid });
    } catch {
      /* ignore if not mounted */
    }
    await opts.rpc.destroy({ conversationId: sid });
    // Destroy semantics cover the conversation's chat record too: delete the
    // transcript (.jsonl/.trajectory.jsonl) files. Must run BEFORE the store
    // entries are deleted — the transcript paths live in entry.sessionFile.
    if (opts.deleteSessionArtifacts) {
      try {
        await opts.deleteSessionArtifacts(sid);
      } catch (e) {
        opts.logger?.warn?.(
          `[secafs-chat] destroy: transcript cleanup failed for ${sid}: ${
            e instanceof Error ? e.message : String(e)
          }`,
        );
      }
    }
    // Delete both forms of the session key so the conversation disappears
    // from sessions.list entirely. Otherwise the agent-side entry (or the
    // bare entry, depending on which form the caller passed) lingers and
    // continued chat input would re-bootstrap a fresh empty workspace.
    const mk = opts.mainKey ?? "main";
    const aid = opts.defaultAgentId ?? "main";
    const bareKey = `${mk}:${sid}`;
    const canonicalKey = `agent:${aid}:${mk}:${sid}`;
    const keys = [...new Set([bareKey, canonicalKey, params.sessionKey])];
    // Clear ALL forms before deleting ANY: workspace.clear patches every key
    // form, so interleaving clear+delete per key recreates entries deleted in
    // a previous iteration (observed as immortal husk entries after destroy).
    for (const key of keys) {
      try {
        await opts.workspace.clear(key);
      } catch {
        /* nothing to clear; continue */
      }
    }
    for (const key of keys) {
      try {
        await opts.sessions.delete(key);
      } catch {
        /* missing entry; continue */
      }
    }
    return { destroyed: true };
  });
}
