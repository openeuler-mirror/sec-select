#!/usr/bin/env bash
# Bring up secafs daemon + OpenClaw gateway in ONE user namespace so FUSE
# mounts (created by the daemon) are visible to the agent tools the gateway
# spawns. Run: bash run-stack.sh  (it re-execs itself inside unshare).
set -uo pipefail

SECAFS_BIN_DIR="${SECAFS_BIN_DIR:-/home/kou/projects/openclaw_secafs/secafs/cli/target/debug}"
OPENCLAW_DIR="${OPENCLAW_DIR:-/home/kou/projects/openclaw_secafs/openclaw}"
PG_URL="${PG_URL:-opengauss://secafs:Secafs%21123@localhost:5433/secafs}"
# Persistent runtime dirs (NOT /tmp: systemd-tmpfiles aging + reboot wipe
# would kill the socket/mountpoints during long-running tests).
SOCK="${SOCK:-$HOME/.secafs/run/secafs.sock}"
MOUNT_ROOT="${MOUNT_ROOT:-$HOME/.secafs/mounts}"

if [ -z "${_IN_NS:-}" ]; then
  exec unshare --user --map-root-user --mount env _IN_NS=1 \
    SECAFS_BIN_DIR="$SECAFS_BIN_DIR" OPENCLAW_DIR="$OPENCLAW_DIR" \
    PG_URL="$PG_URL" SOCK="$SOCK" MOUNT_ROOT="$MOUNT_ROOT" bash "$0"
fi

export PATH="$SECAFS_BIN_DIR:$PATH"
mkdir -p "$MOUNT_ROOT"
echo "[run-stack] starting secafs daemon (socket $SOCK)…"
# Supervise the daemon: a crash takes the FUSE mounts with it, but the plugin
# remounts on demand (session.open / auto-mount hook) — so a respawn loop is
# enough to keep secafs.* methods serviceable without restarting the stack.
(
  while :; do
    secafs serve api --socket "$SOCK" --pg-url "$PG_URL" --mount-root "$MOUNT_ROOT"
    code=$?
    echo "[run-stack] daemon exited (code $code); respawning in 1s…"
    # The dead daemon's FUSE mountpoints linger as disconnected carcasses
    # ("Transport endpoint is not connected") that block remounting.
    grep " $MOUNT_ROOT/" /proc/mounts | awk '{print $2}' | while read -r m; do
      umount -l "$m" 2>/dev/null && echo "[run-stack] detached stale mount $m"
    done
    sleep 1
  done
) &
SUPERVISOR_PID=$!
for i in $(seq 1 30); do [ -S "$SOCK" ] && break; sleep 0.5; done
[ -S "$SOCK" ] && echo "[run-stack] daemon socket up" || echo "[run-stack] WARN: socket not seen"

echo "[run-stack] starting gateway…"
cd "$OPENCLAW_DIR"
pnpm openclaw gateway run --force
kill "$SUPERVISOR_PID" 2>/dev/null
pkill -x secafs 2>/dev/null || true
