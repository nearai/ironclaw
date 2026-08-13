#!/usr/bin/env node
/**
 * Operator CLI for IdentyClaw host login / HOLA (IronClaw deploy).
 * Hermes-shaped verbs (underscores) are primary; hyphen forms remain aliases.
 *
 * Usage:
 *   node src/cli.mjs enroll
 *   node src/cli.mjs ensure_session [--api-endpoint URL] [--base URL]
 *   node src/cli.mjs list_sessions
 *   node src/cli.mjs me [--api-endpoint URL]
 *   node src/cli.mjs agents [--limit N]
 *   node src/cli.mjs request METHOD /api/path [--body JSON]
 *   node src/cli.mjs request --method GET --path /api/me/identity
 *   node src/cli.mjs create_hola [--recipient MUNDO]
 *   node src/cli.mjs verify_hola --hola 'HOLA/...' [--expected MUNDO]
 *   node src/cli.mjs info
 */
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  apiRequest,
  createHola,
  ensureSession,
  helperInfo,
  listSessions,
  resolveCredentialsPath,
  verifyHola,
} from "./lib.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const require = createRequire(pathToFileURL(path.join(ROOT, "package.json")));

function usage(code = 0) {
  const text = `idcp — IdentyClaw helpers for IronClaw

Commands:
  enroll
  ensure_session [--base URL] [--api-endpoint URL] [--credentials PATH]
  list_sessions
  me [--base URL] [--api-endpoint URL]
  agents [--limit N]
  request METHOD /api/path [--body JSON] [--base URL] [--no-auth]
  request --method METHOD --path /api/... [--body JSON]
  create_hola [--recipient ID] [--base URL] [--token-id ID]
  verify_hola --hola LINE [--expected ID] [--base URL] [--auth]
  info

Hyphen aliases (ensure-session, create-hola, …) are accepted.
`;
  process.stdout.write(text);
  process.exit(code);
}

function argValue(argv, names) {
  const list = Array.isArray(names) ? names : [names];
  for (const name of list) {
    const i = argv.indexOf(name);
    if (i >= 0) return argv[i + 1];
  }
  return undefined;
}

function hasFlag(argv, name) {
  return argv.includes(name);
}

function normalizeCmd(cmd) {
  const map = {
    "ensure-session": "ensure_session",
    "list-sessions": "list_sessions",
    "create-hola": "create_hola",
    "verify-hola": "verify_hola",
  };
  return map[cmd] || cmd;
}

function nearCredentialsDir() {
  return (
    process.env.IDENTYCLAW_NEAR_CREDENTIALS_DIR ||
    process.env.IRONCLAW_NEAR_CREDENTIALS_DIR ||
    path.join(
      process.env.IRONCLAW_APP_DIR || process.env.IDENTYCLAW_HOME || process.cwd(),
      "secrets",
      "near-credentials"
    )
  );
}

function loadHolaClient() {
  return require(path.join(ROOT, "vendor", "hola-client", "index.js"));
}

function cmdEnroll() {
  const dir = nearCredentialsDir();
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  try {
    fs.chmodSync(dir, 0o700);
  } catch {
    /* best effort */
  }

  const existing = fs.readdirSync(dir).filter((f) => f.endsWith(".json"));
  if (existing.length > 0) {
    return {
      ok: true,
      already: true,
      near_credentials_dir: dir,
      files: existing,
      next: "Human: mint Passport at https://purchase.identyclaw.com with account_id, then: idcp ensure_session",
    };
  }

  const candidates = [
    process.env.GENNEARACCOUNT_BIN,
    "gennearaccount",
    path.join(process.env.HOME || "", "gennearaccount/src/gennearaccount"),
  ].filter(Boolean);

  for (const bin of candidates) {
    const gen = spawnSync(bin, ["gennearaccount", dir], { encoding: "utf8" });
    if (gen.error && gen.error.code === "ENOENT") continue;
    if (gen.status !== 0) {
      return {
        ok: false,
        error: `${bin} failed`,
        stderr: gen.stderr,
        stdout: gen.stdout,
      };
    }
    const files = fs.readdirSync(dir).filter((f) => f.endsWith(".json"));
    let account_id = null;
    if (files[0]) {
      try {
        const raw = JSON.parse(fs.readFileSync(path.join(dir, files[0]), "utf8"));
        account_id = raw.account_id || raw.implicit_account_id;
      } catch {
        /* ignore */
      }
    }
    return {
      ok: true,
      method: bin,
      near_credentials_dir: dir,
      files,
      account_id,
      next_human:
        "Purchase Passport at https://purchase.identyclaw.com with account_id, then: idcp ensure_session && idcp me",
    };
  }

  const { writeNearCredentialsFile } = loadHolaClient();
  const written = writeNearCredentialsFile(dir, { force: false });
  return {
    ok: true,
    method: "hola-client",
    near_credentials_dir: dir,
    files: [path.basename(written.filePath)],
    account_id: written.implicit_account_id,
    next_human:
      "Purchase Passport at https://purchase.identyclaw.com with account_id, then: idcp ensure_session && idcp me",
  };
}

