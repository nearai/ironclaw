// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function mcpTabSourceForTest() {
  const source = readFileSync(new URL("./mcp-tab.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { McpTab };`;
}

const LABELS = {
  "mcp.installed": "Installed MCP servers",
  "mcp.available": "Available MCP servers",
};

function renderMcpTab(props) {
  const context = {
    ExtensionCard() {},
    RegistryCard() {},
    React: {},
    globalThis: {},
    useT: () => (key) => LABELS[key] || key,
  };
  vm.runInNewContext(mcpTabSourceForTest(), context);
  return context.globalThis.__testExports.McpTab(props);
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

test("McpTab available heading renders the label without a stray '$' marker", () => {
  const rendered = renderMcpTab({
    mcpServers: [],
    mcpRegistry: [{ package_ref: { id: "notion" } }],
    onActivate: () => {},
    onConfigure: () => {},
    onRemove: () => {},
    onInstall: () => {},
    isBusy: false,
  });

  const strings = collectStringValues(rendered);
  assert.ok(
    strings.includes("Available MCP servers"),
    "expected the translated available-servers label in the heading",
  );
  assert.ok(
    !strings.includes("$"),
    "expected no leftover template-literal '$' text node in the heading",
  );
});
