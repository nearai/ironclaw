import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { subscribeToasts, toast } from "./toast.js";

function sourceForTest(path, exportNames, transform = (source) => source) {
  const source = transform(readFileSync(new URL(path, import.meta.url), "utf8"));
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
    lines.push(
      line
        .replace(/^export function /, "function ")
        .replace(/^export const /, "const "),
    );
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

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string") scalars.push(value);
    }
  });
  return scalars;
}

function findComponentNodes(root, component) {
  const nodes = [];
  visit(root, (node) => {
    if (Array.isArray(node.values) && node.values.includes(component)) nodes.push(node);
  });
  return nodes;
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

function templateIncludes(root, text) {
  let found = false;
  visit(root, (node) => {
    if (Array.isArray(node.strings) && node.strings.join("").includes(text)) {
      found = true;
    }
  });
  return found;
}

function renderToastViewportHarness() {
  let items = [];
  let subscribed = null;
  const timeouts = [];
  let effectRegistered = false;
  const context = {
    Icon: "Icon",
    globalThis: {},
    html,
    setTimeout: (fn, duration) => {
      timeouts.push({ fn, duration });
    },
    subscribeToasts: (listener) => {
      subscribed = listener;
      return () => {
        if (subscribed === listener) subscribed = null;
      };
    },
    React: {
      useEffect: (fn) => {
        if (!effectRegistered) {
          effectRegistered = true;
          fn();
        }
      },
      useState: () => [
        items,
        (next) => {
          items = typeof next === "function" ? next(items) : next;
        },
      ],
    },
  };

  vm.runInNewContext(
    sourceForTest("../components/toast-viewport.js", ["ToastViewport"]),
    context,
  );

  return {
    emit: (item) => subscribed(item),
    render: () => context.globalThis.__testExports.ToastViewport(),
    timeouts,
    get items() {
      return items;
    },
  };
}

test("toast publishes default and overridden payloads, then unsubscribes cleanly", () => {
  const received = [];
  const unsubscribe = subscribeToasts((item) => received.push(item));

  const firstId = toast("Saved");
  const secondId = toast("Failed", { tone: "error", duration: 900 });
  unsubscribe();
  toast("Ignored");

  assert.equal(secondId, firstId + 1);
  assert.deepEqual(
    received.map(({ message, tone, duration }) => ({ message, tone, duration })),
    [
      { message: "Saved", tone: "info", duration: 2600 },
      { message: "Failed", tone: "error", duration: 900 },
    ],
  );
});

test("toast notifies all active subscribers without replaying old items", () => {
  const first = [];
  const second = [];
  const unsubscribeFirst = subscribeToasts((item) => first.push(item.message));

  toast("first-only");
  const unsubscribeSecond = subscribeToasts((item) => second.push(item.message));
  toast("both");
  unsubscribeFirst();
  toast("second-only");
  unsubscribeSecond();

  assert.deepEqual(first, ["first-only", "both"]);
  assert.deepEqual(second, ["both", "second-only"]);
});

test("ToastViewport renders status rows, tone icons, and auto-removes expired items", () => {
  const harness = renderToastViewportHarness();

  assert.equal(harness.render(), null);

  harness.emit({ id: 10, message: "Settings saved", tone: "success", duration: 50 });
  const rendered = harness.render();
  const icons = findComponentNodes(rendered, "Icon");

  assert.equal(collectScalars(rendered).includes("Settings saved"), true);
  assert.equal(templateIncludes(rendered, 'role="status"'), true);
  assert.equal(componentProps(icons[0], "Icon").name, "check");
  assert.equal(harness.timeouts[0].duration, 50);

  harness.timeouts[0].fn();
  assert.equal(harness.items.length, 0);
  assert.equal(harness.render(), null);
});

test("ToastViewport falls back to info styling and bolt icon for unknown tones", () => {
  const harness = renderToastViewportHarness();

  assert.equal(harness.render(), null);
  harness.emit({ id: 20, message: "Odd tone", tone: "mystery", duration: 100 });
  const rendered = harness.render();
  const [icon] = findComponentNodes(rendered, "Icon");

  assert.equal(collectScalars(rendered).includes("Odd tone"), true);
  assert.equal(componentProps(icon, "Icon").name, "bolt");
  assert.equal(
    collectScalars(rendered).some((value) => value.includes("var(--v2-panel-border)")),
    true,
  );
});

test("queryClient keeps bounded non-refetching default query behavior", () => {
  const constructed = [];
  class QueryClient {
    constructor(config) {
      this.config = config;
      constructed.push(config);
    }
  }
  const context = { QueryClient, globalThis: {} };

  vm.runInNewContext(
    sourceForTest("./query-client.js", ["queryClient"]),
    context,
  );

  assert.equal(constructed.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(context.globalThis.__testExports.queryClient.config)), {
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false,
        retry: 1,
        staleTime: 10_000,
      },
    },
  });
});
