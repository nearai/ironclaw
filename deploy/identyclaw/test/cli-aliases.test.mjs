import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CLI = path.resolve(__dirname, "../src/cli.mjs");
const ROOT = path.resolve(__dirname, "..");

function runCli(args, env = {}) {
  return spawnSync(process.execPath, [CLI, ...args], {
    encoding: "utf8",
    cwd: ROOT,
    env: { ...process.env, ...env },
  });
}

test("cli accepts underscore ensure_session in usage", () => {
  const r = runCli(["--help"]);
  assert.equal(r.status, 0);
  assert.match(r.stdout, /ensure_session/);
  assert.match(r.stdout, /create_hola/);
  assert.match(r.stdout, /enroll/);
});

test("cli hyphen alias ensure-session reaches ensureSession (fails without creds, not unknown cmd)", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "idcp-cli-"));
  const credDir = path.join(tmp, "secrets", "near-credentials");
  fs.mkdirSync(credDir, { recursive: true, mode: 0o700 });
  const r = runCli(["ensure-session"], {
    IRONCLAW_APP_DIR: tmp,
    IDENTYCLAW_NEAR_CREDENTIALS_DIR: credDir,
    IDENTYCLAW_SESSION_DIR: path.join(tmp, "sessions"),
  });
  // Empty creds dir → error from resolveCredentialsPath, not "unknown command"
  assert.notEqual(r.status, 0);
  assert.doesNotMatch(r.stderr, /unknown command/);
  assert.match(r.stderr, /no \*\.json|credentials|NEAR/i);
});

test("enroll creates near-credentials JSON", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "idcp-enroll-"));
  const credDir = path.join(tmp, "secrets", "near-credentials");
  fs.mkdirSync(path.dirname(credDir), { recursive: true });

  const r = runCli(["enroll"], {
    IRONCLAW_APP_DIR: tmp,
    IDENTYCLAW_NEAR_CREDENTIALS_DIR: credDir,
  });
  assert.equal(r.status, 0, r.stderr || r.stdout);
  const out = JSON.parse(r.stdout);
  assert.equal(out.ok, true);
  assert.ok(out.account_id || (out.files && out.files.length));
  const files = fs.readdirSync(credDir).filter((f) => f.endsWith(".json"));
  assert.ok(files.length >= 1);

  const again = runCli(["enroll"], {
    IRONCLAW_APP_DIR: tmp,
    IDENTYCLAW_NEAR_CREDENTIALS_DIR: credDir,
  });
  assert.equal(again.status, 0, again.stderr || again.stdout);
  const reused = JSON.parse(again.stdout);
  assert.equal(reused.already, true);
});
