import assert from "node:assert/strict";
import { afterEach, test } from "vitest";

import {
  apiFetch,
  cancelRun,
  eventStreamRequest,
  executeChatCommand,
  fetchSession,
  fetchTimeline,
  getSessionChannelExtensionId,
  openEventSocket,
  resolveGate,
  setSessionChannelExtensionId,
} from "./api";
import { getNotificationSetupStatus } from "./notification-setup-api";

const originalFetch = globalThis.fetch;
const originalSessionStorage = Object.getOwnPropertyDescriptor(
  globalThis,
  "sessionStorage",
);

function setSessionStorage(): void {
  Object.defineProperty(globalThis, "sessionStorage", {
    configurable: true,
    value: { getItem: () => "", removeItem: () => {}, setItem: () => {} },
  });
}

afterEach(() => {
  globalThis.fetch = originalFetch;
  setSessionChannelExtensionId("");
  if (originalSessionStorage) {
    Object.defineProperty(globalThis, "sessionStorage", originalSessionStorage);
  } else {
    Reflect.deleteProperty(globalThis, "sessionStorage");
  }
});

test("fetchSession rejects malformed success payloads before updating session state", async () => {
  setSessionStorage();
  setSessionChannelExtensionId("existing-channel");
  const responses = [true, {}];
  globalThis.fetch = async () =>
    new Response(JSON.stringify(responses.shift()), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  await assert.rejects(fetchSession(), /invalid session response/);
  assert.equal(getSessionChannelExtensionId(), "existing-channel");
  await assert.rejects(fetchSession(), /invalid session response/);
  assert.equal(getSessionChannelExtensionId(), "existing-channel");
});

test("thread and run routes reject missing path identifiers before URL construction", async () => {
  let fetchCalled = false;
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("fetch should not be called");
  };

  assert.throws(() => eventStreamRequest(), /threadId is required/);
  assert.throws(() => openEventSocket(), /threadId is required/);
  await assert.rejects(fetchTimeline(), /threadId is required/);
  await assert.rejects(
    executeChatCommand({ text: "/help" }),
    /threadId is required/,
  );
  await assert.rejects(cancelRun(), /threadId is required/);
  await assert.rejects(
    resolveGate({ threadId: "thread-1", runId: "run-1" }),
    /gateRef is required/,
  );
  assert.equal(fetchCalled, false);
});

test("apiFetch accepts a JSON record with an own constructor field", async () => {
  setSessionStorage();
  globalThis.fetch = async () =>
    new Response(JSON.stringify({ constructor: "value", result: "ok" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  assert.deepEqual(await apiFetch("/api/webchat/v2/session"), {
    constructor: "value",
    result: "ok",
  });
});

test("notification setup rejects malformed successful responses", async () => {
  setSessionStorage();
  globalThis.fetch = async () =>
    new Response(
      JSON.stringify({
        extension_id: "web-push",
        requires_setup: false,
        enabled: "yes",
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );

  await assert.rejects(
    getNotificationSetupStatus({ extensionId: "web-push" }),
    /invalid notification setup response/,
  );
});