async function main() {
  const argv = process.argv.slice(2);
  const rawCmd = argv[0];
  if (!rawCmd || rawCmd === "-h" || rawCmd === "--help" || rawCmd === "help") usage(0);
  const cmd = normalizeCmd(rawCmd);

  const credentialsPath = argValue(argv, "--credentials");
  const apiEndpoint =
    argValue(argv, ["--base", "--api-endpoint"]) || undefined;

  let result;
  switch (cmd) {
    case "enroll":
      result = cmdEnroll();
      if (result && result.ok === false) {
        process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
        process.exit(1);
      }
      break;
    case "ensure_session":
      result = await ensureSession({ apiEndpoint, credentialsPath });
      break;
    case "list_sessions":
      result = listSessions();
      break;
    case "me":
      result = await apiRequest({
        method: "GET",
        path: "/api/me/identity",
        apiEndpoint,
        auth: true,
        credentialsPath,
      });
      break;
    case "agents": {
      const limit = argValue(argv, "--limit") || "20";
      result = await apiRequest({
        method: "GET",
        path: `/api/agents?limit=${encodeURIComponent(limit)}`,
        auth: false,
      });
      break;
    }
    case "request": {
      // Hermes: idcp request METHOD /api/path
      // Legacy: idcp request --method GET --path /api/...
      let method = argValue(argv, "--method");
      let reqPath = argValue(argv, "--path");
      if (!method || !reqPath) {
        const tokens = [];
        for (let i = 1; i < argv.length; i++) {
          const a = argv[i];
          if (a.startsWith("--")) {
            i += 1; // skip flag value
            continue;
          }
          tokens.push(a);
        }
        if (!method && tokens[0]) method = tokens[0];
        if (!reqPath && tokens[1]) reqPath = tokens[1];
      }
      method = method || "GET";
      if (!reqPath) throw new Error("usage: idcp request METHOD /api/path [--body JSON]");
      const bodyRaw = argValue(argv, "--body");
      const body = bodyRaw ? JSON.parse(bodyRaw) : undefined;
      result = await apiRequest({
        method,
        path: reqPath,
        body,
        apiEndpoint,
        auth: !hasFlag(argv, "--no-auth"),
        credentialsPath,
      });
      break;
    }
    case "create_hola":
      result = await createHola({
        recipient: argValue(argv, "--recipient") || "MUNDO",
        apiEndpoint,
        credentialsPath,
        tokenId: argValue(argv, ["--token-id", "--tokenId"]),
      });
      break;
    case "verify_hola": {
      const hola = argValue(argv, "--hola");
      if (!hola) throw new Error("--hola is required");
      result = await verifyHola({
        hola,
        expectedRecipient: argValue(argv, ["--expected", "--expected-recipient"]),
        apiEndpoint,
        auth: hasFlag(argv, "--auth"),
        credentialsPath,
      });
      break;
    }
    case "info":
      result = helperInfo();
      break;
    case "resolve-credentials":
      // Test/helper: print resolved credentials path without network.
      result = { ok: true, path: resolveCredentialsPath(credentialsPath) };
      break;
    default:
      process.stderr.write(`unknown command: ${rawCmd}\n`);
      usage(1);
  }

  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

main().catch((err) => {
  process.stderr.write(`error: ${err?.message || err}\n`);
  process.exit(1);
});
