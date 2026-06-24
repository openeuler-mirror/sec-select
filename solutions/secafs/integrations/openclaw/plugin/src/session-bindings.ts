export interface SessionEndEvent {
  sessionKey: string;
  // The session's stored workspace override, if any.
  workspace?: { managedBy: string };
}

export interface SessionBindingsDeps {
  rpc: { unmount(p: { conversationId: string }): Promise<{ unmounted: boolean }> };
  sessions: { patch(key: string, p: Record<string, unknown>): Promise<void> };
  logger?: { info?: (msg: string) => void; warn?: (msg: string) => void };
}

/**
 * Invoked on session_end. Unmounts only sessions whose workspace override
 * is managed by secafs-chat. All failures are swallowed (best-effort) but logged.
 */
export async function handleSessionEnd(
  event: SessionEndEvent,
  deps: SessionBindingsDeps,
): Promise<void> {
  if (event.workspace?.managedBy !== "secafs-chat") {
    return;
  }
  const sid = event.sessionKey.split(":")[1] ?? event.sessionKey;
  try {
    await deps.rpc.unmount({ conversationId: sid });
  } catch (e) {
    deps.logger?.warn?.(`[secafs-chat] unmount failed for ${event.sessionKey}: ${String(e)}`);
  }
  try {
    await deps.sessions.patch(event.sessionKey, { mountState: "unmounted" });
  } catch (e) {
    deps.logger?.warn?.(`[secafs-chat] patch failed for ${event.sessionKey}: ${String(e)}`);
  }
  // Success log matters: a silent unmount here once cost a long debugging
  // session (the next direct message hits WorkspaceVanishedError).
  deps.logger?.info?.(`[secafs-chat] session_end: unmounted ${event.sessionKey}`);
}
