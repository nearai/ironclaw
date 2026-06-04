import assert from "node:assert/strict";
import test from "node:test";

import {
  API_KEY_UNCHANGED,
  groupProvidersByStatus,
  providerStatus,
} from "./llm-providers.js";

// Helpers — minimal provider shapes that satisfy the configured/missing
// predicates without coupling the test to the full provider schema.
const builtinReady = (id, opts = {}) => ({
  id,
  name: id,
  builtin: true,
  adapter: "open_ai_completions",
  api_key_required: true,
  base_url_required: false,
  has_api_key: true,
  default_model: "model-x",
  ...opts,
});

const builtinNeedsKey = (id) => ({
  id,
  name: id,
  builtin: true,
  adapter: "anthropic",
  api_key_required: true,
  base_url_required: false,
  has_api_key: false,
  default_model: "claude",
});

const customReady = (id) => ({
  id,
  name: id,
  builtin: false,
  adapter: "open_ai_completions",
  base_url: "https://example.com/v1",
  api_key: API_KEY_UNCHANGED,
  default_model: "m",
});

test("providerStatus returns 'active' for the active id regardless of configuration", () => {
  const provider = builtinNeedsKey("anthropic");
  assert.equal(providerStatus(provider, {}, "anthropic"), "active");
});

test("providerStatus returns 'ready' for configured non-active providers", () => {
  assert.equal(providerStatus(builtinReady("nearai"), {}, "other"), "ready");
  assert.equal(providerStatus(customReady("vllm"), {}, "other"), "ready");
});

test("providerStatus returns 'setup' when an API key is missing", () => {
  assert.equal(providerStatus(builtinNeedsKey("anthropic"), {}, "nearai"), "setup");
});

test("groupProvidersByStatus buckets providers into active/ready/setup", () => {
  const providers = [
    builtinReady("nearai"),
    builtinNeedsKey("anthropic"),
    builtinReady("bedrock"),
    builtinNeedsKey("cerebras"),
    customReady("vllm"),
  ];

  const groups = groupProvidersByStatus(providers, {}, "nearai");

  assert.deepEqual(
    groups.active.map((p) => p.id),
    ["nearai"]
  );
  assert.deepEqual(
    groups.ready.map((p) => p.id),
    ["bedrock", "vllm"]
  );
  assert.deepEqual(
    groups.setup.map((p) => p.id),
    ["anthropic", "cerebras"]
  );
});

test("groupProvidersByStatus preserves upstream order inside each bucket", () => {
  const providers = [
    builtinReady("z-provider"),
    builtinReady("a-provider"),
    builtinNeedsKey("y-needs-key"),
    builtinNeedsKey("b-needs-key"),
  ];

  const groups = groupProvidersByStatus(providers, {}, "none");

  assert.deepEqual(
    groups.ready.map((p) => p.id),
    ["z-provider", "a-provider"],
    "ready bucket preserves input ordering (no implicit sort)"
  );
  assert.deepEqual(
    groups.setup.map((p) => p.id),
    ["y-needs-key", "b-needs-key"],
    "setup bucket preserves input ordering"
  );
});

test("groupProvidersByStatus honours builtin overrides when classifying", () => {
  // Provider declares it needs an API key but the override supplies a stored
  // sentinel value — should be considered ready, not in setup.
  const provider = builtinNeedsKey("anthropic");
  const overrides = { anthropic: { api_key: API_KEY_UNCHANGED } };

  const groups = groupProvidersByStatus([provider], overrides, "nearai");

  assert.deepEqual(groups.ready.map((p) => p.id), ["anthropic"]);
  assert.deepEqual(groups.setup.map((p) => p.id), []);
});

test("groupProvidersByStatus returns empty arrays for missing buckets, not undefined", () => {
  const groups = groupProvidersByStatus([], {}, null);
  assert.deepEqual(groups, { active: [], ready: [], setup: [] });
});
