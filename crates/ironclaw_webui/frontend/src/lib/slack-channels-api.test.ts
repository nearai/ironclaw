// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  SLACK_ALLOWED_CHANNELS_PATH,
  SLACK_ROUTABLE_SUBJECTS_PATH,
  listSlackAllowedChannels,
  listSlackRoutableSubjects,
  normalizeSlackChannelIds,
  saveSlackAllowedChannels,
} from "./slack-channels-api";

test("normalizeSlackChannelIds trims, dedupes, drops blanks, and sorts", () => {
  assert.deepEqual(normalizeSlackChannelIds([" C0OPS ", "", "C0ENG", "C0OPS"]), [
    "C0ENG",
    "C0OPS",
  ]);
});

test("listSlackRoutableSubjects reads the Reborn team-subject endpoint", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ team_id: "T0", subjects: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const response = await listSlackRoutableSubjects();

  assert.deepEqual(response, { team_id: "T0", subjects: [] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, SLACK_ROUTABLE_SUBJECTS_PATH);
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
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

test("saveSlackAllowedChannels sends explicit selected subjects when present", async () => {
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

  await saveSlackAllowedChannels([
    { channel_id: "C0OPS", subject_user_id: "user:ops-team-agent" },
  ]);

  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channels: [{ channel_id: "C0OPS", subject_user_id: "user:ops-team-agent" }],
  });
});

test("saveSlackAllowedChannels preserves structured rows when one subject is missing", async () => {
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

  await saveSlackAllowedChannels([
    { channel_id: "C0OPS", subject_user_id: "user:slack-channel:abc" },
    { channel_id: "C0NEW", subject_user_id: "" },
  ]);

  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channels: [
      { channel_id: "C0OPS", subject_user_id: "user:slack-channel:abc" },
      { channel_id: "C0NEW", subject_user_id: "" },
    ],
  });
});
