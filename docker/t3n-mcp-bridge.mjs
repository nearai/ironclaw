#!/usr/bin/env node

import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn } from "node:child_process";

const socketPath = process.env.MCP_SOCKET_PATH || "/var/run/t3n-mcp/t3n-mcp.sock";
const projectDir = process.env.T3N_PROJECT_DIR || "/app";
const builtEntrypoint = path.join(projectDir, "dist/esm/index.js");

try {
  const mcp = JSON.parse(fs.readFileSync(path.join(projectDir, "package.json"), "utf8"));
  const sdkPkg = path.join(projectDir, "node_modules/@terminal3/t3n-sdk/package.json");
  const sdk = JSON.parse(fs.readFileSync(sdkPkg, "utf8"));
  process.stderr.write(`[t3n-mcp-bridge] versions t3n-mcp=${mcp.version} t3n-sdk=${sdk.version}\n`);
} catch { /* non-fatal */ }

function log(message, extra = undefined) {
  if (extra === undefined) {
    process.stderr.write(`[t3n-mcp-bridge] ${message}\n`);
    return;
  }

  process.stderr.write(`[t3n-mcp-bridge] ${message} ${JSON.stringify(extra)}\n`);
}

function createLineParser(onLine) {
  let buffer = "";
  return chunk => {
    buffer += chunk.toString("utf8");

    while (true) {
      const newlineIndex = buffer.indexOf("\n");
      if (newlineIndex === -1) {
        break;
      }

      const line = buffer.slice(0, newlineIndex).trim();
      buffer = buffer.slice(newlineIndex + 1);
      if (line) {
        onLine(line);
      }
    }
  };
}

function wireClient(socket) {
  const command = fs.existsSync(builtEntrypoint) ? "node" : "npx";
  const args = fs.existsSync(builtEntrypoint)
    ? [builtEntrypoint]
    : ["tsx", "src/index.ts"];

  const child = spawn(command, args, {
    cwd: projectDir,
    env: { ...process.env },
    stdio: ["pipe", "pipe", "pipe"],
  });

  log("Spawned Trinity MCP child", { pid: child.pid, command, args });

  const parseSocketInput = createLineParser(line => {
    child.stdin.write(`${line}\n`);
  });
  const parseServerOutput = createLineParser(line => {
    socket.write(`${line}\n`);
  });

  socket.on("data", chunk => {
    parseSocketInput(chunk);
  });

  child.stdout.on("data", chunk => {
    try {
      parseServerOutput(chunk);
    } catch (error) {
      log("Failed to parse Trinity MCP stdout", {
        error: error instanceof Error ? error.message : String(error),
      });
      socket.destroy(error instanceof Error ? error : new Error(String(error)));
    }
  });

  child.stderr.on("data", chunk => {
    process.stderr.write(chunk);
  });

  const shutdown = () => {
    if (!child.killed) {
      child.kill("SIGTERM");
    }
  };

  socket.on("close", shutdown);
  socket.on("error", error => {
    log("Socket error", { error: error.message });
    shutdown();
  });

  child.on("error", error => {
    log("Failed to spawn Trinity MCP child", { error: error.message });
    socket.destroy(error);
  });

  child.on("exit", (code, signal) => {
    log("Trinity MCP child exited", { code, signal });
    socket.end();
  });
}

fs.mkdirSync(path.dirname(socketPath), { recursive: true });
if (fs.existsSync(socketPath)) {
  fs.unlinkSync(socketPath);
}

const server = net.createServer(wireClient);

server.listen(socketPath, () => {
  fs.chmodSync(socketPath, 0o777);
  log("Listening on Unix socket", { socketPath });
});

const cleanup = () => {
  server.close(() => {
    if (fs.existsSync(socketPath)) {
      fs.unlinkSync(socketPath);
    }
    process.exit(0);
  });
};

process.on("SIGINT", cleanup);
process.on("SIGTERM", cleanup);
