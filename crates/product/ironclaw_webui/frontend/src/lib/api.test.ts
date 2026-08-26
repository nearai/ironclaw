// @ts-nocheck
import assert from "node:assert/strict";
import { afterEach, test } from "vitest";

import {
  attachmentUrl,
  clientActionId,
  deleteAutomation,
  deleteThread,
  eventStreamRequest,
  fetchAttachmentBlob,
  fetchAttachmentDataUrl,
  listAutomations,
  listThreads,
  pauseAutomation,
  queryLogs,
  queryOperatorLogs,
  renameAutomation,
  resumeAutomation,
  runAutomation,
  getNotificationSetupStatus,
  setNotificationChannels,
  setupExtension,
  enableNotificationSetup,
  disableNotificationSetup,
} from "./api";

const originalFetch = globalThis.fetch;
const originalSessionStorage = globalThis.sessionStorage;

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.sessionStorage = originalSessionStorage;
});

function withCryptoGlobal(replacement, run) {
  const prior = Object.getOwnPropertyDescriptor(globalThis, "crypto");
  Object.defineProperty(globalThis, "crypto", {
    value: replacement,
    configurable: true,
    writable: true,
  });
  const restore = () => {
    if (prior) {
      Object.defineProperty(globalThis, "crypto", prior);
    } else {
      delete globalThis.crypto;
    }
  };
  try {
    const result = run();
    if (result && typeof result.then === "function") {
      return result.then(
        (value) => {
          restore();
          return value;
        },
        (error) => {
          restore();
          throw error;
        },
      );
    }
    restore();
    return result;
  } catch (error) {
    restore();
    throw error;
  }
}

test("listAutomations reads through the v2 automations route", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.window = { location: { origin: "http://localhost" } };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ automations: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const response = await listAutomations({ limit: 50, runLimit: 25 });

  assert.deepEqual(response, { automations: [] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/automations?limit=50&run_limit=25");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

test("eventStreamRequest keeps the bearer out of the stream URL", () => {
  const sessionStorageDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "sessionStorage",
  );
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
  try {
    globalThis.sessionStorage = {
      getItem: () => "token-1",
      setItem: () => {},
      removeItem: () => {},
    };
    globalThis.window = { location: { origin: "http://localhost" } };

    const request = eventStreamRequest({
      threadId: "thread/needs encoding",
      connectionId: "connection-1",
    });

    assert.equal(
      request.url,
      "http://localhost/api/webchat/v2/threads/thread%2Fneeds%20encoding/events?connection_id=connection-1",
    );
    assert.equal(new URL(request.url).searchParams.has("token"), false);
    assert.deepEqual(request.headers(), { Authorization: "Bearer token-1" });
  } finally {
    restoreGlobal("sessionStorage", sessionStorageDescriptor);
    restoreGlobal("window", windowDescriptor);
  }
});

function restoreGlobal(name, descriptor) {
  if (descriptor) {
    Object.defineProperty(globalThis, name, descriptor);
  } else {
    delete globalThis[name];
  }
}

test("listAutomations propagates api errors from the automations route", async () => {
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

  await assert.rejects(listAutomations({ limit: 50 }), (error) => {
    assert.equal(error.name, "ApiError");
    assert.equal(error.status, 503);
    assert.equal(error.statusText, "Service Unavailable");
    assert.equal(error.body, "temporarily unavailable");
    return true;
  });
});

test("log queries forward abort signals to both caller and operator endpoints", async () => {
  const calls = [];
  const controller = new AbortController();
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.window = { location: { origin: "http://localhost" } };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ logs: { entries: [] } }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await queryLogs({ cursor: "caller-cursor", signal: controller.signal });
  await queryOperatorLogs({ cursor: "operator-cursor", signal: controller.signal });

  assert.equal(calls.length, 2);
  assert.equal(calls[0].path, "/api/webchat/v2/logs?cursor=caller-cursor");
  assert.equal(calls[0].options.signal, controller.signal);
  assert.equal(calls[1].path, "/api/webchat/v2/operator/logs?cursor=operator-cursor");
  assert.equal(calls[1].options.signal, controller.signal);
});

test("listThreads passes pagination and candidate-thread filters", async () => {
  const calls = [];
  const controller = new AbortController();
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ threads: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const response = await listThreads({
    limit: 100,
    candidateThreadId: "thread-active",
    signal: controller.signal,
  });

  assert.deepEqual(response, { threads: [] });
  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/threads?limit=100&candidate_thread_id=thread-active",
  );
  assert.equal(calls[0].options.signal, controller.signal);
});

