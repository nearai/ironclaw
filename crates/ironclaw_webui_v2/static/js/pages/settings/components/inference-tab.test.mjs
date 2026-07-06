import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { INFERENCE_FIELDS } from "../lib/settings-schema.js";
import { filterSettingsSections, matchesSearch } from "../lib/settings-search.js";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

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
    ProviderManagement: component("ProviderManagement"),
    SettingsGroup: component("SettingsGroup"),
    SettingsSearchEmpty: component("SettingsSearchEmpty"),
    globalThis: {},
    html,
    INFERENCE_FIELDS,
    filterSettingsSections,
    matchesSearch,
    useLlmProviders: () => ({
      activeProviderId: "openai",
      selectedModel: "gpt-4.1",
      providers: [{ id: "openai", default_model: "gpt-4.1" }],
      hasActiveProvider: true,
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(sourceForTest("./inference-tab.js", ["InferenceTab"]), context);
  return { context, exports: context.globalThis.__testExports };
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
