import assert from "node:assert/strict";
import { afterEach, test } from "vitest";

import {
  getExtensionPairingStatus,
  mintExtensionPairingCode,
  unpairExtension,
} from "./extension-pairing-api";

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
  if (originalSessionStorage) {
    Object.defineProperty(globalThis, "sessionStorage", originalSessionStorage);
  } else {
    Reflect.deleteProperty(globalThis, "sessionStorage");
  }
});

test("pairing endpoints decode their successful wire payloads", async () => {
  setSessionStorage();
  const calls: Array<{ path: RequestInfo | URL; options?: RequestInit }> = [];
  const responses = [
    new Response(
      JSON.stringify({
        code: "PAIR-123",
        deep_link: "https://example.test/pair",
        expires_at: "2026-09-02T10:00:00Z",
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
    new Response(JSON.stringify({ connected: false }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
    new Response(null, { status: 204 }),
  ];
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    const response = responses.shift();
    if (!response) throw new Error("unexpected fetch");
    return response;
  };

  assert.deepEqual(await mintExtensionPairingCode("telegram"), {
    code: "PAIR-123",
    deep_link: "https://example.test/pair",
    expires_at: "2026-09-02T10:00:00Z",
  });
  assert.deepEqual(await getExtensionPairingStatus("telegram"), {
    connected: false,
    pending: null,
  });
  assert.equal(calls[1].options?.cache, "no-store");
  assert.equal(await unpairExtension("telegram"), undefined);
});

test("pairing endpoints reject malformed successful responses", async () => {
  setSessionStorage();
  const payloads: unknown[] = [
    { deep_link: null, expires_at: "2026-09-02T10:00:00Z" },
    { connected: "false", pending: null },
    {
      connected: false,
      pending: { code: "PAIR-123", expires_at: 123 },
    },
  ];
  globalThis.fetch = async () =>
    new Response(JSON.stringify(payloads.shift()), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  await assert.rejects(
    mintExtensionPairingCode("telegram"),
    /invalid pairing code response/,
  );
  await assert.rejects(
    getExtensionPairingStatus("telegram"),
    /invalid pairing status response/,
  );
  await assert.rejects(
    getExtensionPairingStatus("telegram"),
    /invalid pairing status response/,
  );
});
