import assert from "node:assert/strict";
import test from "node:test";

import {
  approvePairingCode,
  fetchPairingRequests,
} from "./extensions-api.js";

function installFetch(t, handler) {
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  t.after(() => {
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  });

  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = handler;
}

test("fetchPairingRequests reads pending pairing requests for a channel", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ requests: [{ code: "A1B2C3" }] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  const response = await fetchPairingRequests("telegram");

  assert.deepEqual(response, { requests: [{ code: "A1B2C3" }] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/pairing/telegram");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

test("approvePairingCode submits a pairing code for the authenticated user", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ success: true, message: "Pairing approved." }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  const response = await approvePairingCode("telegram", "A1B2C3");

  assert.deepEqual(response, { success: true, message: "Pairing approved." });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/pairing/telegram/approve");
  assert.equal(calls[0].options.method, "POST");
  assert.equal(calls[0].options.headers.get("Content-Type"), "application/json");
  assert.deepEqual(JSON.parse(calls[0].options.body), { code: "A1B2C3" });
});

test("pairing API helpers require a channel", () => {
  assert.throws(() => fetchPairingRequests(""), /Pairing channel is required/);
  assert.throws(() => approvePairingCode("", "A1B2C3"), /Pairing channel is required/);
});
