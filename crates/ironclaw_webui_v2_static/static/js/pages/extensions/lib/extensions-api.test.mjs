import assert from "node:assert/strict";
import test from "node:test";

import {
  activateExtension,
  fetchExtensionRegistry,
  fetchExtensions,
  fetchExtensionSetup,
  installExtension,
  removeExtension,
  startExtensionOauth,
  submitExtensionSetup,
} from "./extensions-api.js";

function installFetch(t, handler) {
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  const originalDateNow = Date.now;
  t.after(() => {
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
    Date.now = originalDateNow;
  });

  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = handler;
}

test("extension registry and list calls stay on v2 extension routes", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({ path, options });
    const body = path.endsWith("/registry")
      ? { packages: [] }
      : { extensions: [] };
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  assert.deepEqual(await fetchExtensions(), { extensions: [] });
  assert.deepEqual(await fetchExtensionRegistry(), { packages: [] });
  assert.deepEqual(
    calls.map((call) => call.path),
    ["/api/webchat/v2/extensions", "/api/webchat/v2/extensions/registry"],
  );
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  assert.equal(calls[1].options.headers.get("Authorization"), "Bearer token-1");
});

test("extension lifecycle mutations encode package ids and use v2 routes", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({
      path,
      options,
      body: options.body ? JSON.parse(options.body) : null,
    });
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  await installExtension({ kind: "extension", id: "pkg/needs encoding" });
  await activateExtension({ id: "pkg/needs encoding" });
  await removeExtension({ id: "pkg/needs encoding" });

  assert.deepEqual(
    calls.map((call) => [call.path, call.options.method]),
    [
      ["/api/webchat/v2/extensions/install", "POST"],
      ["/api/webchat/v2/extensions/pkg%2Fneeds%20encoding/activate", "POST"],
      ["/api/webchat/v2/extensions/pkg%2Fneeds%20encoding/remove", "POST"],
    ],
  );
  assert.deepEqual(calls[0].body, {
    package_ref: { kind: "extension", id: "pkg/needs encoding" },
  });
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  assert.equal(calls[1].options.headers.get("Authorization"), "Bearer token-1");
  assert.equal(calls[2].options.headers.get("Authorization"), "Bearer token-1");
});

test("extension setup submit posts action payload through the v2 setup route", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({
      path,
      options,
      body: options.body ? JSON.parse(options.body) : null,
    });
    return new Response(JSON.stringify({ configured: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  assert.deepEqual(await fetchExtensionSetup("slack"), { configured: true });
  await submitExtensionSetup(
    { id: "slack" },
    { bot_token: "xoxb-redacted" },
    { team_id: "T0" },
  );

  assert.equal(calls[0].path, "/api/webchat/v2/extensions/slack/setup");
  assert.equal(calls[0].options.method, undefined);
  assert.equal(calls[1].path, "/api/webchat/v2/extensions/slack/setup");
  assert.equal(calls[1].options.method, "POST");
  assert.deepEqual(calls[1].body, {
    action: "submit",
    payload: {
      secrets: { bot_token: "xoxb-redacted" },
      fields: { team_id: "T0" },
    },
  });
});

test("extension OAuth start builds provider payload without using legacy routes", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({ path, options, body: JSON.parse(options.body) });
    return new Response(JSON.stringify({ authorization_url: "https://auth.example" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
  Date.now = () => Date.parse("2026-06-27T00:00:00.000Z");

  await startExtensionOauth(
    { id: "notion" },
    {
      provider: "notion",
      setup: {
        account_label: "Docs workspace",
        scopes: ["read", "write"],
        invocation_id: "invocation-1",
      },
    },
  );

  assert.equal(
    calls[0].path,
    "/api/webchat/v2/extensions/notion/setup/oauth/start",
  );
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(calls[0].body, {
    provider: "notion",
    account_label: "Docs workspace",
    scopes: ["read", "write"],
    expires_at: "2026-06-27T00:10:00.000Z",
    invocation_id: "invocation-1",
  });
});

test("extension api helpers fail before fetch when package ids are missing", async (t) => {
  let fetchCalled = false;
  installFetch(t, async () => {
    fetchCalled = true;
    throw new Error("fetch should not be called");
  });

  assert.throws(() => activateExtension({}), /package_ref is required/);
  assert.throws(() => removeExtension(), /package_ref is required/);
  assert.throws(() => fetchExtensionSetup({ id: "" }), /package_ref is required/);
  assert.equal(fetchCalled, false);
});