test("automation mutations use encoded v2 automation routes", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ updated: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await runAutomation({ automationId: "automation/needs encoding" });
  await pauseAutomation({ automationId: "automation/needs encoding" });
  await resumeAutomation({ automationId: "automation/needs encoding" });
  await renameAutomation({
    automationId: "automation/needs encoding",
    name: "Renamed status",
  });
  await deleteAutomation({ automationId: "automation/needs encoding" });

  assert.equal(calls.length, 5);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding/run",
  );
  assert.equal(calls[0].options.method, "POST");
  assert.equal(
    calls[1].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding/pause",
  );
  assert.equal(calls[1].options.method, "POST");
  assert.equal(
    calls[2].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding/resume",
  );
  assert.equal(calls[2].options.method, "POST");
  assert.equal(
    calls[3].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding",
  );
  assert.equal(calls[3].options.method, "POST");
  assert.equal(calls[3].options.body, JSON.stringify({ name: "Renamed status" }));
  assert.equal(
    calls[4].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding",
  );
  assert.equal(calls[4].options.method, "DELETE");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

test("setupExtension includes a client action id", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ success: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await setupExtension("web-access", {
    clientActionId: "setup-action-1",
    action: "submit",
    payload: { fields: {} },
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/extensions/web-access/setup");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    client_action_id: "setup-action-1",
    action: "submit",
    payload: { fields: {} },
  });
});

test("setupExtension serializes a generated client action id", async () => {
  const calls = [];
  const priorCrypto = Object.getOwnPropertyDescriptor(globalThis, "crypto");
  Object.defineProperty(globalThis, "crypto", {
    value: { randomUUID: () => "generated-setup-action" },
    configurable: true,
    writable: true,
  });
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ success: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    await setupExtension("web-access", {
      action: "submit",
      payload: { fields: {} },
    });
  } finally {
    if (priorCrypto) {
      Object.defineProperty(globalThis, "crypto", priorCrypto);
    } else {
      delete globalThis.crypto;
    }
  }

  assert.equal(calls.length, 1);
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    client_action_id: "generated-setup-action",
    action: "submit",
    payload: { fields: {} },
  });
});

test("setNotificationChannels posts the full-replace target_ids body with no other wire fields", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ channels: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await setNotificationChannels({ targetIds: ["slack-dm-alpha", "slack-dm-beta"] });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/outbound/notification-channels");
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    target_ids: ["slack-dm-alpha", "slack-dm-beta"],
  });
});

test("setNotificationChannels rejects an omitted targetIds instead of posting a destructive clear-all", async () => {
  let fetchCalled = false;
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("fetch should not be called");
  };

  // `RebornSetNotificationChannelsRequest` deliberately omits
  // `#[serde(default)]` on `target_ids` so an omitted field is a 400, never
  // an implicit clear-all. A client-side `targetIds ?? []` default defeats
  // that server-side guard entirely: the backend accepts `[]` as a valid,
  // *intentional* full replace, so a caller that simply forgot the argument
  // would silently wipe every stored notification channel.
  await assert.rejects(setNotificationChannels(), /targetIds must be an array/);
  await assert.rejects(setNotificationChannels({}), /targetIds must be an array/);
  await assert.rejects(
    setNotificationChannels({ targetIds: null }),
    /targetIds must be an array/,
  );
  await assert.rejects(
    setNotificationChannels({ targetIds: "slack-dm-alpha" }),
    /targetIds must be an array/,
  );
  assert.equal(fetchCalled, false, "a malformed targetIds must never reach the wire");
});

