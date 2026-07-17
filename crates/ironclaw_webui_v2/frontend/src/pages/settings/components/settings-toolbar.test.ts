import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";
import {
  SettingsImportError,
  SettingsImportFailureReason,
} from "../lib/settings-api";

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

function findElement(root, predicate) {
  let found = null;
  visit(root, (node) => {
    if (!found && node.props && predicate(node)) found = node;
  });
  return found;
}

function collectText(root) {
  const text = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string") text.push(value);
    }
  });
  return text;
}

function createHarness(onImport) {
  const refs = [];
  const states = [];
  let refCursor = 0;
  let stateCursor = 0;

  class FileReader {
    result: string | null = null;
    onload: (() => void) | null = null;

    readAsText(file: { contents: string }) {
      this.result = file.contents;
      this.onload?.();
    }
  }

  const React = {
    useCallback: (fn) => fn,
    useEffect: () => {},
    useRef: (initial) => {
      const index = refCursor++;
      if (!refs[index]) refs[index] = { current: initial };
      return refs[index];
    },
    useState: (initial) => {
      const index = stateCursor++;
      if (!(index in states)) states[index] = initial;
      return [
        states[index],
        (next) => {
          states[index] = typeof next === "function" ? next(states[index]) : next;
        },
      ];
    },
  };

  const context = {
    Blob,
    Button: "Button",
    FileReader,
    Icon: "Icon",
    React,
    SettingsImportError,
    saveBlob: () => {},
    useT: () => (key: string, values: Record<string, string> = {}) => {
      if (key === "settings.importNoSupported") return "No supported settings found";
      if (key === "settings.importFailed") return `Import failed: ${values.message}`;
      return key;
    },
    window: {
      clearTimeout: () => {},
      setTimeout: () => 1,
    },
  };
  const { SettingsToolbar } = runVmModuleForTest(
    "./settings-toolbar.tsx",
    ["SettingsToolbar"],
    context,
    import.meta.url
  );

  return {
    render() {
      refCursor = 0;
      stateCursor = 0;
      return SettingsToolbar({
        settingsExport: null,
        onImport,
        isImporting: false,
        searchQuery: "",
        onSearchChange: () => {},
        onSearchClear: () => {},
        onBack: () => {},
        canGoBack: false,
      });
    },
  };
}

test("SettingsToolbar reports unsupported imports as an error instead of success", async () => {
  const harness = createHarness(async () => {
    throw new SettingsImportError({
      success: false,
      imported: 0,
      results: [],
      reason: SettingsImportFailureReason.NoSupportedSettings,
      message: "No supported settings were found in the selected file",
    });
  });
  let rendered = harness.render();
  const fileInput = findElement(
    rendered,
    (node) => node.type === "input" && node.props.type === "file"
  );
  assert.ok(fileInput, "expected settings import file input");

  await fileInput.props.onChange({
    target: { files: [{ contents: JSON.stringify({ settings: {} }) }] },
    currentTarget: { value: "selected" },
  });

  rendered = harness.render();
  const status = findElement(rendered, (node) => node.props.role === "status");
  assert.ok(status, "expected import status feedback");
  assert.ok(collectText(status).includes("No supported settings found"));
  assert.ok(!collectText(status).some((text) => text.includes("importSuccess")));
});
