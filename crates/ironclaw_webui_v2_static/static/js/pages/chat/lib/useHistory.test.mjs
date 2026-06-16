import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useHistorySourceForTest() {
  const source = readFileSync(
    new URL("../hooks/useHistory.js", import.meta.url),
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
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join(
    "\n",
  )}\nglobalThis.__testExports = { useHistory, mergePreservingClientOnly };`;
}

function createReactStub({ setCalls = [] } = {}) {
  return {
    useCallback: (fn) => fn,
    useEffect: (fn) => {
      fn();
    },
    useRef: (value) => ({ current: value }),
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [
        value,
        (next) => {
          value = typeof next === "function" ? next(value) : next;
          setCalls.push(value);
        },
      ];
    },
  };
}

async function flushMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
}

test("useHistory records a load error when timeline fetch fails", async () => {
  const setCalls = [];
  const consoleErrors = [];
  const context = {
    authScope: () => "scope-1",
    console: {
      error: (...args) => consoleErrors.push(args),
    },
    fetchTimeline: async () => {
      throw new Error("timeline unavailable");
    },
    globalThis: {},
    messagesFromTimeline: () => {
      throw new Error("failed timeline should not be transformed");
    },
    React: createReactStub({ setCalls }),
  };

  vm.runInNewContext(useHistorySourceForTest(), context);
  context.globalThis.__testExports.useHistory("thread-1", {});
  await flushMicrotasks();

  assert.equal(setCalls.at(-1).isLoading, false);
  assert.equal(
    setCalls.at(-1).loadError,
    "Failed to load conversation history.",
  );
  assert.equal(consoleErrors.length, 1);
});

test("mergePreservingClientOnly keeps err-* bubbles and lets the timeline win otherwise", () => {
  const context = { globalThis: {}, React: createReactStub() };
  vm.runInNewContext(useHistorySourceForTest(), context);
  const { mergePreservingClientOnly } = context.globalThis.__testExports;

  const timeline = [
    { id: "msg-user-1", role: "user" },
    { id: "tool-abc", role: "tool_activity", toolParameters: "{}", toolResultPreview: "ok" },
    { id: "msg-assistant-1", role: "assistant" },
  ];
  const current = [
    { id: "msg-user-1", role: "user" },
    { id: "tool-abc", role: "tool_activity", toolParameters: null, toolResultPreview: null },
    { id: "err-run-1", role: "error", content: "run failed" },
  ];

  const merged = mergePreservingClientOnly(timeline, current);

  // Timeline order is authoritative and the rich tool card replaces the
  // sparse live one; the client-only err-* bubble is preserved at the end.
  assert.equal(
    merged.map((m) => m.id).join(","),
    "msg-user-1,tool-abc,msg-assistant-1,err-run-1",
  );
  const toolCard = merged.find((m) => m.id === "tool-abc");
  assert.equal(toolCard.toolParameters, "{}");
  assert.equal(toolCard.toolResultPreview, "ok");
});
