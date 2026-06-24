#!/usr/bin/env bash
# Run a command inside the secafs daemon's mount namespace, where the
# per-session FUSE mounts are visible (they are invisible from the host
# shell — run-stack.sh starts the stack under unshare --user --mount).
#
#   secafs-ns.sh                          # interactive shell inside the ns
#   secafs-ns.sh ls -la ~/.secafs/mounts/<id>/
#   secafs-ns.sh cat ~/.secafs/mounts/<id>/2.txt
set -euo pipefail

DPID=$(pgrep -x secafs | head -1)
if [ -z "${DPID:-}" ]; then
  echo "secafs daemon is not running (no 'secafs' process found)" >&2
  exit 1
fi
if [ $# -eq 0 ]; then
  exec nsenter -t "$DPID" -m -U --preserve-credentials bash
fi
exec nsenter -t "$DPID" -m -U --preserve-credentials "$@"
