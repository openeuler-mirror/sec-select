---
title: SecAFS Rollback
description: Per-conversation rollback for SecAFS Chat sessions.
---

SecAFS Chat supports a per-conversation rollback mode. When enabled, every
assistant turn that completes is paired with a Copy-on-Write snapshot of the
conversation's working directory and KV state. Each turn-final assistant
message in the chat UI gets a "Roll back to here" button.

## Enabling rollback

In a SecAFS Chat session header, click the Rollback toggle. The first
snapshot point is created when the next turn finishes. From then on, every
turn-final assistant message in the chat gains a roll-back button.

## Rolling back

Click the button on any assistant message. The UI confirms the destructive
intent (messages after that point will be deleted, file changes since that
turn will be reverted) and runs the rollback.

After rollback completes:

- The conversation history is truncated to (and including) that message.
- Files in the working directory match the state right after that turn.
- The session id stays the same; you continue typing in the same chat.

## Disabling rollback

Click the toggle when on. The UI confirms how many snapshots will be purged
and the storage they free, then disables rollback for the session.

## Storage

Snapshots are Copy-on-Write. Storage cost is proportional to the bytes you
*change* between snapshots, not the size of your workspace. Unchanged files
cost zero per snapshot. Deleting a large file does cost storage proportional
to that file's size for as long as the snapshot containing it is retained.

When rollback is not enabled for a volume, the v0.6 trigger short-circuits
on a single indexed `fs_volume_state` lookup. Microbenchmarks show overhead
within run-to-run noise.

## Caveats

- Rollback is in-place: you cannot recover the conversation messages or file
  contents that were rolled back. Forking is not supported in v1.
- The first snapshot point appears after the first turn that completes after
  enabling. There is no implicit pre-existing snapshot.
- Disabling rollback purges all existing snapshots for that session.
- Plugin config flag `enableRollbackUI` controls whether the gateway methods
  are registered. Default is `true`.

## Configuration

In `~/.openclaw/openclaw.json` under the secafs-chat plugin entry:

```json
{
  "plugins": {
    "entries": {
      "secafs-chat": {
        "enabled": true,
        "config": {
          "postgresUrl": "postgres://secafs:secafs@localhost:5433/secafs",
          "manageDaemon": true,
          "mountRoot": "/home/me/.local/state/secafs/mounts",
          "enableRollbackUI": true
        }
      }
    }
  }
}
```

Set `enableRollbackUI` to `false` to hide the rollback UI for this
deployment without dropping the v0.6 schema.
