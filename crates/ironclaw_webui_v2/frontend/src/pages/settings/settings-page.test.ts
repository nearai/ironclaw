import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../test-support/vm-module-harness";

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

function collectStrings(root) {
  const strings = [];
  visit(root, (node) => {
    if (Array.isArray(node.strings)) strings.push(...node.strings);
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string") strings.push(value);
    }
  });
  return strings;
}

test("SettingsPage leaves import failure feedback to SettingsToolbar", () => {
  const importError = new Error("No supported settings were found in the selected file");
  const SettingsToolbar = "SettingsToolbar";
  const context = {
    AgentTab: "AgentTab",
    AppearanceTab: "AppearanceTab",
    ChannelsTab: "ChannelsTab",
    InferenceTab: "InferenceTab",
    LanguageTab: "LanguageTab",
    Navigate: "Navigate",
    NetworkingTab: "NetworkingTab",
    React: {
      useEffect: () => {},
      useState: (initial) => [initial, () => {}],
    },
    RestartBanner: "RestartBanner",
    SettingsToolbar,
    SkillsTab: "SkillsTab",
    ToolsTab: "ToolsTab",
    TraceCommonsTab: "TraceCommonsTab",
    UsersTab: "UsersTab",
    useOutletContext: () => ({
      gatewayStatus: null,
      gatewayStatusQuery: null,
      isAdmin: false,
      theme: "system",
      setTheme: () => {},
    }),
    useParams: () => ({ tab: "language" }),
    useSettings: () => ({
      settings: {},
      query: { data: null, isLoading: false },
      save: () => {},
      savedKeys: {},
      needsRestart: false,
      importSettings: async () => {},
      isImporting: false,
      saveError: null,
      importError,
    }),
    useT: () => (key) => key,
  };
  const { SettingsPage } = runVmModuleForTest(
    "./settings-page.tsx",
    ["SettingsPage"],
    context,
    import.meta.url
  );

  const rendered = SettingsPage();
  const strings = collectStrings(rendered);
  assert.ok(strings.includes(SettingsToolbar), "expected toolbar-owned import feedback surface");
  assert.ok(!strings.includes("settings.importFailed"));
  assert.ok(!strings.includes(importError.message));
});
