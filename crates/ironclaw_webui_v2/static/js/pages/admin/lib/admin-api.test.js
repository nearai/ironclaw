// Unit tests for the WebChat v2 admin user-management API client.
//
// Run with Node's built-in test runner (no extra deps):
//   node --test crates/ironclaw_webui_v2/static/js/pages/admin/lib/admin-api.test.js
//
// NOTE: `build.rs` deliberately excludes `*.test.js` from the embedded
// asset bundle, so this file is never served to the browser.
//
// These tests drive the REAL `apiFetch` (per `.claude/rules/testing.md`
// "Test Through the Caller"): we stub `globalThis.fetch` — the sink
// `apiFetch` ultimately calls — and assert the exact wire request
// (method, path, body) plus the client-side response mapping. Stubbing
// `apiFetch` itself would hide request/response-contract drift and
// argument loss, the exact bug class the rule warns about.

import assert from "node:assert/strict";
import { test, beforeEach, afterEach } from "node:test";
import {
  fetchAdminUsers,
  createAdminUser,
  updateAdminUser,
  suspendAdminUser,
  activateAdminUser,
  fetchUserSecrets,
  putUserSecret,
  deleteUserSecret,
  createUserToken,
} from "./admin-api.js";

// `api.js` reads a bearer via `sessionStorage.getItem`, which does not
// exist in Node. A minimal in-memory stub keeps `apiFetch` running; an
// empty token simply omits the Authorization header.
globalThis.sessionStorage = {
  store: new Map(),
  getItem(key) {
    return this.store.has(key) ? this.store.get(key) : null;
  },
  setItem(key, value) {
    this.store.set(key, String(value));
  },
  removeItem(key) {
    this.store.delete(key);
  },
};

const originalFetch = globalThis.fetch;
let calls;

// Install a fetch stub that records every request and replies with a
// JSON response built by `responder(path, init)`. The real `apiFetch`
// runs on top of this, so `calls` captures the actual wire request.
function stubFetch(responder) {
  calls = [];
  globalThis.fetch = async (path, init) => {
    calls.push({ path, init });
    const body = responder(path, init);
    return {
      ok: true,
      status: 200,
      statusText: "OK",
      headers: new Headers({ "content-type": "application/json" }),
      json: async () => body,
      text: async () => JSON.stringify(body),
    };
  };
}

