import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { groupProvidersByStatus } from "../lib/llm-providers.js";

const PROVIDER_GROUP_LABELS = [
  "llm.groupActive",
  "llm.groupReady",
  "llm.groupSetup",
];

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

function collectTemplateText(root) {
  const text = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings)) return;
    text.push(...node.strings);
  });
  return text.join("");
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

function deepValuesAfter(root, fragment) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function builtinProvider(id, overrides = {}) {
  return {
    id,
    name: id,
    builtin: true,
    adapter: "open_ai_completions",
    api_key_required: true,
    base_url_required: false,
    has_api_key: true,
    default_model: "model",
    ...overrides,
  };
}

function customProvider(id, overrides = {}) {
  return {
    id,
    name: id,
    builtin: false,
    adapter: "ollama",
    configured: true,
    default_model: "llama",
    ...overrides,
  };
}

function useProviderManagementActionsStub({ providers, activeProviderId }) {
  return () => ({
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
      activeProviderId,
      builtinOverrides: {},
      error: null,
      isBusy: false,
      isLoading: false,
      selectedModel: "llama",
    },
  });
}

function renderProviderManagement({ providers, activeProviderId = "nearai", searchQuery = "" }) {
  const ProviderCard = "ProviderCard";
  const context = {
    Button: "Button",
    Card: "Card",
    Icon: "Icon",
    ProviderCard,
    ProviderDialog: "ProviderDialog",
    ProviderLoginStatus: "ProviderLoginStatus",
    SettingsSearchEmpty: "SettingsSearchEmpty",
    globalThis: {},
    groupProvidersByStatus,
    html,
    useProviderManagementActions: useProviderManagementActionsStub({
      providers,
      activeProviderId,
    }),
    useProviderLogin: () => ({
      codexBusy: false,
      nearaiBusy: false,
      startCodex: () => {},
      startNearai: () => {},
      startNearaiWallet: () => {},
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
    searchQuery,
  });
  const cardProps = findComponentNodes(rendered, ProviderCard).map((node) =>
    componentProps(node, ProviderCard)
  );
  const loginStatusNode = findComponentNodes(rendered, "ProviderLoginStatus")[0];
  const loginStatusProps = loginStatusNode
    ? componentProps(loginStatusNode, "ProviderLoginStatus")
    : null;
  return { rendered, cardProps, loginStatusProps };
}

function renderProviderLoginStatus({ nearaiSsoAvailable, hostname }) {
  const context = {
    globalThis: {},
    html,
    useT: () => (key) => key,
  };
  if (hostname !== undefined) {
    context.window = { location: { hostname } };
  }
  vm.runInNewContext(
    sourceForTest("./provider-login-status.js", ["ProviderLoginStatus"]),
    context
  );
  return context.globalThis.__testExports.ProviderLoginStatus({
    login: {
      nearaiBusy: false,
      nearaiError: "",
      codexBusy: false,
      codexError: "",
      codexCode: null,
    },
    nearaiSsoAvailable,
  });
}

function groupLabels(rendered) {
  return collectScalars(rendered).filter((value) => PROVIDER_GROUP_LABELS.includes(value));
}

function depsChanged(previous, next) {
  if (!previous || !next || previous.length !== next.length) return true;
  return next.some((value, index) => value !== previous[index]);
}

function createReactStateStub(state) {
  return {
    useCallback: (fn) => fn,
    useEffect: (fn, deps) => {
      if (depsChanged(state.effectDeps, deps)) {
        state.effectDeps = deps ? Array.from(deps) : deps;
        fn();
      }
    },
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

function createReactMenuStateStub(state) {
  return {
    useEffect: (fn, deps) => {
      if (depsChanged(state.effectDeps, deps)) {
        state.effectDeps = deps ? Array.from(deps) : deps;
        fn();
      }
    },
    useRef: () => ({ current: null }),
    useState: (initial) => {
      if (!Object.hasOwn(state, "open")) {
        state.open = typeof initial === "function" ? initial() : initial;
      }
      return [
        state.open,
        (next) => {
          state.open = typeof next === "function" ? next(state.open) : next;
        },
      ];
    },
  };
}

function createProviderCardHarness() {
  const state = {};
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
    providerAcceptsApiKey: (provider) => provider.accepts_api_key !== false,
    providerDisplayModel: (provider) => provider.default_model || "model",
    providerEffectiveBaseUrl: (provider) => provider.base_url || "https://example.com/v1",
    providerMissingReason: (provider) => provider.missing || "api_key",
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./provider-card.js", ["ProviderCard"]),
    context
  );

  return {
    state,
    render: (props) =>
      context.globalThis.__testExports.ProviderCard({
        activeProviderId: "nearai",
        selectedModel: "active-model",
        builtinOverrides: {},
        isBusy: false,
        onUse: () => {},
        onConfigure: () => {},
        onDelete: () => {},
        onNearaiLogin: () => {},
        onNearaiWallet: () => {},
        onCodexLogin: () => {},
        loginBusy: false,
        ...props,
      }),
  };
}

function createNearAiSetupMenuHarness() {
  const state = {};
  const calls = [];
  const context = {
    Button: "Button",
    Icon: "Icon",
    React: createReactMenuStateStub(state),
    document: {
      addEventListener: (type, handler) => {
        state.listeners ??= {};
        state.listeners[type] = handler;
      },
      removeEventListener: (type, handler) => {
        if (state.listeners?.[type] === handler) delete state.listeners[type];
      },
    },
    globalThis: {},
    html,
  };

  vm.runInNewContext(
    sourceForTest("../../onboarding/onboarding-page.js", ["NearAiSetupMenu"]),
    context
  );

  return {
    calls,
    state,
    render: (props = {}) =>
      context.globalThis.__testExports.NearAiSetupMenu({
        provider: builtinProvider("nearai", { adapter: "nearai" }),
        isBusy: false,
        login: {
          nearaiBusy: false,
          startNearai: (provider) => calls.push(["sso", provider]),
          startNearaiWallet: () => calls.push(["wallet"]),
        },
        t: (key) => key,
        onSetUp: (provider) => calls.push(["configure", provider.id]),
        ...props,
      }),
  };
}

function firstButtonProps(rendered) {
  return componentProps(findComponentNodes(rendered, "Button")[0], "Button");
}

test("ProviderManagement groups filtered providers through the render caller", () => {
  const { rendered, cardProps } = renderProviderManagement({
    providers: [
      builtinProvider("nearai", { adapter: "nearai" }),
      builtinProvider("openai"),
      builtinProvider("anthropic", {
        adapter: "anthropic",
        has_api_key: false,
      }),
    ],
  });

  assert.deepEqual(groupLabels(rendered), PROVIDER_GROUP_LABELS);
  assert.deepEqual(deepValuesAfter(rendered, "data-provider-status="), [
    "active",
    "ready",
    "setup",
  ]);
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
  const { rendered, cardProps } = renderProviderManagement({
    providers: [builtinProvider("openai")],
    searchQuery: "open",
  });

  assert.deepEqual(groupLabels(rendered), ["llm.groupReady"]);
  assert.deepEqual(
    cardProps.map((props) => props.provider.id),
    ["openai"]
  );
});

test("ProviderCard disclosure responds to row, keyboard, and chevron controls", () => {
  const harness = createProviderCardHarness();
  const renderOpenAi = () =>
    harness.render({
      provider: builtinProvider("openai", { default_model: "gpt" }),
    });

  let rendered = renderOpenAi();
  assert.equal(valueAfter(rendered, "aria-expanded="), "false");

  valueAfter(rendered, "onClick=")();
  assert.equal(harness.state.expanded, true);

  rendered = renderOpenAi();
  assert.equal(valueAfter(rendered, "aria-expanded="), "true");

  valueAfter(rendered, "onClick=")();
  assert.equal(harness.state.expanded, false);

  rendered = renderOpenAi();
  valuesAfter(rendered, "onClick=")[1]();
  assert.equal(harness.state.expanded, true);
});

test("ProviderCard syncs disclosure state when active provider changes", () => {
  const harness = createProviderCardHarness();
  const provider = builtinProvider("openai", { default_model: "gpt" });

  let rendered = harness.render({ provider, activeProviderId: "nearai" });
  assert.equal(valueAfter(rendered, "aria-expanded="), "false");

  rendered = harness.render({ provider, activeProviderId: "openai" });
  rendered = harness.render({ provider, activeProviderId: "openai" });
  assert.equal(valueAfter(rendered, "aria-expanded="), "true");
  assert.equal(harness.state.expanded, true);

  rendered = harness.render({ provider, activeProviderId: "nearai" });
  rendered = harness.render({ provider, activeProviderId: "nearai" });
  assert.equal(valueAfter(rendered, "aria-expanded="), "false");
  assert.equal(harness.state.expanded, false);
});

test("ProviderCard actions keep existing callbacks without toggling disclosure", () => {
  const calls = [];
  const harness = createProviderCardHarness();

  let rendered = harness.render({
    onUse: (provider) => calls.push(["use", provider.id]),
    provider: builtinProvider("openai", { default_model: "gpt" }),
  });

  firstButtonProps(rendered).onClick();
  assert.deepEqual(calls, [["use", "openai"]]);
  assert.equal(harness.state.expanded, false);

  rendered = harness.render({
    onConfigure: (provider) => calls.push(["configure", provider.id]),
    provider: builtinProvider("anthropic", {
      adapter: "anthropic",
      configured: false,
      default_model: "claude",
      missing: "api_key",
    }),
  });
  firstButtonProps(rendered).onClick();
  assert.deepEqual(calls.at(-1), ["configure", "anthropic"]);
  assert.equal(harness.state.expanded, false);

  harness.state.expanded = true;
  rendered = harness.render({
    onDelete: (provider) => calls.push(["delete", provider.id]),
    provider: customProvider("local"),
  });
  const deleteButton = findComponentNodes(rendered, "Button").find((node) =>
    collectScalars(node).includes("common.delete")
  );
  assert.ok(deleteButton, "expected delete button for expanded custom provider");
  componentProps(deleteButton, "Button").onClick();
  assert.deepEqual(calls.at(-1), ["delete", "local"]);
  assert.equal(harness.state.expanded, true);
});

test("ProviderCard renders login actions instead of generic use for login providers", () => {
  const calls = [];
  const harness = createProviderCardHarness();

  let rendered = harness.render({
    activeProviderId: "openai",
    onConfigure: (provider) => calls.push(["configure", provider.id]),
    provider: builtinProvider("nearai", { adapter: "nearai", has_api_key: false }),
  });
  let labels = collectScalars(rendered);
  let templateText = collectTemplateText(rendered);
  assert.ok(labels.includes("onboarding.nearWallet"));
  assert.ok(labels.includes("llm.addApiKey"));
  assert.ok(templateText.includes("GitHub"));
  assert.ok(templateText.includes("Google"));
  assert.ok(!labels.includes("llm.use"));
  const addKeyButton = findComponentNodes(rendered, "Button").find((node) => {
    const scalars = collectScalars(node);
    return scalars.includes("llm.addApiKey") && !scalars.includes("onboarding.nearWallet");
  });
  assert.ok(addKeyButton, "expected NEAR API key action");
  componentProps(addKeyButton, "Button").onClick();
  assert.deepEqual(calls, [["configure", "nearai"]]);

  rendered = harness.render({
    activeProviderId: "openai",
    provider: builtinProvider("openai_codex"),
  });
  labels = collectScalars(rendered);
  templateText = collectTemplateText(rendered);
  assert.ok(labels.includes("onboarding.codexSignIn"));
  assert.ok(!labels.includes("llm.use"));
});

test("ProviderCard renders generic use action for NEAR when an API key is configured", () => {
  const calls = [];
  const harness = createProviderCardHarness();
  harness.state.expanded = true;

  const rendered = harness.render({
    activeProviderId: "openai",
    onUse: (provider) => calls.push(["use", provider.id]),
    provider: builtinProvider("nearai", {
      adapter: "nearai",
      has_api_key: true,
    }),
  });
  const labels = collectScalars(rendered);
  const templateText = collectTemplateText(rendered);

  assert.ok(labels.includes("llm.use"));
  assert.ok(labels.includes("llm.configure"));
  assert.ok(!labels.includes("llm.addApiKey"));
  assert.ok(!labels.includes("onboarding.nearWallet"));
  assert.ok(!templateText.includes("GitHub"));

  firstButtonProps(rendered).onClick();
  assert.deepEqual(calls, [["use", "nearai"]]);
});

test("NearAiSetupMenu keeps NEAR onboarding SSO choices behind setup dropdown", () => {
  const harness = createNearAiSetupMenuHarness();

  let rendered = harness.render();
  assert.equal(valueAfter(rendered, "aria-expanded="), "false");
  assert.equal(firstButtonProps(rendered).disabled, false);
  let labels = collectScalars(rendered);
  assert.ok(labels.includes("onboarding.setUp"));
  assert.ok(!labels.includes("llm.addApiKey"));
  assert.ok(!labels.includes("onboarding.nearWallet"));
  assert.ok(!labels.includes("GitHub"));

  firstButtonProps(rendered).onClick();
  assert.equal(harness.state.open, true);

  rendered = harness.render();
  assert.equal(valueAfter(rendered, "aria-expanded="), "true");
  assert.equal(typeof harness.state.listeners.keydown, "function");
  labels = collectScalars(rendered);
  assert.ok(labels.includes("llm.addApiKey"));
  assert.ok(labels.includes("onboarding.nearWallet"));
  assert.ok(labels.includes("GitHub"));
  assert.ok(labels.includes("Google"));

  deepValuesAfter(rendered, "onClick=")[1]();
  assert.deepEqual(harness.calls, [["configure", "nearai"]]);
  assert.equal(harness.state.open, false);

  firstButtonProps(harness.render()).onClick();
  rendered = harness.render();
  deepValuesAfter(rendered, "onClick=")[3]();
  assert.deepEqual(harness.calls.at(-1), ["sso", "github"]);
});

test("NearAiSetupMenu disables setup trigger while setup or login is busy", () => {
  const harness = createNearAiSetupMenuHarness();

  assert.equal(firstButtonProps(harness.render({ isBusy: true })).disabled, true);
  assert.equal(
    firstButtonProps(
      harness.render({
        login: {
          nearaiBusy: true,
          startNearai: () => {},
          startNearaiWallet: () => {},
        },
      })
    ).disabled,
    true
  );
});

test("NearAiSetupMenu closes the setup dropdown on Escape", () => {
  const harness = createNearAiSetupMenuHarness();

  firstButtonProps(harness.render()).onClick();
  harness.render();

  harness.state.listeners.keydown({ key: "Enter" });
  assert.equal(harness.state.open, true);

  harness.state.listeners.keydown({ key: "Escape" });
  assert.equal(harness.state.open, false);
});

test("ProviderLoginStatus shows the localhost SSO notice only on a local origin with NEAR AI SSO available", () => {
  const localWithSso = renderProviderLoginStatus({
    nearaiSsoAvailable: true,
    hostname: "localhost",
  });
  assert.ok(
    collectScalars(localWithSso).includes("onboarding.nearaiLocalSso"),
    "expected notice on localhost when NEAR AI SSO is available"
  );

  const loopbackWithSso = renderProviderLoginStatus({
    nearaiSsoAvailable: true,
    hostname: "127.0.0.1",
  });
  assert.ok(
    collectScalars(loopbackWithSso).includes("onboarding.nearaiLocalSso"),
    "expected notice on 127.0.0.1 too"
  );

  const localWithoutSso = renderProviderLoginStatus({
    nearaiSsoAvailable: false,
    hostname: "localhost",
  });
  assert.ok(
    !collectScalars(localWithoutSso).includes("onboarding.nearaiLocalSso"),
    "no notice when NEAR AI SSO is not on screen"
  );

  const publicWithSso = renderProviderLoginStatus({
    nearaiSsoAvailable: true,
    hostname: "app.example.com",
  });
  assert.ok(
    !collectScalars(publicWithSso).includes("onboarding.nearaiLocalSso"),
    "no notice on a public origin"
  );
});

test("ProviderManagement enables the localhost SSO notice only when NEAR AI SSO is actionable", () => {
  const notActive = renderProviderManagement({
    providers: [builtinProvider("nearai", { adapter: "nearai" }), builtinProvider("openai")],
    activeProviderId: "openai",
  });
  assert.equal(notActive.loginStatusProps.nearaiSsoAvailable, true);

  const nearaiActive = renderProviderManagement({
    providers: [builtinProvider("nearai", { adapter: "nearai" }), builtinProvider("openai")],
    activeProviderId: "nearai",
  });
  assert.equal(nearaiActive.loginStatusProps.nearaiSsoAvailable, false);

  const noNearai = renderProviderManagement({
    providers: [builtinProvider("openai")],
    activeProviderId: "openai",
  });
  assert.equal(noNearai.loginStatusProps.nearaiSsoAvailable, false);
});
