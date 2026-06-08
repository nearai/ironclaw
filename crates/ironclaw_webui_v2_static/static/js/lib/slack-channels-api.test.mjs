import assert from "node:assert/strict";
import test from "node:test";

import {
  SLACK_ALLOWED_CHANNELS_PATH,
  listSlackAllowedChannels,
  normalizeSlackChannelIds,
  saveSlackAllowedChannels,
} from "./slack-channels-api.js";

test("normalizeSlackChannelIds trims, dedupes, drops blanks, and sorts", () => {
  assert.deepEqual(normalizeSlackChannelIds([" C0OPS ", "", "C0ENG", "C0OPS"]), [
    "C0ENG",
    "C0OPS",
  ]);
});

test("listSlackAllowedChannels reads the Reborn allowed-channel endpoint", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ team_id: "T0", channels: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const response = await listSlackAllowedChannels();

  assert.deepEqual(response, { team_id: "T0", channels: [] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, SLACK_ALLOWED_CHANNELS_PATH);
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

test("saveSlackAllowedChannels replaces the Reborn allowed-channel set", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ success: true, channels: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const response = await saveSlackAllowedChannels(["C0OPS", " C0ENG ", "C0OPS"]);

  assert.deepEqual(response, { success: true, channels: [] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, SLACK_ALLOWED_CHANNELS_PATH);
  assert.equal(calls[0].options.method, "PUT");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  assert.equal(calls[0].options.headers.get("Content-Type"), "application/json");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channel_ids: ["C0OPS", " C0ENG ", "C0OPS"],
  });
});
