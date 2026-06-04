import assert from "node:assert/strict";
import test from "node:test";

import { fetchAutomations } from "./automations-api.js";

test("fetchAutomations reads through the v2 automations route", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ automations: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const response = await fetchAutomations();

  assert.deepEqual(response, { automations: [] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/automations?limit=50");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

test("fetchAutomations propagates api errors from the automations route", async () => {
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async () =>
    new Response("temporarily unavailable", {
      status: 503,
      statusText: "Service Unavailable",
      headers: { "content-type": "text/plain" },
    });

  await assert.rejects(fetchAutomations(), (error) => {
    assert.equal(error.name, "ApiError");
    assert.equal(error.status, 503);
    assert.equal(error.statusText, "Service Unavailable");
    assert.equal(error.body, "temporarily unavailable");
    return true;
  });
});
