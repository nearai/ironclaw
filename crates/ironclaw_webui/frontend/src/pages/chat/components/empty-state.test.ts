// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// Same vm-harness convention as chat-input.test.ts: read the real source,
// strip imports, rename the export, and stub the module's free variables
// (including its child components) so the vm can evaluate it directly. This
// is the only way to observe what EmptyState actually forwards to its
// internal ChatInput — chat.test.ts stubs both components as no-ops and so
// cannot see prop plumbing between them.
function emptyStateSourceForTest() {
  const source = readFileSync(
    new URL("./empty-state.tsx", import.meta.url),
    "utf8",
  );
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
    lines.push(line.replace("export function EmptyState", "function EmptyState"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { EmptyState };`;
}

function findComponent(node, component) {
  if (!node || typeof node !== "object") return null;
  if (!Array.isArray(node.values)) return null;
  const componentIndex = node.values.indexOf(component);
  if (componentIndex >= 0) {
    return node;
  }
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

const HTML_ATTRIBUTE_PATTERN = /([A-Za-z][A-Za-z0-9-]*)=\s*$/;

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(HTML_ATTRIBUTE_PATTERN)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function renderEmptyState(props = {}) {
  const components = {
    Icon() {},
    ChatInput() {},
  };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
  };
  vm.runInNewContext(emptyStateSourceForTest(), context);
  const tree = context.globalThis.__testExports.EmptyState({
    onSuggestion: () => {},
    onSend: () => {},
    disabled: false,
    sendDisabled: false,
    initialText: "",
    resetKey: "",
    draftKey: "draft-key",
    context: {},
    statusText: "",
    canCancel: false,
    onCancel: () => {},
    ...props,
  });
  return { tree, components };
}

test("EmptyState forwards a non-empty commands list to its composer so the menu is reachable from the landing view", () => {
  const commands = [
    { name: "status", title: "Status", description: "d", usage: "/status" },
  ];
  const { tree, components } = renderEmptyState({ commands });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.deepEqual(props.commands, commands);
});
