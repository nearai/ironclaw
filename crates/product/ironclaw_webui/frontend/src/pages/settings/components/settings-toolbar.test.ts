// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function visitNode(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visitNode(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  visitNode(node.children, fn);
  visitNode(node.values, fn);
}

function findComponentProps(root, component) {
  let props = null;
  visitNode(root, (node) => {
    if (!props && node.type === component) props = node.props;
  });
  assert.ok(props, "expected SettingsToolbar to render SearchField");
  return props;
}

test("SettingsToolbar supplies the controlled SearchField contract", () => {
  function Button() {}
  function Icon() {}
  function SearchField() {}
  const onSearchChange = () => {};
  const onSearchClear = () => {};
  const context = {
    Blob,
    Button,
    FileReader: function FileReader() {},
    Icon,
    NoSupportedSettingsImportError: class extends Error {},
    React: {
      useCallback: (callback) => callback,
      useEffect: () => {},
      useRef: (value) => ({ current: value }),
      useState: (value) => [value, () => {}],
    },
    SearchField,
    saveBlob: () => {},
    useFilePicker: () => [() => {}, {}],
    useT: () => (key) => ({
      "settings.clearSearch": "Clear search",
      "settings.searchPlaceholder": "Search settings...",
    })[key] || key,
    window: {
      clearTimeout: () => {},
      setTimeout: () => 1,
    },
  };
  const { SettingsToolbar } = runVmModuleForTest(
    "./settings-toolbar.tsx",
    ["SettingsToolbar"],
    context,
    import.meta.url,
  );
  const rendered = SettingsToolbar({
    settingsExport: null,
    onImport: () => {},
    isImporting: false,
    searchQuery: "model",
    onSearchChange,
    onSearchClear,
    canGoBack: false,
  });

  const searchProps = findComponentProps(rendered, SearchField);
  assert.equal(searchProps.value, "model");
  assert.equal(searchProps.placeholder, "Search settings...");
  assert.equal(searchProps["aria-label"], "Search settings...");
  assert.equal(searchProps.clearLabel, "Clear search");
  assert.equal(searchProps.onChange, onSearchChange);
  assert.equal(searchProps.onClear, onSearchClear);
});
