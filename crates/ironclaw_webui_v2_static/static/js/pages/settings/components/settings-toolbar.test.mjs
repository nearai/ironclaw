import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

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

function propsAfterComponent(node, component, start) {
  const props = {};
  for (let index = start + 1; index < node.values.length; index += 1) {
    if (node.values[index] === component) break;
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function componentInstances(root, component) {
  const instances = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    node.values.forEach((value, index) => {
      if (value === component) instances.push(propsAfterComponent(node, component, index));
    });
  });
  return instances;
}

function propValues(root, propName) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.values.forEach((value, index) => {
      const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
      if (name === propName) values.push(value);
    });
  });
  return values;
}

function templateIncludes(root, text) {
  let found = false;
  visit(root, (node) => {
    if (Array.isArray(node.strings) && node.strings.join("").includes(text)) {
      found = true;
    }
  });
  return found;
}

function renderToolbar(props = {}) {
  const savedBlobs = [];
  const stateUpdates = [];
  const timers = [];
  const fileInputRef = { current: { clicked: false, click() { this.clicked = true; } } };
  const context = {
    Blob: class Blob {
      constructor(parts, options) {
        this.parts = parts;
        this.type = options?.type || "";
      }
    },
    Button: "Button",
    FileReader: class FileReader {
      readAsText(file) {
        this.result = file.text;
        if (file.failRead) {
          this.error = new Error("read failed");
          this.onerror();
        } else {
          this.onload();
        }
      }
    },
    Icon: "Icon",
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: () => fileInputRef,
      useState: (initial) => [initial, (next) => stateUpdates.push(next)],
    },
    globalThis: {},
    html,
    saveBlob: (blob, filename) => savedBlobs.push({ blob, filename }),
    useT: () => (key, values = {}) => `${key}${values.message ? `:${values.message}` : ""}`,
    window: {
      clearTimeout: () => {},
      setTimeout: (fn, duration) => {
        timers.push({ fn, duration });
        return timers.length;
      },
    },
  };

  vm.runInNewContext(sourceForTest("./settings-toolbar.js", ["SettingsToolbar"]), context);
  const rendered = context.globalThis.__testExports.SettingsToolbar({
    settingsExport: { settings: { "agent.auto_approve_tools": false } },
    onImport: async () => {},
    isImporting: false,
    searchQuery: "",
    onSearchChange: () => {},
    onSearchClear: () => {},
    onBack: () => {},
    canGoBack: false,
    ...props,
  });
  return { fileInputRef, rendered, savedBlobs, stateUpdates, timers };
}

function secondaryButtons(root) {
  return componentInstances(root, "Button").filter((props) => Object.hasOwn(props, "disabled"));
}

function exportButton(root) {
  return secondaryButtons(root)[0];
}

function importButton(root) {
  return secondaryButtons(root)[1];
}

function onChangeHandlers(root) {
  return propValues(root, "onChange");
}

function onClickHandlers(root) {
  return propValues(root, "onClick");
}

test("SettingsToolbar renders search file controls and disables unavailable actions", () => {
  const ready = renderToolbar();
  const importing = renderToolbar({ isImporting: true });
  const noExport = renderToolbar({ settingsExport: null });

  assert.equal(templateIncludes(ready.rendered, 'type="search"'), true);
  assert.equal(templateIncludes(ready.rendered, 'type="file"'), true);
  assert.equal(templateIncludes(ready.rendered, 'accept=".json,application/json"'), true);
  assert.equal(exportButton(ready.rendered).disabled, false);
  assert.equal(importButton(ready.rendered).disabled, false);
  assert.equal(exportButton(importing.rendered).disabled, true);
  assert.equal(importButton(importing.rendered).disabled, true);
  assert.equal(exportButton(noExport.rendered).disabled, true);
});