beforeEach(() => {
  calls = [];
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

// The client must JSON-encode request bodies. `apiFetch` forwards `options.body`
// to `fetch` unchanged (it does NOT serialize — see `lib/api.js`), so a raw
// object body would reach the wire as the string "[object Object]" and the
// backend would reject it. Asserting the body is a serialized string that
// parses back to the expected object locks that in.
function jsonBody(call) {
  assert.equal(
    typeof call.init.body,
    "string",
    "request body must be a JSON string, not a raw object",
  );
  return JSON.parse(call.init.body);
}

test("fetchAdminUsers GETs the users route and normalizes id === user_id", async () => {
  stubFetch(() => ({
    users: [
      { user_id: "u-1", email: "a@example.com" },
      { user_id: "u-2", email: "b@example.com" },
    ],
  }));

  const result = await fetchAdminUsers();

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users");
  // No explicit method => GET.
  assert.equal(calls[0].init.method, undefined);

  assert.equal(result.total, 2);
  assert.deepEqual(
    result.users.map((u) => ({ id: u.id, user_id: u.user_id })),
    [
      { id: "u-1", user_id: "u-1" },
      { id: "u-2", user_id: "u-2" },
    ],
  );
});

test("fetchAdminUsers returns an empty list when the response has no users array", async () => {
  stubFetch(() => ({}));
  const result = await fetchAdminUsers();
  assert.deepEqual(result, { users: [], total: 0 });
});

test("createAdminUser POSTs the payload, defaults role, and surfaces the one-time token", async () => {
  stubFetch(() => ({
    user: { user_id: "u-9", email: "new@example.com", display_name: "New" },
    api_token: "one-time-bearer-abc",
  }));

  const result = await createAdminUser({
    email: "new@example.com",
    display_name: "New",
    // role omitted -> defaults to "member"
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(jsonBody(calls[0]), {
    email: "new@example.com",
    display_name: "New",
    role: "member",
  });

  assert.equal(result.token, "one-time-bearer-abc");
  assert.equal(result.id, "u-9");
  assert.equal(result.user_id, "u-9");
});

test("createAdminUser passes an explicit role through unchanged", async () => {
  stubFetch(() => ({ user: { user_id: "u-10" }, api_token: "tok" }));
  await createAdminUser({ email: "x@example.com", display_name: "X", role: "admin" });
  assert.equal(jsonBody(calls[0]).role, "admin");
});

test("updateAdminUser with { role } POSTs the dedicated role endpoint", async () => {
  stubFetch(() => ({ user: { user_id: "u-1", role: "admin" } }));

  const result = await updateAdminUser("u-1", { role: "admin" });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users/u-1/role");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(jsonBody(calls[0]), { role: "admin" });
  assert.equal(result.id, "u-1");
});

test("updateAdminUser without role PATCHes the base user route with profile fields", async () => {
  stubFetch(() => ({ user: { user_id: "u-2", display_name: "Renamed" } }));

  const result = await updateAdminUser("u-2", {
    display_name: "Renamed",
    metadata: { team: "ops" },
  });

  assert.equal(calls.length, 1);
  // Different branch: base route, PATCH, no `/role` segment.
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users/u-2");
  assert.equal(calls[0].init.method, "PATCH");
  assert.deepEqual(jsonBody(calls[0]), {
    display_name: "Renamed",
    metadata: { team: "ops" },
  });
  assert.equal(result.id, "u-2");
});

test("updateAdminUser URL-encodes the id in both branches", async () => {
  stubFetch(() => ({ user: { user_id: "a b/c" } }));

  await updateAdminUser("a b/c", { role: "member" });
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users/a%20b%2Fc/role");

  await updateAdminUser("a b/c", { display_name: "z" });
  assert.equal(calls[1].path, "/api/webchat/v2/admin/users/a%20b%2Fc");
});

test("suspendAdminUser POSTs the status route with status=suspended", async () => {
  stubFetch(() => ({ user: { user_id: "u-1", status: "suspended" } }));

  const result = await suspendAdminUser("u-1");

  assert.equal(calls.length, 1);
  // Must hit the shared /status route, NOT a v1-shaped /suspend endpoint.
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users/u-1/status");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(jsonBody(calls[0]), { status: "suspended" });
  assert.equal(result.id, "u-1");
});

test("activateAdminUser POSTs the status route with status=active", async () => {
  stubFetch(() => ({ user: { user_id: "u-1", status: "active" } }));

  const result = await activateAdminUser("u-1");

  assert.equal(calls.length, 1);
  // Same /status route as suspend, NOT a v1-shaped /activate endpoint.
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users/u-1/status");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(jsonBody(calls[0]), { status: "active" });
  assert.equal(result.id, "u-1");
});

test("fetchUserSecrets GETs the secrets route and returns the array", async () => {
  stubFetch(() => ({ secrets: [{ handle: "openai_api_key" }] }));

  const result = await fetchUserSecrets("u-1");

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/admin/users/u-1/secrets");
  assert.equal(calls[0].init.method, undefined);
  assert.deepEqual(result, [{ handle: "openai_api_key" }]);
});

test("fetchUserSecrets returns an empty array when secrets are absent", async () => {
  stubFetch(() => ({}));
  assert.deepEqual(await fetchUserSecrets("u-1"), []);
});

test("putUserSecret PUTs to the URL-encoded secret path with the value body", async () => {
  stubFetch(() => ({ secret: { handle: "open ai/key" } }));

  const result = await putUserSecret("u 1", "open ai/key", "sk-secret");

  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/admin/users/u%201/secrets/open%20ai%2Fkey",
  );
  assert.equal(calls[0].init.method, "PUT");
  assert.deepEqual(jsonBody(calls[0]), { value: "sk-secret" });
  assert.deepEqual(result, { handle: "open ai/key" });
});

test("deleteUserSecret DELETEs the same URL-encoded secret path", async () => {
  stubFetch(() => ({ ok: true }));

  await deleteUserSecret("u 1", "open ai/key");

  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/admin/users/u%201/secrets/open%20ai%2Fkey",
  );
  assert.equal(calls[0].init.method, "DELETE");
});

test("createUserToken rejects (re-issue is not yet supported) without any request", async () => {
  stubFetch(() => ({}));
  await assert.rejects(() => createUserToken("u-1", "my token"), /re-issue not yet supported/);
  assert.equal(calls.length, 0);
});
