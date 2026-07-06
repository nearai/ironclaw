import assert from "node:assert/strict";
import test from "node:test";

import { SLACK_SETUP_PATH, saveSlackSetup } from "./slack-setup-api.js";

test("saveSlackSetup trims secrets and drops whitespace-only values", async () => {
  const calls = [];
  const originalSessionStorage = globalThis.sessionStorage;
  const originalFetch = globalThis.fetch;
  try {
    globalThis.sessionStorage = {
      getItem: () => "token-1",
      setItem: () => {},
      removeItem: () => {},
    };
    globalThis.fetch = async (path, options) => {
      calls.push({ path, options });
      return new Response(JSON.stringify({ configured: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    };

    await saveSlackSetup({
      installation_id: " install ",
      team_id: " T0TEAM ",
      api_app_id: " A0APP ",
      user_id: " user:operator ",
      shared_subject_user_id: "",
      bot_token: " xoxb-secret ",
      signing_secret: "   ",
    });

    assert.equal(calls.length, 1);
    assert.equal(calls[0].path, SLACK_SETUP_PATH);
    assert.equal(calls[0].options.method, "PUT");
    assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
    assert.deepEqual(JSON.parse(calls[0].options.body), {
      installation_id: "install",
      team_id: "T0TEAM",
      api_app_id: "A0APP",
      user_id: "user:operator",
      shared_subject_user_id: null,
      bot_token: "xoxb-secret",
    });
  } finally {
    globalThis.sessionStorage = originalSessionStorage;
    globalThis.fetch = originalFetch;
  }
});
