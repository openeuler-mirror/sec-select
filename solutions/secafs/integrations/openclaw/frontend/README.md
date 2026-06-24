# SecAFS Console (frontend)

A dependency-free, single-file (`index.html`) gateway client that makes the
**SecAFS filesystem the primary surface**. It is a normal OpenClaw **gateway
client** (operator role over the WebSocket protocol) — not an OpenClaw "channel"
and not part of OpenClaw's bundled UI — so the OpenClaw checkout stays pristine.

Full deploy guide: **[../README.md](../README.md)**.

## Run

The console talks to the **bridge** (`../bridge/bridge.mjs`), which also serves
this file and proxies frames to the gateway with write scopes. Start the bridge
(README step 7) and open **http://127.0.0.1:8090**; connect to
`ws://127.0.0.1:8090` with a blank token.

> Why the bridge and not `python3 -m http.server` + a direct gateway connection:
> the gateway grants write scopes only to a paired **device identity** (Ed25519
> challenge), which a browser bearer token can't obtain. The bridge holds the
> local device credential and adds write scopes; the browser stays crypto-free.

## Features

- **Sessions** (left): create with optional alias; list with mount badges
  (🟢 mounted / ⚪ saved-in-DB); per row ✏️ rename, ⬇ export, ⏏ close, 🗑 destroy.
  Clicking a row opens the session (mounts on demand).
- **Chat** (center): one agent turn per send; the agent runs with cwd inside the
  FUSE mount (Path C). History loads from the server on open and after rollback.
- **Per-message rollback**: hover any agent reply → **⏪** to roll workspace
  files *and* chat history back to right after that reply; **🕘** opens the
  restore-point timeline.
- **Files / Editor** (right): browse the tree, open a file, edit, **Save to SecAFS**.
- **Export / Import**: ⬇ downloads a session as a standard `.tar.gz`
  (`manifest.json` + `workspace/**` + `chat/*.jsonl`), built client-side with the
  browser-native `CompressionStream`; **⬆ Import** restores one as a fresh copy
  (works across machines / openGauss instances).

## Gateway methods used

`secafs.status`, `secafs.session.{create,open,close,destroy,list,rename,export,import}`,
`secafs.tree`, `secafs.fs.{read,write}`, `secafs.rollback.{setEnabled,list,restore,snapshot}`,
`agent`, `chat.history`. The bridge whitelists exactly these.

## Notes

- Pure static asset — no build step, no framework. Edit `index.html` directly.
- Agent turns can run for minutes; the client uses a 600s timeout for `agent`
  and a "thinking" placeholder, and surfaces errors inline in the reply bubble.
