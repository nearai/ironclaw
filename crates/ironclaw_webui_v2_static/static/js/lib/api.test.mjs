import assert from "node:assert/strict";
import test from "node:test";

import {
  attachmentUrl,
  deleteThread,
  fetchAttachmentBlob,
  fetchAttachmentDataUrl,
  listAutomations,
  pauseAutomation,
  resumeAutomation,
} from "./api.js";

test("listAutomations reads through the v2 automations route", async () => {
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

  const response = await listAutomations({ limit: 50, runLimit: 25 });

  assert.deepEqual(response, { automations: [] });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/webchat/v2/automations?limit=50&run_limit=25");
  assert.equal(calls[0].options.credentials, "same-origin");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
});

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

test("pauseAutomation and resumeAutomation post to encoded v2 automation routes", async () => {
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

  await pauseAutomation({ automationId: "automation/needs encoding" });
  await resumeAutomation({ automationId: "automation/needs encoding" });

  assert.equal(calls.length, 2);
  assert.equal(
    calls[0].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding/pause",
  );
  assert.equal(calls[0].options.method, "POST");
  assert.equal(
    calls[1].path,
    "/api/webchat/v2/automations/automation%2Fneeds%20encoding/resume",
  );
  assert.equal(calls[1].options.method, "POST");
  assert.equal(calls[0].options.headers.get("Authorization"), "Bearer token-1");
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
  await assert.rejects(resumeAutomation({}), /automationId is required/);
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
