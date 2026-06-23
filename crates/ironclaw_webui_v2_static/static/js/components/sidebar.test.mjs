import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sidebarSourceForTest() {
  const source = readFileSync(new URL("./sidebar.js", import.meta.url), "utf8");
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
    lines.push(line.replace("export function Sidebar", "function Sidebar"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { Sidebar };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function findComponent(node, component) {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findComponent(child, component);
      if (found) return found;
    }
    return null;
  }
  if (!node || typeof node !== "object") return null;
  if (Array.isArray(node.values) && node.values.includes(component)) return node;
  if (!Array.isArray(node.values)) return null;
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
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

function renderSidebar(props = {}) {
  const context = {
    Link() {},
    SidebarFooter() {},
    SidebarNav() {},
    SidebarThreads() {},
    SidebarTraceCredits() {},
    globalThis: {},
    html,
  };
  const threadsState = {
    activeThreadId: "thread-1",
    isCreating: false,
    threads: [{ id: "thread-1", title: "Chat one" }],
  };

  vm.runInNewContext(sidebarSourceForTest(), context);
  const tree = context.globalThis.__testExports.Sidebar({
    isAdmin: false,
    onClose: () => {},
    onDeleteThread: () => {},
    onNewChat: () => {},
    onSelectThread: () => {},
    onSignOut: () => {},
    profile: null,
    theme: "light",
    threadsState,
    toggleTheme: () => {},
    ...props,
  });
  const threads = findComponent(tree, context.SidebarThreads);
  assert.ok(threads, "Sidebar should render SidebarThreads");
  return componentProps(threads, context.SidebarThreads);
}

test("Sidebar lets callers clear the active thread highlight", () => {
  const props = renderSidebar({ activeThreadId: null });

  assert.equal(props.activeThreadId, null);
});

test("Sidebar falls back to the thread state's active id", () => {
  const props = renderSidebar();

  assert.equal(props.activeThreadId, "thread-1");
});
