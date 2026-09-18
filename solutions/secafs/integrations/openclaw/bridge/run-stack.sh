#!/usr/bin/env bash
# Bring up secafs daemon + OpenClaw gateway + the Console bridge in ONE user
# namespace so FUSE mounts (created by the daemon) are visible to the agent
# tools the gateway spawns. Run: bash run-stack.sh  (it re-execs itself inside
# unshare).
set -uo pipefail

# Derive the checkout from this script's own location. Absolute paths baked in
# here only work on one box, and a "$WS"-style indirection breaks outright
# under `set -u` when the caller forgot to export it. bridge/ always sits at
# <secafs>/integrations/openclaw/bridge, and the openclaw checkout beside the
# secafs one (<workspace>/{openclaw,secafs}) per the integration README.
HERE="${HERE:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
SECAFS_ROOT="${SECAFS_ROOT:-$(cd "$HERE/../../.." && pwd)}"

SECAFS_BIN_DIR="${SECAFS_BIN_DIR:-$SECAFS_ROOT/cli/target/debug}"
OPENCLAW_DIR="${OPENCLAW_DIR:-$(cd "$SECAFS_ROOT/.." && pwd)/openclaw}"
PG_URL="${PG_URL:-opengauss://secafs:Secafs%21123@localhost:5433/secafs}"
# Persistent runtime dirs (NOT /tmp: systemd-tmpfiles aging + reboot wipe
# would kill the socket/mountpoints during long-running tests).
SOCK="${SOCK:-$HOME/.secafs/run/secafs.sock}"
MOUNT_ROOT="${MOUNT_ROOT:-$HOME/.secafs/mounts}"
BRIDGE_PORT="${BRIDGE_PORT:-8090}"

if [ -z "${_IN_NS:-}" ]; then
  exec unshare --user --map-root-user --mount env _IN_NS=1 \
    HERE="$HERE" SECAFS_ROOT="$SECAFS_ROOT" \
    SECAFS_BIN_DIR="$SECAFS_BIN_DIR" OPENCLAW_DIR="$OPENCLAW_DIR" \
    PG_URL="$PG_URL" SOCK="$SOCK" MOUNT_ROOT="$MOUNT_ROOT" \
    BRIDGE_PORT="$BRIDGE_PORT" bash "$0"
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

# Start the Console bridge here rather than by hand in a second shell: a
# hand-typed relaunch is how its env drifted (a dropped FRONTEND_DIR made every
# Console request 404 while the logs still read "http+ws on ..."). bridge.mjs
# resolves the frontend dir relative to itself, so only the token/port matter.
BRIDGE_PID=""
GATEWAY_TOKEN="$(node -e 'try{console.log(require(require("os").homedir()+"/.openclaw/openclaw.json").gateway.auth.token)}catch(e){}' 2>/dev/null)"
if ss -tln 2>/dev/null | grep -q ":$BRIDGE_PORT "; then
  echo "[run-stack] port $BRIDGE_PORT already serving — leaving the running bridge alone"
elif [ -z "$GATEWAY_TOKEN" ]; then
  echo "[run-stack] WARN: no gateway.auth.token in ~/.openclaw/openclaw.json — bridge not started"
  echo "[run-stack]       run 'openclaw onboard' first, then restart this script"
else
  echo "[run-stack] starting Console bridge on :$BRIDGE_PORT…"
  # Launch it as `node bridge.mjs` from its own directory: an absolute path
  # here would change the cmdline and silently break the pkill/pgrep -f
  # "node bridge.mjs" that the runbook and everyone's muscle memory use.
  (
    cd "$HERE" || exit 1
    OPENCLAW_DIR="$OPENCLAW_DIR" GATEWAY_TOKEN="$GATEWAY_TOKEN" PORT="$BRIDGE_PORT" \
      exec node bridge.mjs
  ) &
  BRIDGE_PID=$!
fi

echo "[run-stack] starting gateway…"
cd "$OPENCLAW_DIR"
pnpm openclaw gateway run --force
[ -n "$BRIDGE_PID" ] && kill "$BRIDGE_PID" 2>/dev/null
kill "$SUPERVISOR_PID" 2>/dev/null
pkill -x secafs 2>/dev/null || true
