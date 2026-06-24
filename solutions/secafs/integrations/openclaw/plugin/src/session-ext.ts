import type { SecafsRollbackState } from "./rollback-types.js";

/**
 * Persistence codec for the plugin's own session-entry state.
 *
 * Upstream `SessionEntry` does NOT declare the plugin's custom fields. The
 * sanctioned place for plugin-owned session state is
 * `entry.pluginExtensions[<pluginId>]` (see upstream `config/sessions/types.ts`).
 * So on disk we keep these fields under `pluginExtensions["secafs-chat"]`, and
 * in memory we surface them at top level for convenient access (the rest of the
 * plugin reads `entry.kind` / `entry.mountState` / `entry.secafsRollback`).
 *
 * - `hoistSecafsExt` (load): copy the namespaced fields up to top level. Falls
 *   back to any legacy top-level values so stores written before this migration
 *   keep working (and get rewritten into the namespace on the next save).
 * - `foldSecafsExt` (save): move the fields from top level INTO the namespace
 *   and strip the top-level copies, keeping the on-disk entry within the
 *   upstream-typed shape.
 */
export const SECAFS_PLUGIN_ID = "secafs-chat";

export interface SecafsExt {
  kind?: "secafs";
  mountState?: "mounted" | "unmounted" | "stuck";
  secafsRollback?: SecafsRollbackState;
  /** User-facing display name for the conversation (secafs.session.rename). */
  alias?: string;
}

const EXT_KEYS = ["kind", "mountState", "secafsRollback", "alias"] as const;

type AnyEntry = Record<string, unknown> & {
  pluginExtensions?: Record<string, Record<string, unknown>>;
};

function readNamespace(entry: AnyEntry): SecafsExt | undefined {
  const ns = entry.pluginExtensions?.[SECAFS_PLUGIN_ID];
  return ns ? (ns as SecafsExt) : undefined;
}

export function hoistSecafsExt<T>(entry: T): T {
  const e = entry as unknown as AnyEntry | null;
  if (!e || typeof e !== "object") return entry;
  const ns = readNamespace(e);
  if (!ns) return entry; // nothing namespaced; any legacy top-level fields stay as-is
  const next: AnyEntry = { ...e };
  for (const k of EXT_KEYS) {
    if (ns[k] !== undefined) next[k] = ns[k];
  }
  return next as unknown as T;
}

export function foldSecafsExt<T>(entry: T): T {
  const e = entry as unknown as AnyEntry | null;
  if (!e || typeof e !== "object") return entry;
  const topLevel: SecafsExt = {};
  let hasTopLevel = false;
  for (const k of EXT_KEYS) {
    if (e[k] !== undefined) {
      (topLevel as Record<string, unknown>)[k] = e[k];
      hasTopLevel = true;
    }
  }
  const existing = readNamespace(e);
  if (!hasTopLevel && !existing) return entry;
  const merged: SecafsExt = { ...(existing ?? {}), ...topLevel };
  const next: AnyEntry = { ...e };
  for (const k of EXT_KEYS) delete next[k];
  next.pluginExtensions = {
    ...(e.pluginExtensions ?? {}),
    [SECAFS_PLUGIN_ID]: merged as Record<string, unknown>,
  };
  return next as unknown as T;
}

export function hoistSecafsStore<S extends Record<string, unknown>>(store: S): S {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(store)) out[k] = hoistSecafsExt(v);
  return out as S;
}

export function foldSecafsStore<S extends Record<string, unknown>>(store: S): S {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(store)) out[k] = foldSecafsExt(v);
  return out as S;
}
