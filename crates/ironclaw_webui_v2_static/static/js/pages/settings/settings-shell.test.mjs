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

const SETTINGS_TABS = [
  { id: "inference", labelKey: "settings.inference", icon: "spark" },
  { id: "agent", labelKey: "settings.agent", icon: "bolt" },
  { id: "channels", labelKey: "settings.channels", icon: "send" },
  { id: "networking", labelKey: "settings.networking", icon: "pulse" },
  { id: "tools", labelKey: "settings.tools", icon: "tool" },
  { id: "skills", labelKey: "settings.skills", icon: "file" },
  { id: "traces", labelKey: "settings.traceCommons", icon: "layers" },
  { id: "users", labelKey: "settings.users", icon: "lock" },
  { id: "language", labelKey: "settings.language", icon: "globe" },
];

function renderSettingsPage({ requestedTab, isAdmin = false } = {}) {
  const Navigate = "Navigate";
  const components = {
    AgentTab: "AgentTab",
    ChannelsTab: "ChannelsTab",
    InferenceTab: "InferenceTab",
    LanguageTab: "LanguageTab",
    NetworkingTab: "NetworkingTab",
    RestartBanner: "RestartBanner",
    SkillsTab: "SkillsTab",
    ToolsTab: "ToolsTab",
    TraceCommonsTab: "TraceCommonsTab",
    UsersTab: "UsersTab",
  };
  const stateSetters = [];
  const context = {
    ...components,
    Navigate,
    globalThis: {},
    html,
    React: {
      useEffect: (fn) => fn(),
      useState: (value) => {
        const setter = (next) => stateSetters.push(next);
        return [value, setter];
      },
    },
    useOutletContext: () => ({
      gatewayStatus: { state: "running" },
      gatewayStatusQuery: { isLoading: false },
      isAdmin,
    }),
    useParams: () => (requestedTab == null ? {} : { tab: requestedTab }),
    useSettings: () => ({
      settings: {},
      query: { isLoading: false },
      save: () => {},
      savedKeys: {},
      needsRestart: false,
      saveError: null,
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(sourceForTest("./settings-page.js", ["SettingsPage"]), context);
  return {
    ...components,
    Navigate,
    rendered: context.globalThis.__testExports.SettingsPage(),
    stateSetters,
  };
}

function renderSettingsTabs({ activeTab = "agent", isAdmin = false, mobile = false } = {}) {
  const calls = [];
  const context = {
    Icon: "Icon",
    SETTINGS_TABS,
    globalThis: {},
    html,
    React: { useMemo: (fn) => fn() },
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./components/settings-tabs.js", ["SettingsTabs", "SettingsTabsMobile"]),
    context,
  );
  const component = mobile
    ? context.globalThis.__testExports.SettingsTabsMobile
    : context.globalThis.__testExports.SettingsTabs;
  return {
    rendered: component({ activeTab, isAdmin, onTabChange: (id) => calls.push(id) }),
    calls,
  };
}

test("SettingsPage defaults admins to inference and members to language", () => {
  const admin = renderSettingsPage({ isAdmin: true });
  const member = renderSettingsPage({ isAdmin: false });

  assert.equal(findComponentNodes(admin.rendered, admin.InferenceTab).length, 1);
  assert.equal(findComponentNodes(member.rendered, member.LanguageTab).length, 1);
});

test("SettingsPage redirects unknown tabs to the role-specific default", () => {
  const admin = renderSettingsPage({ requestedTab: "missing", isAdmin: true });
  const member = renderSettingsPage({ requestedTab: "missing", isAdmin: false });

  assert.equal(componentProps(findComponentNodes(admin.rendered, admin.Navigate)[0], admin.Navigate).to, "/settings/inference");
  assert.equal(componentProps(findComponentNodes(member.rendered, member.Navigate)[0], member.Navigate).to, "/settings/language");
});

test("SettingsPage redirects non-admin operator tabs to language", () => {
  for (const requestedTab of ["inference", "users"]) {
    const result = renderSettingsPage({ requestedTab, isAdmin: false });
    const navigate = findComponentNodes(result.rendered, result.Navigate)[0];

    assert.equal(componentProps(navigate, result.Navigate).to, "/settings/language");
  }
});

test("SettingsTabs hide operator-only tabs for members and expose all tabs for admins", () => {
  assert.deepEqual(
    collectScalars(renderSettingsTabs({ isAdmin: false }).rendered).filter((value) =>
      value.startsWith("settings.")
    ),
    ["settings.agent", "settings.channels", "settings.networking", "settings.tools", "settings.skills", "settings.traceCommons", "settings.language"],
  );
  assert.deepEqual(
    collectScalars(renderSettingsTabs({ isAdmin: true }).rendered).filter((value) =>
      value.startsWith("settings.")
    ),
    SETTINGS_TABS.map((tab) => tab.labelKey),
  );
});

test("SettingsTabsMobile falls back to the first visible tab when active tab is hidden", () => {
  const rendered = renderSettingsTabs({ activeTab: "inference", isAdmin: false, mobile: true }).rendered;
  const labels = collectScalars(rendered).filter((value) => value.startsWith("settings."));

  assert.equal(labels[0], "settings.agent");
  assert.equal(labels.includes("settings.inference"), false);
});

test("SettingsTabs call onTabChange with the selected visible tab id", () => {
  const { rendered, calls } = renderSettingsTabs({ isAdmin: false });
  const handlers = [];
  visit(rendered, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes("onClick=")) handlers.push(node.values[index]);
    });
  });

  assert.equal(handlers.length, 7);
  handlers[0]();

  assert.deepEqual(calls, ["agent"]);
});