test("SettingsToolbar exports the current settings payload as JSON", () => {
  const toolbar = renderToolbar();

  exportButton(toolbar.rendered).onClick();

  assert.equal(toolbar.savedBlobs[0].filename, "ironclaw-settings.json");
  assert.equal(toolbar.savedBlobs[0].blob.type, "application/json");
  assert.deepEqual(
    JSON.parse(toolbar.savedBlobs[0].blob.parts[0]),
    { settings: { "agent.auto_approve_tools": false } },
  );
  assert.equal(toolbar.stateUpdates.some((value) => value?.tone === "success"), true);
  assert.equal(toolbar.timers.some((timer) => timer.duration === 3500), true);
});

test("SettingsToolbar wires search back clear and import picker actions", () => {
  const calls = [];
  const toolbar = renderToolbar({
    canGoBack: true,
    searchQuery: "agent",
    onBack: () => calls.push(["back"]),
    onSearchChange: (value) => calls.push(["search", value]),
    onSearchClear: () => calls.push(["clear"]),
  });

  componentInstances(toolbar.rendered, "Button").find((props) => !Object.hasOwn(props, "disabled")).onClick();
  onChangeHandlers(toolbar.rendered)[0]({ target: { value: "network" } });
  onClickHandlers(toolbar.rendered).at(-1)();
  importButton(toolbar.rendered).onClick();

  assert.deepEqual(calls, [["back"], ["search", "network"], ["clear"]]);
  assert.equal(toolbar.fileInputRef.current.clicked, true);
});

test("SettingsToolbar imports only valid settings JSON and always clears file input", async () => {
  const imported = [];
  const toolbar = renderToolbar({ onImport: async (payload) => imported.push(payload) });
  const importFile = onChangeHandlers(toolbar.rendered).at(-1);
  const validEvent = {
    target: { files: [{ text: JSON.stringify({ settings: { theme: "dark" } }) }], value: "valid.json" },
  };

  await importFile(validEvent);
  assert.deepEqual(JSON.parse(JSON.stringify(imported)), [{ settings: { theme: "dark" } }]);
  assert.equal(validEvent.target.value, "");

  for (const text of [
    "{",
    JSON.stringify({}),
    JSON.stringify({ settings: [] }),
    JSON.stringify({ settings: "bad" }),
  ]) {
    const invalidEvent = { target: { files: [{ text }], value: "bad.json" } };
    await importFile(invalidEvent);
    assert.equal(invalidEvent.target.value, "");
  }
  assert.equal(imported.length, 1);
  assert.equal(toolbar.stateUpdates.some((value) => value?.tone === "error"), true);
});

test("SettingsToolbar ignores empty file selections", async () => {
  const toolbar = renderToolbar({
    onImport: async () => {
      throw new Error("empty file selection must not import");
    },
  });
  const event = { target: { files: [], value: "empty" } };

  await onChangeHandlers(toolbar.rendered).at(-1)(event);

  assert.equal(event.target.value, "");
  assert.deepEqual(toolbar.stateUpdates, []);
});

test("settings search covers keys labels groups arrays objects and primitive values", () => {
  const sections = [
    {
      groupKey: "settings.group.agent",
      fields: [
        { key: "agent.mode", labelKey: "settings.field.mode", descKey: "settings.field.modeDesc" },
        { key: "agent.tags", label: "Tags" },
        { key: "agent.object", label: "Nested" },
      ],
    },
  ];
  const settings = {
    "agent.mode": "careful",
    "agent.tags": ["alpha", "beta"],
    "agent.object": { provider: "nearai" },
  };
  const t = (key) => ({
    "settings.group.agent": "Agent settings",
    "settings.field.mode": "Execution mode",
    "settings.field.modeDesc": "Controls behavior",
  }[key] || key);

  assert.equal(matchesSearch("NEARAI", [settings["agent.object"]]), true);
  assert.deepEqual(
    filterSettingsSections(sections, settings, "execution", t)[0].fields.map((field) => field.key),
    ["agent.mode"],
  );
  assert.deepEqual(
    filterSettingsSections(sections, settings, "beta", t)[0].fields.map((field) => field.key),
    ["agent.tags"],
  );
  assert.equal(filterSettingsSections(sections, settings, "missing", t).length, 0);
});
