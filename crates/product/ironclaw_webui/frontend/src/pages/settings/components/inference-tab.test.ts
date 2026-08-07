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

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function component(name) {
  return function TestComponent() {
    return name;
  };
}

function renderInferenceModule() {
  const context = {
    Badge: component("Badge"),
    Button: component("Button"),
    Card: component("Card"),
    ConfirmDialog: component("ConfirmDialog"),
    ProviderManagement: component("ProviderManagement"),
    SettingsGroup: component("SettingsGroup"),
    SettingsSearchEmpty: component("SettingsSearchEmpty"),
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => [initial, () => {}],
    },
    html,
    INFERENCE_FIELDS,
    filterSettingsSections,
    matchesSearch,
    useLlmProviders: () => ({
      activeProviderId: "openai",
      selectedModel: "gpt-4.1",
      providers: [{ id: "openai", default_model: "gpt-4.1" }],
      hasActiveProvider: true,
      isResetting: false,
      resetToDefaults: async () => {},
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
});

test("Inference tab resets model settings only after shared-dialog confirmation", () => {
  const { context, exports } = renderInferenceModule();
  const rendered = exports.InferenceTab({
    settings: {},
    gatewayStatus: null,
    onSave: () => {},
    savedKeys: {},
    isLoading: false,
    searchQuery: "",
  });

  const buttonScalars = findComponentNodes(rendered, context.Button)
    .flatMap((node) => node.values)
    .filter((value) => typeof value === "string");
  assert.ok(buttonScalars.includes("llm.resetToDefaults"));

  const [dialog] = findComponentNodes(rendered, context.ConfirmDialog).map((node) =>
    componentProps(node, context.ConfirmDialog)
  );
  assert.equal(dialog.open, false);
  assert.equal(dialog.title, "llm.confirmResetToDefaults");
  assert.equal(dialog.confirmLabel, "llm.resetToDefaults");
  assert.equal(typeof dialog.onConfirm, "function");
});
