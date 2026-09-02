import assert from "node:assert/strict";
import { test } from "vitest";

import { INFERENCE_FIELDS } from "../lib/settings-schema";
import { filterSettingsSections, matchesSearch } from "../lib/settings-search";
import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function findComponentNodes(root, component) {
  const found = [];
  visit(root, (node) => {
    if (Array.isArray(node.values) && node.values.includes(component)) {
      found.push(node);
    }
  });
  return found;
}

function component(name) {
  return function TestComponent() {
    return name;
  };
}

function renderInferenceModule() {
  const context = {
    Badge: component("Badge"),
    Card: component("Card"),
    Skeleton: component("Skeleton"),
    ProviderManagement: component("ProviderManagement"),
    ModelSelectionPolicyEditor: component("ModelSelectionPolicyEditor"),
    ModelCapabilityBadges: component("ModelCapabilityBadges"),
    UserModelPreferenceSelector: component("UserModelPreferenceSelector"),
    SettingsGroup: component("SettingsGroup"),
    SettingsSearchEmpty: component("SettingsSearchEmpty"),
    html,
    INFERENCE_FIELDS,
    filterSettingsSections,
    matchesSearch,
    modelEntryFor: (entries, model) => entries.find((entry) => entry.id === model),
    normalizeModelCatalog: (catalog) => ({
      models: catalog.models || [],
      modelEntries: catalog.model_entries || [],
    }),
    useLlmProviders: () => ({
      activeProviderId: "openai",
      selectedModel: "gpt-4.1",
      providers: [{ id: "openai", default_model: "gpt-4.1" }],
      userModelPolicy: {
        provider_id: "openai",
        allowed_models: ["gpt-4.1"],
        model_entries: [
          {
            id: "gpt-4.1",
            input_modalities: ["text", "image"],
            output_modalities: ["text"],
          },
        ],
      },
      hasActiveProvider: true,
    }),
    useT: () => (key) => key,
  };

  const exports = runVmModuleForTest(
    "./inference-tab.tsx",
    ["InferenceTab"],
    context,
    import.meta.url
  );
  return { context, exports };
}

test("Inference tab omits unsupported operator-config fields", () => {
  const { context, exports } = renderInferenceModule();
  const rendered = exports.InferenceTab({
    isAdmin: true,
    settings: {},
    gatewayStatus: null,
    onSave: () => {},
    savedKeys: {},
    isLoading: false,
    searchQuery: "",
  });

  assert.equal(
    findComponentNodes(rendered, context.SettingsGroup).length,
    0,
    "unsupported settings like temperature must not render editable controls"
  );
  assert.equal(
    findComponentNodes(rendered, context.ProviderManagement).length,
    1,
    "LLM provider management should remain visible"
  );
  assert.equal(
    findComponentNodes(rendered, context.ModelSelectionPolicyEditor).length,
    1,
    "admins need a UI to enable the tenant model allowlist"
  );
  assert.equal(
    findComponentNodes(rendered, context.ModelCapabilityBadges).length,
    1,
    "active model summary should show persisted capabilities"
  );
});

test("Inference tab gives non-admin users model preference without provider controls", () => {
  const { context, exports } = renderInferenceModule();
  const rendered = exports.InferenceTab({
    isAdmin: false,
    settings: {},
    gatewayStatus: null,
    onSave: () => {},
    savedKeys: {},
    isLoading: false,
    searchQuery: "",
  });

  assert.equal(
    findComponentNodes(rendered, context.UserModelPreferenceSelector).length,
    1,
    "ordinary users should receive the caller-scoped model selector"
  );
  assert.equal(
    findComponentNodes(rendered, context.ProviderManagement).length,
    0,
    "ordinary users must not receive operator provider controls"
  );
  assert.equal(
    findComponentNodes(rendered, context.ModelSelectionPolicyEditor).length,
    0,
    "ordinary users must not receive tenant policy controls"
  );
});
