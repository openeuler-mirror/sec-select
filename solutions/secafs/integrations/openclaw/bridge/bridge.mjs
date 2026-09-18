// SecAFS local bridge.
//
// Why: the OpenClaw gateway binds operator WRITE scopes to device-signature
// auth (Ed25519 challenge) — a browser bearer token cannot get write. This
// bridge holds the local paired device credential (via the SDK's
// callGatewayFromCli, same path the CLI uses) and proxies the frontend's
// gateway-protocol frames to the gateway with write scopes. The frontend talks
// to the bridge over a plain localhost WS (no browser crypto needed).
//
// Run from anywhere; point it at the openclaw checkout via OPENCLAW_DIR.
//   OPENCLAW_DIR=/abs/openclaw GATEWAY_TOKEN=<gateway.auth.token> node bridge.mjs
// Then open http://127.0.0.1:8090 and connect the frontend to ws://127.0.0.1:8090.
import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, extname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// Resolve deploy paths relative to this file rather than to an absolute
// dev-box path: the bridge has to work from any checkout and any cwd.
// frontend/ is a fixed in-repo sibling of bridge/; the openclaw checkout
// sits beside the secafs one (<workspace>/{openclaw,secafs}) per the
// integration README.
const HERE = dirname(fileURLToPath(import.meta.url));
const OPENCLAW_DIR =
  process.env.OPENCLAW_DIR || join(HERE, "..", "..", "..", "..", "openclaw");
const GATEWAY_URL = process.env.GATEWAY_URL || "ws://127.0.0.1:18789";
const GATEWAY_TOKEN = process.env.GATEWAY_TOKEN || "";
const FRONTEND_DIR = process.env.FRONTEND_DIR || join(HERE, "..", "frontend");
const PORT = Number(process.env.PORT || 8090);

// Without this the server still logs "http+ws on ..." while every page
// request 404s -- a silent failure that reads as a network or auth
// problem from the browser side.
if (!existsSync(join(FRONTEND_DIR, "index.html"))) {
  console.error(
    `[secafs-bridge] FATAL: no index.html under ${FRONTEND_DIR}\n` +
      "  point FRONTEND_DIR at <secafs>/integrations/openclaw/frontend",
  );
  process.exit(1);
}

// The bridge lives outside the openclaw checkout, so resolve openclaw's runtime
// + ws by absolute file URL (their transitive deps still resolve from inside
// openclaw/node_modules because the dist files live there).
const wsMod = await import(pathToFileURL(join(OPENCLAW_DIR, "node_modules/ws/index.js")).href);
const WebSocketServer =
  wsMod.WebSocketServer ?? wsMod.default?.WebSocketServer ?? wsMod.default?.Server ?? wsMod.Server;
const { callGatewayFromCli } = await import(
  pathToFileURL(join(OPENCLAW_DIR, "dist/plugin-sdk/gateway-runtime.js")).href
);

const SECAFS_METHODS = [
  "secafs.status",
  "secafs.session.create",
  "secafs.session.open",
  "secafs.session.close",
  "secafs.session.destroy",
  "secafs.session.list",
  "secafs.session.rename",
  "secafs.session.export",
  "secafs.session.import",
  "secafs.rollback.setEnabled",
  "secafs.rollback.list",
  "secafs.rollback.restore",
  "secafs.rollback.snapshot",
  "secafs.tree",
  "secafs.fs.read",
  "secafs.fs.write",
  "agent",
  "chat.history",
];

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
};

const http = createServer(async (rq, rs) => {
  let p = (rq.url || "/").split("?")[0];
  if (p === "/") p = "/index.html";
  try {
    const buf = await readFile(join(FRONTEND_DIR, p));
    rs.writeHead(200, {
      "content-type": MIME[extname(p)] || "application/octet-stream",
      // dev console iterates fast — a stale cached page talking to a newer
      // bridge/plugin produces confusing half-working behavior
      "cache-control": "no-store",
    });
    rs.end(buf);
  } catch {
    rs.writeHead(404);
    rs.end("not found");
  }
});

const wss = new WebSocketServer({ server: http });
wss.on("connection", (ws) => {
  ws.on("message", async (data) => {
    let f;
    try {
      f = JSON.parse(String(data));
    } catch {
      return;
    }
    if (f.type !== "req") return;
    const reply = (ok, payload, error) =>
      ws.send(JSON.stringify({ type: "res", id: f.id, ok, payload, error }));

    // Fake the handshake: the bridge authenticates downstream, so the frontend
    // sees a write-capable operator session.
    if (f.method === "connect") {
      reply(true, {
        protocol: 4,
        server: { version: "secafs-bridge" },
        features: { methods: SECAFS_METHODS, events: [] },
        auth: { role: "operator", scopes: ["operator.read", "operator.write", "operator.admin"] },
        policy: {},
      });
      return;
    }

    try {
      // Agent turns routinely run multi-minute (tool calls, file writes); a
      // 120s cap made the UI report failure while the run completed fine.
      const opts = {
        url: GATEWAY_URL,
        token: GATEWAY_TOKEN,
        timeout: f.method === "agent" ? "600000" : "120000",
      };
      const extra = {
        scopes: ["operator.read", "operator.write", "operator.admin"],
        expectFinal: f.method === "agent",
      };
      // The gateway 'agent' method requires an idempotencyKey; inject one if
      // the frontend didn't supply it.
      const params =
        f.method === "agent"
          ? { idempotencyKey: randomUUID(), ...(f.params ?? {}) }
          : (f.params ?? {});
      const result = await callGatewayFromCli(f.method, opts, params, extra);
      reply(true, result);
    } catch (e) {
      reply(false, undefined, { message: e?.message || String(e) });
    }
  });
});

http.listen(PORT, "127.0.0.1", () =>
  console.log(`[secafs-bridge] http+ws on http://127.0.0.1:${PORT} -> gateway ${GATEWAY_URL}`),
);
