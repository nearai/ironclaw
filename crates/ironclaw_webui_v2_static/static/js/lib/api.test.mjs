import assert from "node:assert/strict";
import test from "node:test";

import { deleteThread, listAutomations } from "./api.js";

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
