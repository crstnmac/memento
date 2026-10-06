#!/usr/bin/env node
/**
 * Poppy — local MCP stdio bridge.
 *
 * Claude Desktop (and other MCP-capable clients) speak JSON-RPC over stdio.
 * Poppy's local MCP server speaks JSON-RPC over HTTP/SSE on 127.0.0.1. This
 * script relays one to the other, so no data ever leaves your Mac and no
 * backend round-trip is needed — the tools run against the local database
 * inside the app.
 *
 * Usage in claude_desktop_config.json:
 *   "mcpServers": {
 *     "poppy": {
 *       "command": "node",
 *       "args": ["/absolute/path/to/mcpStdio.cjs"]
 *     }
 *   }
 *
 * You can also run:  node mcpStdio.cjs --add-to-claude-desktop
 */

const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");

const PORT = Number(process.env.POPPY_MCP_PORT || 3961);
const HOST = "127.0.0.1";
const TOKEN = process.env.POPPY_MCP_TOKEN || "";

if (process.argv.includes("--add-to-claude-desktop")) {
  addToClaudeDesktop();
  return;
}

const CLIENTS = [];
let buffer = "";

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  let idx;
  while ((idx = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, idx).trim();
    buffer = buffer.slice(idx + 1);
    if (!line) continue;
    try {
      const msg = JSON.parse(line);
      handleIncoming(msg);
    } catch (err) {
      writeErr({
        jsonrpc: "2.0",
        error: { code: -32700, message: `Parse error: ${err.message}` },
      });
    }
  }
});

process.stdin.on("end", () => process.exit(0));

function handleIncoming(msg) {
  // Notifications don't get a reply; just forward and drop.
  if (msg.method && msg.method.startsWith("notifications/")) {
    postJson(msg, () => {});
    return;
  }
  // Non-notification request: forward and echo the response back to stdout.
  postJson(msg, (reply) => {
    if (reply) writeOut(reply);
  });
}

function postJson(body, done) {
  const payload = JSON.stringify(body);
  const req = http.request(
    {
      host: HOST,
      port: PORT,
      path: "/messages",
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(payload),
        "Authorization": `Bearer ${TOKEN}`,
      },
    },
    (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8").trim();
        if (!text) return done(null);
        try {
          done(JSON.parse(text));
        } catch {
          // The app answered non-JSON (e.g. a notification 202). Ignore.
          done(null);
        }
      });
    },
  );
  req.on("error", (err) => {
    writeErr({
      jsonrpc: "2.0",
      id: body.id != null ? body.id : null,
      error: {
        code: -32000,
        message: `Could not reach the local Poppy MCP server (${HOST}:${PORT}): ` +
          `${err.message} — is Poppy running?`,
      },
    });
  });
  req.write(payload);
  req.end();
}

function writeOut(msg) {
  process.stdout.write(JSON.stringify(msg) + "\n");
}

function writeErr(msg) {
  // Claude Desktop tolerates error responses framed like a result.
  if (process.stdout.writable) writeOut(msg);
  else CLIENTS.push(msg);
}

function addToClaudeDesktop() {
  const configDir = path.join(
    os.homedir(),
    "Library",
    "Application Support",
    "Claude",
  );
  const configPath = path.join(configDir, "claude_desktop_config.json");
  const script = path.resolve(__filename);
  let config = { mcpServers: {} };
  if (fs.existsSync(configPath)) {
    try {
      config = JSON.parse(fs.readFileSync(configPath, "utf8"));
    } catch {
      // Preserve nothing if the existing file is corrupt; back it up below.
    }
  }
  if (!config.mcpServers) config.mcpServers = {};
  const desired = {
    command: "node",
    args: [script],
    env: {
      POPPY_MCP_PORT: String(PORT),
      POPPY_MCP_TOKEN: TOKEN,
    },
  };
  if (!fs.existsSync(configDir)) fs.mkdirSync(configDir, { recursive: true });
  const backup = `${configPath}.bak`;
  if (fs.existsSync(configPath) && !process.argv.includes("--force")) {
    if (JSON.stringify(config.mcpServers.poppy) === JSON.stringify(desired)) {
      console.log(`Already configured: ${configPath}`);
      return;
    }
  }
  config.mcpServers.poppy = desired;
  if (fs.existsSync(configPath)) {
    fs.copyFileSync(configPath, backup);
  }
  fs.writeFileSync(configPath, JSON.stringify(config, null, 2) + "\n");
  console.log(`Wrote poppy MCP server to ${configPath}`);
  console.log(`Bridge: node ${script} (talks to ${HOST}:${PORT})`);
  console.log(
    "Restart Claude Desktop to pick it up. Everything stays on this Mac.",
  );
}
