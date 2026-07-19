// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const COPY = {
  "automations.empty.copied": "Copied",
  "automations.empty.copyPrompt": "Copy prompt",
  "automations.empty.example1": "Remind me every weekday morning to review alerts.",
  "automations.empty.example2": "Every Friday, summarize open customer escalations.",
  "automations.empty.example3": "At 9am tomorrow, ask me for deployment notes.",
  "automations.empty.examplesTitle": "Try one of these",
  "automations.empty.onboardingDescription": "Create automations by chatting with the agent.",
  "automations.empty.onboardingTitle": "No automations yet",
  "automations.empty.startInChat": "Start in chat",
};

function sourceForTest() {
  const source = readFileSync(new URL("./automations-empty-state.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { AutomationsEmptyState, ExamplePrompt };`;
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
      if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function componentProps(root, component) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (let index = 0; index < node.values.length; index += 1) {
      if (node.values[index] !== component) continue;
      const current = {};
      for (let propIndex = index + 1; propIndex < node.values.length; propIndex += 1) {
        const name = node.strings[propIndex]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
        if (name) current[name] = node.values[propIndex];
      }
      props.push(current);
    }
  });
  return props;
}

function nativeProps(root, tagName) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !node.strings.join("").includes(`<${tagName}`)) return;
    const current = {};
    node.strings.forEach((part, index) => {
      const name = part.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
      if (name) current[name] = node.values[index];
    });
    props.push(current);
  });
  return props;
}

function t(key) {
  return COPY[key] || key;
}

function createHarness({ clipboard = async () => {}, includeClipboard = true } = {}) {
  const state = [];
  let cursor = 0;
  let nextTimerId = 1;
  const navigations = [];
  const copiedText = [];
  const clearedTimers = [];
  const scheduledTimers = [];
  const cleanups = [];

  function Button() {}
  function Icon() {}
  function Panel() {}

  const React = {
    useEffect(effect) {
      const cleanup = effect();
      if (typeof cleanup === "function") cleanups.push(cleanup);
    },
    useRef(initial) {
      return { current: initial };
    },
    useState(initial) {
      const index = cursor;
      cursor += 1;
      if (!(index in state)) state[index] = initial;
      return [
        state[index],
        (next) => {
          state[index] = typeof next === "function" ? next(state[index]) : next;
        },
      ];
    },
  };

  const navigator = includeClipboard
    ? {
        clipboard: {
          writeText: async (text) => {
            copiedText.push(text);
            await clipboard(text);
          },
        },
      }
    : {};

  const context = {
    globalThis: {},
    Button,
    Icon,
    Panel,
    React,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    navigator,
    clearTimeout: (timerId) => {
      clearedTimers.push(timerId);
    },
    setTimeout: (callback, delay) => {
      const id = nextTimerId;
      nextTimerId += 1;
      scheduledTimers.push({ id, callback, delay });
      return id;
    },
    useNavigate: () => (path) => navigations.push(path),
    useT: () => t,
  };

  vm.runInNewContext(sourceForTest(), context);
  const exports = context.globalThis.__testExports;
  return {
    Button,
    Icon,
    copiedText,
    clearedTimers,
    exports,
    navigations,
    scheduledTimers,
    cleanup() {
      for (const cleanup of cleanups.splice(0)) cleanup();
    },
    renderEmpty() {
      cursor = 0;
      return exports.AutomationsEmptyState();
    },
    renderPrompt() {
      cursor = 0;
      return exports.ExamplePrompt({ promptKey: "automations.empty.example1" });
    },
  };
}

test("empty state renders onboarding actions and navigates to chat", () => {
  const harness = createHarness();

  const rendered = harness.renderEmpty();
  const labels = collectScalars(rendered);

  assert.ok(labels.includes("No automations yet"));
  assert.ok(labels.includes("Create automations by chatting with the agent."));
  assert.ok(labels.includes("Start in chat"));
  assert.equal(componentProps(rendered, harness.exports.ExamplePrompt).length, 3);

  const [button] = componentProps(rendered, harness.Button);
  button.onClick();

  assert.deepEqual(harness.navigations, ["/chat"]);
});

test("example prompt copies to clipboard and shows success state", async () => {
  const harness = createHarness();
  const initial = harness.renderPrompt();

  const [copyButton] = nativeProps(initial, "button");
  assert.equal(copyButton["aria-label"], "Copy prompt");

  await copyButton.onClick();

  assert.deepEqual(harness.copiedText, [
    "Remind me every weekday morning to review alerts.",
  ]);
  const copied = harness.renderPrompt();
  assert.equal(nativeProps(copied, "button")[0]["aria-label"], "Copied");
  assert.equal(componentProps(copied, harness.Icon)[0].name, "check");
});

test("example prompt replaces the success reset timer on rapid recopy", async () => {
  const harness = createHarness();
  const rendered = harness.renderPrompt();
  const [copyButton] = nativeProps(rendered, "button");

  await copyButton.onClick();
  await copyButton.onClick();

  assert.deepEqual(
    harness.scheduledTimers.map((timer) => timer.delay),
    [1500, 1500],
  );
  assert.ok(
    harness.clearedTimers.includes(1),
    "second copy must clear the first reset timer",
  );
});

test("example prompt clears the pending success reset timer on unmount", async () => {
  const harness = createHarness();
  const rendered = harness.renderPrompt();
  const [copyButton] = nativeProps(rendered, "button");

  await copyButton.onClick();
  harness.cleanup();

  assert.ok(
    harness.clearedTimers.includes(1),
    "unmount cleanup must clear the pending reset timer",
  );
});

test("example prompt ignores clipboard rejection and stays copyable", async () => {
  const harness = createHarness({
    clipboard: async () => {
      throw new Error("denied");
    },
  });

  const rendered = harness.renderPrompt();
  const [copyButton] = nativeProps(rendered, "button");

  await assert.doesNotReject(copyButton.onClick());

  const afterReject = harness.renderPrompt();
  assert.equal(nativeProps(afterReject, "button")[0]["aria-label"], "Copy prompt");
  assert.equal(componentProps(afterReject, harness.Icon)[0].name, "copy");
});

test("example prompt ignores missing clipboard API and stays copyable", async () => {
  const harness = createHarness({ includeClipboard: false });

  const rendered = harness.renderPrompt();
  const [copyButton] = nativeProps(rendered, "button");

  await assert.doesNotReject(copyButton.onClick());

  assert.deepEqual(harness.copiedText, []);
  const afterCopy = harness.renderPrompt();
  assert.equal(nativeProps(afterCopy, "button")[0]["aria-label"], "Copy prompt");
  assert.equal(componentProps(afterCopy, harness.Icon)[0].name, "copy");
});
