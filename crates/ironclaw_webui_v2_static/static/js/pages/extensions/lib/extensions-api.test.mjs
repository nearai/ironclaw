import assert from "node:assert/strict";
import test from "node:test";

import { approvePairingCode, fetchPairingRequests } from "./extensions-api.js";

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

test("fetchPairingRequests returns no pending requests without calling a legacy route", async (t) => {
  let called = false;
  installFetch(t, async () => {
    called = true;
    return new Response("{}", {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });

  const response = await fetchPairingRequests("telegram");

  // Reborn v2 has no admin pending-request queue, and the old /api/pairing route
  // is not mounted — so this resolves empty without any network call.
  assert.deepEqual(response, { requests: [] });
  assert.equal(called, false, "must not call the unmounted /api/pairing route");
});

test("approvePairingCode redeems through the mounted v2 endpoint", async (t) => {
  const calls = [];
  installFetch(t, async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({ provider: "slack", provider_user_id: "U1" }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  });

  const response = await approvePairingCode("telegram", "A1B2C3", {
    threadId: "thread-1",
    requestId: "pairing-gate-1",
  });

  assert.equal(response.success, true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/extensions/pairing/redeem");
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channel: "telegram",
    code: "A1B2C3",
    thread_id: "thread-1",
    request_id: "pairing-gate-1",
  });
});

test("pairing API helpers require a channel", () => {
  assert.throws(() => fetchPairingRequests(""), /Pairing channel is required/);
  assert.throws(() => approvePairingCode("", "A1B2C3"), /Pairing channel is required/);
});