test("setNotificationChannels sends an explicit empty array as the intentional clear-all", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ channels: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  // An explicit `[]` stays the one supported way to clear the set (spec §7:
  // an empty list clears every notification channel) — the guard above
  // rejects only the *absent* argument.
  await setNotificationChannels({ targetIds: [] });

  assert.equal(calls.length, 1);
  assert.deepEqual(JSON.parse(calls[0].options.body), { target_ids: [] });
});

test("automation state mutations reject before fetch when automation id is missing", async () => {
  let fetchCalled = false;
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("fetch should not be called");
  };

  await assert.rejects(pauseAutomation(), /automationId is required/);
  await assert.rejects(runAutomation(), /automationId is required/);
  await assert.rejects(resumeAutomation({}), /automationId is required/);
  await assert.rejects(renameAutomation({ name: "Renamed" }), /automationId is required/);
  await assert.rejects(
    renameAutomation({ automationId: "automation-alpha" }),
    /name is required/,
  );
  await assert.rejects(deleteAutomation({ automationId: "" }), /automationId is required/);
  assert.equal(fetchCalled, false);
});

test("deleteThread sends DELETE to the encoded thread route", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({ thread_id: "thread/needs encoding", deleted: true }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      }
    );
  };

  const response = await deleteThread({ threadId: "thread/needs encoding" });

  assert.deepEqual(response, {
    thread_id: "thread/needs encoding",
    deleted: true,
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/threads/thread%2Fneeds%20encoding");
  assert.equal(calls[0].options.method, "DELETE");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

test("deleteThread rejects before fetch when thread id is missing", async () => {
  let fetchCalled = false;
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("fetch should not be called");
  };

  await assert.rejects(deleteThread(), /threadId is required/);

  assert.equal(fetchCalled, false);
});

test("attachmentUrl encodes the (thread, message, attachment) triple", () => {
  assert.equal(
    attachmentUrl({ threadId: "t 1", messageId: "m/1", attachmentId: "a:1" }),
    "/api/webchat/v2/threads/t%201/messages/m%2F1/attachments/a%3A1",
  );
});

test("attachmentUrl fails fast when a part is missing", () => {
  // Never build a `.../undefined/...` path that would later carry the bearer.
  assert.throws(() => attachmentUrl({ messageId: "m1", attachmentId: "a1" }));
  assert.throws(() => attachmentUrl({ threadId: "t1", attachmentId: "a1" }));
  assert.throws(() => attachmentUrl({ threadId: "t1", messageId: "m1" }));
  assert.throws(() => attachmentUrl());
});

// Regression: the thumbnail must be a `data:` URL, never a `blob:` object URL.
// The SPA's CSP is `img-src 'self' data:`, so a blob URL was refused and the
// thumbnail never rendered. Reverting to `URL.createObjectURL` would throw here.
test("fetchAttachmentDataUrl returns a data URL and never mints a blob URL", async () => {
  globalThis.window = { location: { origin: "https://app.test" } };
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async () =>
    new Response(new Uint8Array([1, 2, 3, 4]), {
      status: 200,
      headers: { "content-type": "image/png" },
    });
  // Keep the real `URL` constructor (the same-origin guard needs `new URL`);
  // only poison `createObjectURL` so a blob-URL regression fails the test.
  // Save/restore the previous value so we don't leak global state into other
  // tests (order-independence).
  const priorCreateObjectURL = globalThis.URL.createObjectURL;
  globalThis.URL.createObjectURL = () => {
    throw new Error("blob: URLs violate the SPA CSP img-src 'self' data:");
  };
  globalThis.FileReader = class {
    readAsDataURL() {
      this.result = "data:image/png;base64,AQIDBA==";
      if (this.onload) this.onload();
    }
  };

  try {
    const url = await fetchAttachmentDataUrl(
      attachmentUrl({ threadId: "t1", messageId: "m1", attachmentId: "a1" }),
    );
    assert.ok(url.startsWith("data:"), `expected a data URL, got ${url}`);
  } finally {
    globalThis.URL.createObjectURL = priorCreateObjectURL;
  }
});

// The bearer is a critical sink: an off-origin attachment URL must be rejected
// before the token is attached.
test("fetchAttachmentBlob rejects an off-origin URL before sending the bearer", async () => {
  globalThis.window = { location: { origin: "https://app.test" } };
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  let fetchCalled = false;
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("fetch should not be reached for an off-origin URL");
  };

  await assert.rejects(
    fetchAttachmentBlob("https://evil.example/steal"),
    (error) => error.name === "ApiError" && error.status === 400,
  );
  assert.equal(fetchCalled, false);
});

