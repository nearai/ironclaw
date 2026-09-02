// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function activityRunSourceForTest() {
  const source = readFileSync(new URL("./activity-run.tsx", import.meta.url), "utf8");
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
    lines.push(line.replace("export function ActivityRun", "function ActivityRun"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ActivityRun, ActivityItem };`;
}

test("ActivityRun keeps running tool activity collapsed by default", () => {
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon() {},
    MarkdownRenderer() {},
    React: {
      useMemo: (factory) => factory(),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    summarizeActivity: () => ({
      label: "Activity - 1 tool, running",
      hasError: false,
    }),
    useT: () => (key) => key,
    ToolActivity() {},
  };

  vm.runInNewContext(activityRunSourceForTest(), context);
  const tree = context.globalThis.__testExports.ActivityRun({
    activity: [
      {
        id: "tool-search",
        role: "tool_activity",
        toolName: "web-access.search",
        toolStatus: "running",
      },
    ],
  });

  assert.ok(containsScalar(tree, "false"));
  assert.equal(hasComponentNamed(tree, "ActivityItem"), false);
});

test("ActivityRun keeps declined tool activity collapsed", () => {
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon() {},
    MarkdownRenderer() {},
    React: {
      useMemo: (factory) => factory(),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    summarizeActivity: () => ({
      label: "Activity - 1 tool, 1 declined",
      hasError: false,
      hasDeclined: true,
    }),
    useT: () => (key) => key,
    ToolActivity() {},
  };

  vm.runInNewContext(activityRunSourceForTest(), context);
  const tree = context.globalThis.__testExports.ActivityRun({
    activity: [
      {
        id: "tool-install",
        role: "tool_activity",
        toolName: "extension_install",
        toolStatus: "declined",
      },
    ],
  });

  assert.ok(containsScalar(tree, "false"));
  assert.equal(hasComponentNamed(tree, "ActivityItem"), false);
});

test("ActivityRun keeps failed nested tool activity collapsed", () => {
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon() {},
    MarkdownRenderer() {},
    React: {
      useMemo: (factory) => factory(),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    summarizeActivity: () => ({
      label: "Activity - 1 tool, 1 failed",
      hasError: true,
    }),
    useT: () => (key) => key,
    ToolActivity() {},
  };

  vm.runInNewContext(activityRunSourceForTest(), context);
  const tree = context.globalThis.__testExports.ActivityRun({
    activity: [
      {
        id: "assistant-tool-call",
        role: "assistant",
        toolCalls: [
          {
            id: "tool-search",
            toolName: "web-access.search",
            toolStatus: "error",
          },
        ],
      },
    ],
  });

  assert.ok(containsScalar(tree, "false"));
  assert.equal(hasComponentNamed(tree, "ActivityItem"), false);
});

test("ActivityRun keeps reasoning activity collapsed", () => {
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon() {},
    MarkdownRenderer() {},
    React: {
      useMemo: (factory) => factory(),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    summarizeActivity: () => ({
      label: "Activity",
      hasError: false,
    }),
    useT: () => (key) => key,
    ToolActivity() {},
  };

  vm.runInNewContext(activityRunSourceForTest(), context);
  const tree = context.globalThis.__testExports.ActivityRun({
    activity: [
      {
        id: "reasoning",
        role: "thinking",
        content: "Considering the available evidence.",
      },
    ],
  });

  assert.ok(containsScalar(tree, "false"));
  assert.equal(hasComponentNamed(tree, "ActivityItem"), false);
});

function hasComponentNamed(node, name) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return false;
  if (node.values.some((value) => typeof value === "function" && value.name === name)) {
    return true;
  }
  return node.values.some((value) => {
    if (Array.isArray(value)) return value.some((item) => hasComponentNamed(item, name));
    return hasComponentNamed(value, name);
  });
}

function containsScalar(node, expected) {
  if (node === expected) return true;
  if (Array.isArray(node)) return node.some((item) => containsScalar(item, expected));
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return false;
  return node.values.some((value) => containsScalar(value, expected));
}

test("ActivityRun renders a narration phase as a settled note inside the run", () => {
  const rendered = [];
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon(props) {
      rendered.push(`icon:${props.name}`);
    },
    MarkdownRenderer(props) {
      rendered.push(`markdown:${props.content}:streaming=${props.streaming}`);
    },
    React: {
      useMemo: (factory) => factory(),
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    summarizeActivity: () => ({ label: "Activity", hasError: false }),
    useT: () => (key) => key,
    ToolActivity() {},
    messageBelongsToActiveRun: () => false,
  };

  vm.runInNewContext(activityRunSourceForTest(), context);
  const item = context.globalThis.__testExports.ActivityItem({
    item: {
      id: "text-text:run-1:1",
      role: "assistant",
      content: "Let me look.",
      isFinalReply: false,
      isNarration: true,
      turnRunId: "run-1",
    },
    activeRunId: "run-1",
  });

  assert.ok(item, "a narration phase is an activity item, never dropped");
  assert.equal(
    hasComponentNamed(item, "NoteItem"),
    true,
    "narration renders as a note like reasoning does",
  );
});
