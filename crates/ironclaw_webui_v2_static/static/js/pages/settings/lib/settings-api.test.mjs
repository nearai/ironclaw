import assert from "node:assert/strict";
import test from "node:test";

import {
  authorizeTraceHold,
  completeNearaiWalletLogin,
  createUser,
  fetchLlmProviders,
  fetchSkillContent,
  fetchSkills,
  fetchTraceCredits,
  fetchUsers,
  listLlmProviderModels,
  setActiveLlm,
  startCodexLogin,
  startNearaiLogin,
  testLlmProviderConnection,
  toolFromConfigEntry,
  updateUser,
  upsertLlmProvider,
  settingsFromOperatorConfig,
} from "./settings-api.js";

function installFetchRecorder(responseForPath = () => ({})) {
  const calls = [];
  globalThis.sessionStorage = {
    getItem: () => "settings-token",
    setItem: () => {},
    removeItem: () => {},
  };
  globalThis.fetch = async (path, options) => {
    calls.push({
      path,
      options,
      body: options?.body ? JSON.parse(options.body) : undefined,
    });
    return new Response(JSON.stringify(responseForPath(path, options)), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  return calls;
}

test("settingsFromOperatorConfig maps the global auto-approve key", () => {
  const settings = settingsFromOperatorConfig({
    entries: [
      { key: "agent.auto_approve_tools", value: true },
      { key: "tool.example.run", value: { state: "ask_each_time" } },
    ],
  });

  assert.deepEqual(settings, { "agent.auto_approve_tools": true });
});

test("toolFromConfigEntry maps operator config tools for the tools tab", () => {
  assert.deepEqual(
    toolFromConfigEntry({
      key: "tool.example.run",
      mutable: true,
      source: "global",
      value: {
        name: "example.run",
        description: "Run example",
        state: "always_allow",
        default_state: "ask_each_time",
        locked: false,
        effective_source: "global",
      },
    }),
    {
      name: "example.run",
      description: "Run example",
      state: "always_allow",
      default_state: "ask_each_time",
      locked: false,
      effective_source: "global",
    }
  );
});

test("toolFromConfigEntry normalizes legacy and malformed permission values", () => {
  assert.deepEqual(
    toolFromConfigEntry({
      key: "tool.example.ask",
      mutable: false,
      source: "unknown",
      value: {
        state: "ask",
        default_state: "surprise",
      },
    }),
    {
      name: "example.ask",
      description: "",
      state: "ask_each_time",
      default_state: "ask_each_time",
      locked: true,
      effective_source: "default",
    }
  );
});

test("settings LLM provider helpers use v2 routes with bearer-authenticated JSON payloads", async () => {
  const calls = installFetchRecorder((path) => {
    if (path === "/api/webchat/v2/llm/providers") return { providers: [] };
    if (path === "/api/webchat/v2/llm/nearai/login") return { auth_url: "https://near.ai/login" };
    if (path === "/api/webchat/v2/llm/codex/login") return { user_code: "ABCD" };
    return { ok: true };
  });

  await fetchLlmProviders();
  await upsertLlmProvider({ id: "local", name: "Local", default_model: "llama3" });
  await setActiveLlm({ provider_id: "local", model: "llama3" });
  await testLlmProviderConnection({ provider_id: "local" });
  await listLlmProviderModels({ provider_id: "local" });
  await startNearaiLogin({ provider: "google", origin: "https://app.example" });
  await completeNearaiWalletLogin({ account_id: "alice.near", signature: "sig" });
  await startCodexLogin();

  assert.deepEqual(
    calls.map((call) => [call.path, call.options?.method || "GET"]),
    [
      ["/api/webchat/v2/llm/providers", "GET"],
      ["/api/webchat/v2/llm/providers", "POST"],
      ["/api/webchat/v2/llm/active", "POST"],
      ["/api/webchat/v2/llm/test-connection", "POST"],
      ["/api/webchat/v2/llm/list-models", "POST"],
      ["/api/webchat/v2/llm/nearai/login", "POST"],
      ["/api/webchat/v2/llm/nearai/wallet", "POST"],
      ["/api/webchat/v2/llm/codex/login", "POST"],
    ]
  );
  assert.ok(
    calls.every((call) => call.options.credentials === "same-origin"),
    "settings helpers stay on same-origin v2 routes"
  );
  assert.ok(
    calls.every((call) => call.options.headers.get("Authorization") === "Bearer settings-token"),
    "settings helpers propagate the stored bearer"
  );
  assert.deepEqual(calls[1].body, { id: "local", name: "Local", default_model: "llama3" });
  assert.deepEqual(calls[5].body, { provider: "google", origin: "https://app.example" });
});

test("settings skills and trace helpers use v2 routes while users remain explicit stubs", async () => {
  const calls = installFetchRecorder(() => ({ ok: true }));

  await fetchSkills();
  await fetchSkillContent("summarizer/needs encoding");
  await fetchTraceCredits();
  await authorizeTraceHold("trace/needs encoding");
  const users = await fetchUsers();
  const createResult = await createUser({ name: "Ada" });
  const updateResult = await updateUser("u1", { name: "Grace" });

  assert.deepEqual(
    calls.map((call) => [call.path, call.options?.method || "GET"]),
    [
      ["/api/webchat/v2/skills", "GET"],
      ["/api/webchat/v2/skills/summarizer%2Fneeds%20encoding", "GET"],
      ["/api/webchat/v2/traces/credit", "GET"],
      ["/api/webchat/v2/traces/holds/trace%2Fneeds%20encoding/authorize", "POST"],
    ]
  );
  assert.ok(calls.every((call) => call.options.headers.get("Authorization") === "Bearer settings-token"));
  assert.deepEqual(users, { users: [], todo: true });
  assert.equal(createResult.success, false);
  assert.equal(updateResult.success, false);
  assert.match(createResult.message, /v2 users endpoint/);
  assert.match(updateResult.message, /v2 users endpoint/);
});
