import assert from "node:assert/strict";
import test from "node:test";

import { SLACK_PAIRING_REDEEM_PATH, redeemSlackPairingCode } from "./slack-pairing-api.js";

test("redeemSlackPairingCode posts Slack codes to the Reborn pairing endpoint", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({ provider: "slack", provider_user_id: "install-alpha:U123" }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );
  };

  const response = await redeemSlackPairingCode("A1B2C3");

  assert.deepEqual(response, {
    success: true,
    provider: "slack",
    provider_user_id: "install-alpha:U123",
    resumeError: false,
    resumedRunCount: 0,
    message: "Slack account connected.",
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, SLACK_PAIRING_REDEEM_PATH);
  assert.equal(calls[0].options.method, "POST");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
  assert.equal(calls[0].options.headers.get("Content-Type"), "application/json");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channel: "slack",
    code: "A1B2C3",
  });
});

test("redeemSlackPairingCode does not send thread/request ids the server ignores", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({
        provider: "slack",
        provider_user_id: "install-alpha:U123",
        resumed_run_count: 2,
      }),
      { status: 200, headers: { "content-type": "application/json" } }
    );
  };

  const response = await redeemSlackPairingCode("A1B2C3", {
    threadId: "thread-1",
    requestId: "pairing-gate-1",
  });

  // Channel connection is a per-user gate: the server resumes every run this
  // caller parked and does not scope resume by thread/request, so the redeem
  // body must not carry identifiers the server would silently drop.
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    channel: "slack",
    code: "A1B2C3",
  });
  assert.equal(response.resumedRunCount, 2);
});

test("redeemSlackPairingCode surfaces a resume fault instead of dropping it", async () => {
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async () =>
    new Response(
      JSON.stringify({
        provider: "slack",
        provider_user_id: "install-alpha:U123",
        resume_error: true,
        resumed_run_count: 0,
      }),
      { status: 200, headers: { "content-type": "application/json" } }
    );

  const response = await redeemSlackPairingCode("A1B2C3");

  // The binding is durable, so the connection succeeded even though resume
  // faulted; the fault is reported, not swallowed.
  assert.equal(response.success, true);
  assert.equal(response.resumeError, true);
});
