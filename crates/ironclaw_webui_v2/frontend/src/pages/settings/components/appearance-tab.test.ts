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

function renderAppearanceModule({
  showChatLogsShortcut = true,
  theme = "light",
} = {}) {
  const toggles = [];
  const themeChanges = [];
  const translations = {
    "settings.appearance": "Appearance",
    "theme.light": "Light theme",
    "theme.dark": "Dark theme",
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
    ["AppearanceTab", "Switch", "ThemeOption"],
    context,
    import.meta.url
  );
  const render = (props = {}) => exports.AppearanceTab({
    theme,
    onThemeChange: (nextTheme) => themeChanges.push(nextTheme),
    ...props,
  });
  return { exports, render, themeChanges, toggles };
}

test("Appearance tab selects the shared interface theme", () => {
  const { exports, render, themeChanges } = renderAppearanceModule({ theme: "light" });
  const rendered = render();
  const options = findComponentNodes(rendered, exports.ThemeOption);

  assert.equal(options.length, 2);
  const light = componentProps(options[0], exports.ThemeOption);
  const dark = componentProps(options[1], exports.ThemeOption);
  assert.equal(light.label, "Light theme");
  assert.equal(light.checked, true);
  assert.equal(light.value, "light");
  assert.equal(dark.label, "Dark theme");
  assert.equal(dark.checked, false);
  assert.equal(dark.value, "dark");

  dark.onSelect();
  assert.deepEqual(themeChanges, ["dark"]);
});

test("Appearance tab toggles the chat terminal shortcut preference", () => {
  const { exports, render, toggles } = renderAppearanceModule({
    showChatLogsShortcut: true,
  });
  const rendered = render();
  const switchNode = findComponentNode(rendered, exports.Switch);

  assert.ok(switchNode, "expected appearance tab to render a switch");
  assert.ok(collectScalars(rendered).includes("Show chat terminal shortcut"));

  const props = componentProps(switchNode, exports.Switch);
  assert.equal(props.checked, true);
  props.onChange(false);
  assert.deepEqual(toggles, [false]);
});

test("Appearance tab participates in settings search", () => {
  const { render } = renderAppearanceModule();

  const rendered = render({ searchQuery: "dark" });
  assert.equal(findComponentNode(rendered, "SettingsSearchEmpty"), null);

  const empty = render({ searchQuery: "unrelated" });
  assert.ok(findComponentNode(empty, "SettingsSearchEmpty"));
});
