import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function gatewayLayoutSourceForTest() {
  const source = readFileSync(new URL("./gateway-layout.js", import.meta.url), "utf8");
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
    lines.push(line.replace("export function GatewayLayout", "function GatewayLayout"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { GatewayLayout };`;
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

function renderGatewayLayout(pathname) {
  const threadsState = {
    threads: [{ id: "thread-1", title: "Chat one" }],
    activeThreadId: "thread-1",
    isCreating: false,
    setActiveThreadId: () => {},
    deleteThread: async () => {},
  };
  const context = {
    CommandPalette() {},
    Navigate() {},
    Outlet() {},
    PageHeader() {},
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    Sidebar() {},
    ToastViewport() {},
    cn: (...classes) => classes.filter(Boolean).join(" "),
    deleteThreadErrorMessage: () => "delete failed",
    globalThis: {},
    html,
    shouldRouteToOnboarding: () => false,
    toast: () => {},
    useGatewayStatus: () => ({ data: { ok: true }, error: null }),
    useInterfaceTheme: () => ({ theme: "light", toggleTheme: () => {} }),
    useLlmProviders: () => ({
      hasActiveProvider: true,
      isError: false,
      isLoading: false,
    }),
    useLocation: () => ({ pathname }),
    useNavigate: () => () => {},
    useSidebar: () => ({
      close: () => {},
      newChat: () => {},
      open: true,
      selectThread: () => {},
      toggle: () => {},
    }),
    useT: () => (key) => key,
    useThreads: () => threadsState,
  };

  vm.runInNewContext(gatewayLayoutSourceForTest(), context);
  const tree = context.globalThis.__testExports.GatewayLayout({
    isAdmin: false,
    onSignOut: () => {},
    profile: null,
    token: "token",
  });
  const sidebar = findComponent(tree, context.Sidebar);
  assert.ok(sidebar, "GatewayLayout should render Sidebar");
  return componentProps(sidebar, context.Sidebar);
}

test("GatewayLayout clears the sidebar active thread outside chat routes", () => {
  const props = renderGatewayLayout("/automations");

  assert.equal(props.activeThreadId, null);
});

test("GatewayLayout keeps the sidebar active thread on chat routes", () => {
  const props = renderGatewayLayout("/chat/thread-1");

  assert.equal(props.activeThreadId, "thread-1");
});
