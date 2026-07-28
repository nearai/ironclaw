// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  TELEGRAM_PAIRING_PATH,
  TELEGRAM_SETUP_PATH,
  clearTelegramSetup,
  disconnectTelegramPairing,
  getTelegramPairing,
  getTelegramSetup,
  saveTelegramSetup,
  startTelegramPairing,
  telegramSetupError,
} from "./telegram-setup-api";

async function withStubbedFetch(run) {
  const calls = [];
  const originalSessionStorage = globalThis.sessionStorage;
  const originalFetch = globalThis.fetch;
  try {
    globalThis.sessionStorage = {
      getItem: () => "token-1",
      setItem: () => {},
      removeItem: () => {},
    };
    globalThis.fetch = async (path, options = {}) => {
      calls.push({ path, options });
      return new Response(JSON.stringify({ configured: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    };
    await run(calls);
  } finally {
    globalThis.sessionStorage = originalSessionStorage;
    globalThis.fetch = originalFetch;
  }
}

test("telegram setup + pairing wire paths target the single telegram channel", () => {
  assert.equal(TELEGRAM_SETUP_PATH, "/api/webchat/v2/channels/telegram/setup");
  assert.equal(TELEGRAM_PAIRING_PATH, "/api/webchat/v2/channels/telegram/pairing");
});

test("getTelegramSetup GETs the setup path with the session bearer", async () => {
  await withStubbedFetch(async (calls) => {
    await getTelegramSetup();
    assert.equal(calls.length, 1);
    assert.equal(calls[0].path, TELEGRAM_SETUP_PATH);
    assert.equal(calls[0].options.method, undefined);
    assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  });
});

test("saveTelegramSetup trims fields and omits a blank bot token so the saved secret is kept", async () => {
  await withStubbedFetch(async (calls) => {
    await saveTelegramSetup({
      bot_token: "   ",
      webhook_url: " https://assistant.example.com ",
    });
    assert.equal(calls[0].path, TELEGRAM_SETUP_PATH);
    assert.equal(calls[0].options.method, "PUT");
    assert.deepEqual(JSON.parse(calls[0].options.body), {
      webhook_url: "https://assistant.example.com",
    });
  });
});

test("saveTelegramSetup sends a trimmed bot token and nulls a cleared webhook override", async () => {
  await withStubbedFetch(async (calls) => {
    await saveTelegramSetup({
      bot_token: " 123456789:AAtoken ",
      webhook_url: "   ",
    });
    assert.deepEqual(JSON.parse(calls[0].options.body), {
      webhook_url: null,
      bot_token: "123456789:AAtoken",
    });
  });
});

test("clearTelegramSetup DELETEs the setup path", async () => {
  await withStubbedFetch(async (calls) => {
    await clearTelegramSetup();
    assert.equal(calls[0].path, TELEGRAM_SETUP_PATH);
    assert.equal(calls[0].options.method, "DELETE");
  });
});

test("pairing helpers POST/GET/DELETE the pairing path", async () => {
  await withStubbedFetch(async (calls) => {
    await startTelegramPairing();
    await getTelegramPairing();
    await disconnectTelegramPairing();
    assert.deepEqual(
      calls.map((call) => [call.path, call.options.method]),
      [
        [TELEGRAM_PAIRING_PATH, "POST"],
        [TELEGRAM_PAIRING_PATH, undefined],
        [TELEGRAM_PAIRING_PATH, "DELETE"],
      ],
    );
    assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  });
});

test("telegramSetupError prefers the wire error body over the thrown message", () => {
  assert.equal(
    telegramSetupError({ payload: { error: "invalid bot token" }, message: "Request failed" }, "fallback"),
    "invalid bot token",
  );
  assert.equal(
    telegramSetupError({ message: "Request failed" }, "fallback"),
    "Request failed",
  );
  assert.equal(telegramSetupError(null, "fallback"), "fallback");
});
