import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { groupProvidersByStatus } from "../lib/llm-providers.js";

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

function createReactStateStub(state) {
  return {
    useCallback: (fn) => fn,
    useEffect: (fn) => fn(),
    useState: (initial) => {
      if (!Object.hasOwn(state, "expanded")) {
        state.expanded = typeof initial === "function" ? initial() : initial;
      }
      return [
        state.expanded,
        (next) => {
          state.expanded = typeof next === "function" ? next(state.expanded) : next;
        },
      ];
    },
  };
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
  const nodes = [];
  visit(root, (node) => {
    if (Array.isArray(node.values) && node.values.includes(component)) nodes.push(node);
  });
  return nodes;
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

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function valueAfter(rendered, fragment) {
  const index = rendered.strings.findIndex((part) => part.includes(fragment));
  assert.notEqual(index, -1, `expected template fragment ${fragment}`);
  return rendered.values[index];
}

function valuesAfter(rendered, fragment) {
  return rendered.strings.reduce((values, part, index) => {
    if (part.includes(fragment)) values.push(rendered.values[index]);
    return values;
  }, []);
}

function renderProviderCard(context, props) {
  return context.globalThis.__testExports.ProviderCard({
    activeProviderId: "nearai",
    selectedModel: "active-model",
    builtinOverrides: {},
    isBusy: false,
    onUse: () => {},
    onConfigure: () => {},
    onDelete: () => {},
    ...props,
  });
}

function createProviderCardContext({ state = {} } = {}) {
  const context = {
    Badge: "Badge",
    Button: "Button",
    Card: "Card",
    Icon: "Icon",
    React: createReactStateStub(state),
    adapterLabel: (adapter) => adapter,
    globalThis: {},
    html,
    isProviderConfigured: (provider) => provider.configured !== false,
    providerDisplayModel: (provider) => provider.default_model || "model",
    providerEffectiveBaseUrl: (provider) => provider.base_url || "https://example.com/v1",
    providerMissingReason: (provider) => provider.missing || "api_key",
    useT: () => (key) => key,
  };
  vm.runInNewContext(
    sourceForTest("./provider-card.js", ["ProviderCard"]),
    context
  );
  return { context, state };
}

test("ProviderManagement groups filtered providers through the render caller", () => {
  const ProviderCard = "ProviderCard";
  const providers = [
    {
      id: "nearai",
      name: "NEAR AI",
      builtin: true,
      adapter: "nearai",
      api_key_required: true,
      base_url_required: false,
      has_api_key: true,
    },
    {
      id: "openai",
      name: "OpenAI",
      builtin: true,
      adapter: "open_ai_completions",
      api_key_required: true,
      base_url_required: false,
      has_api_key: true,
    },
    {
      id: "anthropic",
      name: "Anthropic",
      builtin: true,
      adapter: "anthropic",
      api_key_required: true,
      base_url_required: false,
      has_api_key: false,
    },
  ];
  const context = {
    Button: "Button",
    Card: "Card",
    Icon: "Icon",
    ProviderCard,
    ProviderDialog: "ProviderDialog",
    SettingsSearchEmpty: "SettingsSearchEmpty",
    globalThis: {},
    groupProvidersByStatus,
    html,
    useProviderManagementActions: () => ({
      allProviderIds: providers.map((provider) => provider.id),
      closeDialog: () => {},
      dialogProvider: null,
      filteredProviders: providers,
      handleDelete: () => {},
      handleSave: () => {},
      handleUse: () => {},
      isDialogOpen: false,
      message: null,
      openDialog: () => {},
      providerState: {
        activeProviderId: "nearai",
        builtinOverrides: {},
        error: null,
        isBusy: false,
        isLoading: false,
        selectedModel: "llama",
      },
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./provider-management.js", ["ProviderManagement"]),
    context
  );
  const rendered = context.globalThis.__testExports.ProviderManagement({
    settings: {},
    gatewayStatus: {},
    searchQuery: "",
  });

  const labels = collectScalars(rendered).filter((value) =>
    ["llm.groupActive", "llm.groupReady", "llm.groupSetup"].includes(value)
  );
  assert.deepEqual(labels, ["llm.groupActive", "llm.groupReady", "llm.groupSetup"]);

  const cardProps = findComponentNodes(rendered, ProviderCard).map((node) =>
    componentProps(node, ProviderCard)
  );
  assert.deepEqual(
    cardProps.map((props) => props.provider.id),
    ["nearai", "openai", "anthropic"]
  );
  assert.deepEqual(
    cardProps.map((props) => props.activeProviderId),
    ["nearai", "nearai", "nearai"]
  );
});

test("ProviderManagement hides empty buckets after search filtering", () => {
  const ProviderCard = "ProviderCard";
  const providers = [
    {
      id: "openai",
      name: "OpenAI",
      builtin: true,
      adapter: "open_ai_completions",
      api_key_required: true,
      base_url_required: false,
      has_api_key: true,
    },
  ];
  const context = {
    Button: "Button",
    Card: "Card",
    Icon: "Icon",
    ProviderCard,
    ProviderDialog: "ProviderDialog",
    SettingsSearchEmpty: "SettingsSearchEmpty",
    globalThis: {},
    groupProvidersByStatus,
    html,
    useProviderManagementActions: () => ({
      allProviderIds: providers.map((provider) => provider.id),
      closeDialog: () => {},
      dialogProvider: null,
      filteredProviders: providers,
      handleDelete: () => {},
      handleSave: () => {},
      handleUse: () => {},
      isDialogOpen: false,
      message: null,
      openDialog: () => {},
      providerState: {
        activeProviderId: "nearai",
        builtinOverrides: {},
        error: null,
        isBusy: false,
        isLoading: false,
        selectedModel: "llama",
      },
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./provider-management.js", ["ProviderManagement"]),
    context
  );
  const rendered = context.globalThis.__testExports.ProviderManagement({
    settings: {},
    gatewayStatus: {},
    searchQuery: "open",
  });

  const labels = collectScalars(rendered).filter((value) =>
    ["llm.groupActive", "llm.groupReady", "llm.groupSetup"].includes(value)
  );
  assert.deepEqual(labels, ["llm.groupReady"]);
  const cardProps = findComponentNodes(rendered, ProviderCard).map((node) =>
    componentProps(node, ProviderCard)
  );
  assert.deepEqual(
    cardProps.map((props) => props.provider.id),
    ["openai"]
  );
});

test("ProviderCard disclosure responds to row, keyboard, and chevron controls", () => {
  const { context, state } = createProviderCardContext();
  let rendered = renderProviderCard(context, {
    provider: {
      id: "openai",
      name: "OpenAI",
      builtin: true,
      adapter: "open_ai_completions",
      configured: true,
      default_model: "gpt",
    },
  });
  assert.equal(valueAfter(rendered, "aria-expanded="), false);

  valueAfter(rendered, "onClick=")();
  assert.equal(state.expanded, true);

  rendered = renderProviderCard(context, {
    provider: {
      id: "openai",
      name: "OpenAI",
      builtin: true,
      adapter: "open_ai_completions",
      configured: true,
      default_model: "gpt",
    },
  });
  assert.equal(valueAfter(rendered, "aria-expanded="), true);

  let prevented = 0;
  valueAfter(rendered, "onKeyDown=")({
    key: "Enter",
    preventDefault: () => {
      prevented += 1;
    },
  });
  assert.equal(prevented, 1);
  assert.equal(state.expanded, false);

  rendered = renderProviderCard(context, {
    provider: {
      id: "openai",
      name: "OpenAI",
      builtin: true,
      adapter: "open_ai_completions",
      configured: true,
      default_model: "gpt",
    },
  });
  const chevronClick = valuesAfter(rendered, "onClick=")[2];
  chevronClick();
  assert.equal(state.expanded, true);
});

test("ProviderCard actions keep existing callbacks without toggling disclosure", () => {
  const calls = [];
  const { context, state } = createProviderCardContext();
  let rendered = renderProviderCard(context, {
    onUse: (provider) => calls.push(["use", provider.id]),
    provider: {
      id: "openai",
      name: "OpenAI",
      builtin: true,
      adapter: "open_ai_completions",
      configured: true,
      default_model: "gpt",
    },
  });
  const actionBarrier = valuesAfter(rendered, "onClick=")[1];
  let stopped = 0;
  actionBarrier({
    stopPropagation: () => {
      stopped += 1;
    },
  });
  assert.equal(stopped, 1);

  const useButton = findComponentNodes(rendered, "Button")[0];
  componentProps(useButton, "Button").onClick();
  assert.deepEqual(calls, [["use", "openai"]]);
  assert.equal(state.expanded, false);

  rendered = renderProviderCard(context, {
    onConfigure: (provider) => calls.push(["configure", provider.id]),
    provider: {
      id: "anthropic",
      name: "Anthropic",
      builtin: true,
      adapter: "anthropic",
      configured: false,
      default_model: "claude",
      missing: "api_key",
    },
  });
  const configureButton = findComponentNodes(rendered, "Button")[0];
  componentProps(configureButton, "Button").onClick();
  assert.deepEqual(calls.at(-1), ["configure", "anthropic"]);
  assert.equal(state.expanded, false);

  state.expanded = true;
  rendered = renderProviderCard(context, {
    onConfigure: (provider) => calls.push(["edit", provider.id]),
    onDelete: (provider) => calls.push(["delete", provider.id]),
    provider: {
      id: "local",
      name: "Local",
      builtin: false,
      adapter: "ollama",
      configured: true,
      default_model: "llama",
    },
  });
  const buttonNodes = findComponentNodes(rendered, "Button");
  const deleteButton = buttonNodes.find((node) => collectScalars(node).includes("common.delete"));
  assert.ok(deleteButton, "expected delete button for expanded custom provider");
  componentProps(deleteButton, "Button").onClick();
  assert.deepEqual(calls.at(-1), ["delete", "local"]);
  assert.equal(state.expanded, true);
});
