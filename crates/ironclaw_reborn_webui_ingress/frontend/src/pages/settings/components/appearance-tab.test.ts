// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

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

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string" || typeof value === "boolean") {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function findComponentNode(root, component) {
  let found = null;
  visit(root, (node) => {
    if (!found && Array.isArray(node.values) && node.values.includes(component)) {
      found = node;
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

function renderAppearanceModule({ showChatLogsShortcut = true } = {}) {
  const toggles = [];
  const translations = {
    "settings.appearance": "Appearance",
    "settings.field.showChatTerminalShortcut": "Show chat terminal shortcut",
    "settings.field.showChatTerminalShortcutDesc":
      "Displays the floating terminal/logs icon inside chat threads.",
  };
  const context = {
    Card: component("Card"),
    Icon: component("Icon"),
    matchesSearch: (query, values) =>
      !query ||
      values.some((value) =>
        String(value || "").toLowerCase().includes(query.toLowerCase())
      ),
    SettingsSearchEmpty: "SettingsSearchEmpty",
    useInterfacePreferences: () => ({
      showChatLogsShortcut,
      setShowChatLogsShortcut: (value) => toggles.push(value),
    }),
    useT: () => (key) => translations[key] || key,
  };
  const exports = runVmModuleForTest(
    "./appearance-tab.tsx",
    ["AppearanceTab", "Switch"],
    context,
    import.meta.url
  );
  return { exports, toggles };
}

test("Appearance tab toggles the chat terminal shortcut preference", () => {
  const { exports, toggles } = renderAppearanceModule({
    showChatLogsShortcut: true,
  });
  const rendered = exports.AppearanceTab({});
  const switchNode = findComponentNode(rendered, exports.Switch);

  assert.ok(switchNode, "expected appearance tab to render a switch");
  assert.ok(collectScalars(rendered).includes("Show chat terminal shortcut"));

  const props = componentProps(switchNode, exports.Switch);
  assert.equal(props.checked, true);
  props.onChange(false);
  assert.deepEqual(toggles, [false]);
});

test("Appearance tab participates in settings search", () => {
  const { exports } = renderAppearanceModule();

  const rendered = exports.AppearanceTab({ searchQuery: "terminal" });
  assert.equal(findComponentNode(rendered, "SettingsSearchEmpty"), null);

  const empty = exports.AppearanceTab({ searchQuery: "unrelated" });
  assert.ok(findComponentNode(empty, "SettingsSearchEmpty"));
});
