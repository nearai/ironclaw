// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function toolsTabSourceForTest() {
  const source = readFileSync(new URL("./tools-tab.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ToolsTab };`;
}

const LABELS = {
  "tools.installed": "Installed tools",
  "tools.available": "Available tools",
};

function renderToolsTab(props) {
  const context = {
    ExtensionCard() {},
    RegistryCard() {},
    React: {},
    globalThis: {},
    useT: () => (key) => LABELS[key] || key,
  };
  vm.runInNewContext(toolsTabSourceForTest(), context);
  return context.globalThis.__testExports.ToolsTab(props);
}

function collectStringValues(node, out = []) {
  if (Array.isArray(node)) {
    for (const item of node) collectStringValues(item, out);
    return out;
  }
  if (typeof node === "string") {
    out.push(node);
    return out;
  }
  if (node && typeof node === "object" && Array.isArray(node.values)) {
    for (const value of node.values) collectStringValues(value, out);
  }
  return out;
}

test("ToolsTab available heading renders the label without a stray '$' marker", () => {
  const rendered = renderToolsTab({
    tools: [],
    toolRegistry: [{ package_ref: { id: "notion" } }],
    onActivate: () => {},
    onConfigure: () => {},
    onRemove: () => {},
    onInstall: () => {},
    isBusy: false,
  });

  const strings = collectStringValues(rendered);
  assert.ok(
    strings.includes("Available tools"),
    "expected the translated available-tools label in the heading",
  );
  assert.ok(
    !strings.includes("$"),
    "expected no leftover template-literal '$' text node in the heading",
  );
});