// Regression: on insecure origins (plain-HTTP self-hosting) `crypto.randomUUID`
// is absent while `crypto.getRandomValues` is present but must be called with
// `this === crypto` — an unbound call throws `TypeError: Illegal invocation`
// and every mutating request died before fetch.
test("clientActionId works when only getRandomValues is available (insecure context)", () => {
  const fakeCrypto = {
    getRandomValues(bytes) {
      if (this !== fakeCrypto) {
        throw new TypeError("Illegal invocation");
      }
      for (let i = 0; i < bytes.length; i += 1) {
        bytes[i] = (i * 37 + 11) % 256;
      }
      return bytes;
    },
  };

  withCryptoGlobal(fakeCrypto, () => {
    const id = clientActionId();
    assert.match(id, /^[0-9a-f]{32}$/);
    assert.notEqual(id, "0".repeat(32));
  });
});

test("clientActionId yields distinct non-zero ids without Web Crypto", () => {
  withCryptoGlobal(undefined, () => {
    const first = clientActionId();
    const second = clientActionId();
    assert.match(first, /^[0-9a-f]{32}$/);
    assert.match(second, /^[0-9a-f]{32}$/);
    // The old fallback returned the unfilled zero array: a constant id that
    // made the server dedupe distinct sends as replays of one action.
    assert.notEqual(first, "0".repeat(32));
    assert.notEqual(first, second);
  });
});

test("clientActionId falls back when global crypto is null", () => {
  withCryptoGlobal(null, () => {
    const id = clientActionId();
    assert.match(id, /^[0-9a-f]{32}$/);
    assert.notEqual(id, "0".repeat(32));
  });
});

test("getNotificationSetupStatus reads the channel's generic status route", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(
      JSON.stringify({
        extension_id: "some-channel",
        requires_setup: true,
        enabled: false,
        detail: {
          registration_count: 0,
          registrations: [],
          bootstrap: { vapid_public_key: "k" },
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  };

  const status = await getNotificationSetupStatus({ extensionId: "some-channel" });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/channels/some-channel/notifications");
  assert.equal(status.requires_setup, true);
  assert.equal(status.detail.bootstrap.vapid_public_key, "k");
  await assert.rejects(getNotificationSetupStatus(), /extensionId is required/);
  assert.equal(calls.length, 1, "a missing extension id must never reach fetch");
});

test("enableNotificationSetup posts the channel-opaque payload to the enable route", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ enabled: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await enableNotificationSetup({
    extensionId: "some-channel",
    payload: {
      endpoint: "https://push.example/send/abc",
      keys: { p256dh: "pk", auth: "as" },
      user_agent: "TestBrowser/1.0",
    },
  });

  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/channels/some-channel/notifications/enable",
  );
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    payload: {
      endpoint: "https://push.example/send/abc",
      keys: { p256dh: "pk", auth: "as" },
      user_agent: "TestBrowser/1.0",
    },
  });

  await assert.rejects(enableNotificationSetup(), /extensionId is required/);
  assert.equal(calls.length, 1, "invalid input must never reach fetch");
});

test("disableNotificationSetup posts the channel-opaque payload to the disable route", async () => {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({ path, options });
    return new Response(JSON.stringify({ enabled: false }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  await disableNotificationSetup({
    extensionId: "some-channel",
    payload: { endpoint: "https://push.example/send/abc" },
  });

  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/channels/some-channel/notifications/disable",
  );
  assert.deepEqual(JSON.parse(calls[0].options.body), {
    payload: { endpoint: "https://push.example/send/abc" },
  });
  await assert.rejects(disableNotificationSetup({}), /extensionId is required/);
});
