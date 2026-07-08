// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { SLACK_SETUP_PATH, saveSlackSetup } from "./slack-setup-api";

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

test("saveSlackSetup includes oauth_client_id and oauth_client_secret when provided", async () => {
  const calls = [];
  const originalSessionStorage = globalThis.sessionStorage;
  const originalFetch = globalThis.fetch;
  try {
    globalThis.sessionStorage = {
      getItem: () => "token-2",
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
      installation_id: "install",
      team_id: "T0TEAM",
      api_app_id: "A0APP",
      user_id: "",
      shared_subject_user_id: "",
      bot_token: "xoxb-secret",
      signing_secret: "signing-secret",
      oauth_client_id: " 123.456 ",
      oauth_client_secret: " client-secret ",
    });

    assert.deepEqual(JSON.parse(calls[0].options.body), {
      installation_id: "install",
      team_id: "T0TEAM",
      api_app_id: "A0APP",
      user_id: null,
      shared_subject_user_id: null,
      bot_token: "xoxb-secret",
      signing_secret: "signing-secret",
      oauth_client_id: "123.456",
      oauth_client_secret: "client-secret",
    });
  } finally {
    globalThis.sessionStorage = originalSessionStorage;
    globalThis.fetch = originalFetch;
  }
});

test("saveSlackSetup drops whitespace-only oauth fields", async () => {
  const calls = [];
  const originalSessionStorage = globalThis.sessionStorage;
  const originalFetch = globalThis.fetch;
  try {
    globalThis.sessionStorage = {
      getItem: () => "token-3",
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
      installation_id: "install",
      team_id: "T0TEAM",
      api_app_id: "A0APP",
      user_id: "",
      shared_subject_user_id: "",
      bot_token: "xoxb-secret",
      signing_secret: "signing-secret",
      oauth_client_id: "   ",
      oauth_client_secret: "   ",
    });

    const body = JSON.parse(calls[0].options.body);
    assert.equal(body.oauth_client_id, undefined);
    assert.equal(body.oauth_client_secret, undefined);
  } finally {
    globalThis.sessionStorage = originalSessionStorage;
    globalThis.fetch = originalFetch;
  }
});
