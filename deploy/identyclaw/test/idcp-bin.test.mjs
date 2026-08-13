import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const IDCP = path.resolve(__dirname, "../bin/idcp");

function plan(args) {
  const r = spawnSync(IDCP, args, {
    encoding: "utf8",
    env: { ...process.env, IDCP_PRINT_PLAN: "1" },
  });
  assert.equal(r.status, 0, r.stderr || r.stdout);
  const line = (r.stdout || "").trim().split("\n").pop();
  const [method, route, body] = line.split("\t");
  return { method, route, body: body || "" };
}

test("idcp ensure_session → POST /v1/ensure_session", () => {
  const p = plan(["ensure_session"]);
  assert.equal(p.method, "POST");
  assert.equal(p.route, "/v1/ensure_session");
  assert.equal(p.body, "{}");
});

test("idcp ensure-session hyphen alias", () => {
  const p = plan(["ensure-session", "--base", "https://api-b.example.com"]);
  assert.equal(p.method, "POST");
  assert.equal(p.route, "/v1/ensure_session");
  assert.match(p.body, /api-b\.example\.com/);
});

test("idcp list_sessions → GET /v1/sessions", () => {
  const p = plan(["list_sessions"]);
  assert.equal(p.method, "GET");
  assert.equal(p.route, "/v1/sessions");
});

test("idcp me → GET /v1/me", () => {
  const p = plan(["me"]);
  assert.equal(p.method, "GET");
  assert.equal(p.route, "/v1/me");
});

test("idcp create_hola maps recipient", () => {
  const p = plan(["create_hola", "--recipient", "PEERTOKEN"]);
  assert.equal(p.method, "POST");
  assert.equal(p.route, "/v1/create_hola");
  assert.match(p.body, /"recipient":"PEERTOKEN"/);
});

test("idcp verify_hola maps expected", () => {
  const p = plan(["verify_hola", "--hola", "HOLA/MUNDO/x", "--expected", "MUNDO"]);
  assert.equal(p.method, "POST");
  assert.equal(p.route, "/v1/verify_hola");
  assert.match(p.body, /"hola":"HOLA\/MUNDO\/x"/);
  assert.match(p.body, /"expectedRecipient":"MUNDO"/);
});

test("idcp request METHOD PATH", () => {
  const p = plan(["request", "GET", "/api/agents"]);
  assert.equal(p.method, "POST");
  assert.equal(p.route, "/v1/request");
  assert.match(p.body, /"method":"GET"/);
  assert.match(p.body, /"path":"\/api\/agents"/);
});

test("idcp enroll prints HOST plan without curl", () => {
  const p = plan(["enroll"]);
  assert.equal(p.method, "HOST");
  assert.equal(p.route, "enroll");
});

test("idcp rejects unknown command", () => {
  const r = spawnSync(IDCP, ["nope"], {
    encoding: "utf8",
    env: { ...process.env, IDCP_PRINT_PLAN: "1" },
  });
  assert.notEqual(r.status, 0);
});
