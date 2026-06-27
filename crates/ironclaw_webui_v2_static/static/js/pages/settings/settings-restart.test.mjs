import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

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
      if (typeof value === "string") scalars.push(value);
    }
  });
  return scalars;
}

function valuesAfterFragment(root, fragment) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function renderSettingsPage({ needsRestart = false } = {}) {
  const RestartBanner = "RestartBanner";
  const context = {
    AgentTab: "AgentTab",
    ChannelsTab: "ChannelsTab",
    InferenceTab: "InferenceTab",
    LanguageTab: "LanguageTab",
    Navigate: "Navigate",
    NetworkingTab: "NetworkingTab",
    RestartBanner,
    SettingsToolbar: "SettingsToolbar",
    SkillsTab: "SkillsTab",
    ToolsTab: "ToolsTab",
    TraceCommonsTab: "TraceCommonsTab",
    UsersTab: "UsersTab",
    globalThis: {},
    html,
    React: {
      useEffect: (fn) => fn(),
      useState: (value) => [value, () => {}],
    },
    useOutletContext: () => ({
      gatewayStatus: { state: "running" },
      gatewayStatusQuery: { isLoading: false },
      isAdmin: true,
    }),
    useParams: () => ({ tab: "agent" }),
    useSettings: () => ({
      settings: {},
      query: { data: { settings: {} }, isLoading: false },
      save: () => {},
      savedKeys: {},
      needsRestart,
      importSettings: () => {},
      isImporting: false,
      saveError: null,
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(sourceForTest("./settings-page.js", ["SettingsPage"]), context);
  return {
    RestartBanner,
    rendered: context.globalThis.__testExports.SettingsPage(),
  };
}

function evaluateRestartHook() {
  const stateWrites = [];
  const context = {
    globalThis: {},
    React: {
      useCallback: (fn) => fn,
      useState: (value) => [value, (next) => stateWrites.push(next)],
    },
    useT: () => (key) => `t:${key}`,
  };

  vm.runInNewContext(
    sourceForTest("./hooks/useGatewayRestart.js", ["useGatewayRestart"]),
    context,
  );
  return {
    restart: context.globalThis.__testExports.useGatewayRestart(),
    stateWrites,
  };
}

function renderRestartBanner({ visible = true, restartOverrides = {} } = {}) {
  const restartCalls = [];
  const restart = {
    restartEnabled: false,
    unavailableReason: "restart unavailable",
    isRestarting: false,
    progressLabel: "",
    error: null,
    message: null,
    confirmOpen: false,
    openConfirm: () => restartCalls.push("open"),
    closeConfirm: () => restartCalls.push("close"),
    confirmRestart: () => restartCalls.push("confirm"),
    ...restartOverrides,
  };
  const context = {
    Button: "Button",
    Icon: "Icon",
    Modal: "Modal",
    ModalBody: "ModalBody",
    ModalFooter: "ModalFooter",
    globalThis: {},
    html,
    useGatewayRestart: () => restart,
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./components/restart-banner.js", ["RestartBanner"]),
    context,
  );
  return {
    Button: context.Button,
    Modal: context.Modal,
    rendered: context.globalThis.__testExports.RestartBanner({
      visible,
      gatewayStatus: { state: "running" },
      gatewayStatusQuery: { isLoading: false },
    }),
    restartCalls,
  };
}

test("SettingsPage hides RestartBanner unless settings report a pending restart", () => {
  const hidden = renderSettingsPage({ needsRestart: false });
  const visible = renderSettingsPage({ needsRestart: true });

  assert.equal(findComponentNodes(hidden.rendered, hidden.RestartBanner).length, 0);
  assert.equal(findComponentNodes(visible.rendered, visible.RestartBanner).length, 1);
});

test("useGatewayRestart returns a disabled v2 no-op interface", () => {
  const { restart } = evaluateRestartHook();

  assert.equal(restart.restartEnabled, false);
  assert.equal(restart.unavailableReason, "t:settings.restartUnavailable");
  assert.equal(restart.isRestarting, false);
  assert.equal(restart.progressLabel, "");
  assert.equal(restart.error, null);
  assert.equal(restart.message, null);
  assert.equal(restart.confirmOpen, false);
});

test("useGatewayRestart confirmation callbacks only toggle local confirmation state", () => {
  const { restart, stateWrites } = evaluateRestartHook();

  restart.openConfirm();
  restart.closeConfirm();
  restart.confirmRestart();

  assert.deepEqual(stateWrites, [true, false, false]);
});

test("RestartBanner renders disabled unavailable action when restart is not wired", () => {
  const { rendered } = renderRestartBanner();

  assert.ok(collectScalars(rendered).includes("settings.restartRequired"));
  assert.ok(collectScalars(rendered).includes("restart unavailable"));
  assert.equal(valuesAfterFragment(rendered, "disabled=")[0], true);
  assert.equal(valuesAfterFragment(rendered, "title=")[0], "restart unavailable");
});

test("RestartBanner stays null when explicitly hidden", () => {
  assert.equal(renderRestartBanner({ visible: false }).rendered, null);
});
