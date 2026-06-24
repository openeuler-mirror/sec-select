# Runbook — bare command sequence

Concepts, config details and troubleshooting live in **[README.md](./README.md)**.
This is the copy-paste path once prerequisites are met. Assumes the sibling
layout `<workspace>/{openclaw,secafs}` and that `~/.openclaw/openclaw.json` is
configured per README step 4.

```bash
WS=<workspace>        # dir containing openclaw/ and secafs/

# 1. OpenClaw — pristine, zero changes (one-time)
git -C "$WS" clone https://github.com/openclaw/openclaw.git
git -C "$WS/openclaw" checkout v2026.6.8-alpha.1
( cd "$WS/openclaw" && pnpm install && pnpm build )

# 2. secafs daemon (one-time per source change)
mkdir -p ~/.local/secafs-buildlibs
ln -sf /usr/lib/x86_64-linux-gnu/liblzma.so.5 ~/.local/secafs-buildlibs/liblzma.so
( cd "$WS/secafs/cli" && LIBRARY_PATH=~/.local/secafs-buildlibs cargo build -p secafs --no-default-features )

# 3. plugin (one-time per source change)
( cd "$WS/secafs/integrations/openclaw/plugin" && npm install && npm run build )

# 4. openGauss
( cd "$WS/secafs" && docker compose -f docker-compose.dev.yml --profile opengauss up -d opengauss )

# 5. daemon + gateway (one userns) — leave running
( cd "$WS/secafs/integrations/openclaw/bridge" && nohup bash run-stack.sh > /tmp/secafs-stack.log 2>&1 & )
#    wait for "[secafs-chat] plugin registered" and "gateway ready" in /tmp/secafs-stack.log

# 6. bridge + static frontend — leave running
( cd "$WS/secafs/integrations/openclaw/bridge" && \
  OPENCLAW_DIR="$WS/openclaw" \
  GATEWAY_TOKEN=$(node -e "console.log(require(require('os').homedir()+'/.openclaw/openclaw.json').gateway.auth.token)") \
  PORT=8090 nohup node bridge.mjs > /tmp/secafs-bridge.log 2>&1 & )

# 7. open the console
echo "http://127.0.0.1:8090   (ws://127.0.0.1:8090, blank token → Connect)"
```

## Stop

```bash
for port in 8090 18789; do
  P=$(ss -ltnp 2>/dev/null | grep ":$port" | grep -oP 'pid=\K[0-9]+' | head -1)
  [ -n "$P" ] && kill "$P"
done
pkill -x secafs 2>/dev/null
( cd "$WS/secafs" && docker compose -f docker-compose.dev.yml stop opengauss )
```

## Verified end-to-end

A full SecAFS chat runs through **SecAFS Console (browser) → bridge → OpenClaw
gateway → secafs-chat plugin → secafs daemon → openGauss + FUSE**, with the
MiniMax agent operating **inside the per-conversation FUSE mount** (Path C).
Files the agent creates persist in openGauss and appear in the file tree;
per-message rollback, export/import, and mount/daemon self-heal are all live.
