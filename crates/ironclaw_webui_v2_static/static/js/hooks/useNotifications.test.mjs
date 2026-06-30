// Run with:
//   node --test crates/ironclaw_webui_v2_static/static/js/hooks/useNotifications.test.mjs

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest() {
  const source = readFileSync(new URL("./useNotifications.js", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { useNotifications };`;
}

function createReactStub() {
  return {
    useCallback: (fn) => fn,
    useEffect: (fn) => {
      fn();
    },
    useMemo: (fn) => fn(),
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [
        value,
        (next) => {
          value = typeof next === "function" ? next(value) : next;
        },
      ];
    },
  };
}

function instantiate(queryState) {
  const baselineCalls = [];
  const context = {
    AUTOMATIONS_BASE_REFETCH_MS: 30_000,
    React: createReactStub(),
    useI18n: () => ({ t: (key) => key, lang: "en" }),
    useQuery: () => queryState,
    listAutomations: async () => ({}),
    normalizeAutomations: () => [],
    automationRunNotifications: () => [{ id: "message-1" }],
    ensureNotificationBaseline: (ids) => {
      baselineCalls.push(ids);
      return { initialized: true, seenIds: new Set(ids) };
    },
    getNotificationState: () => ({ initialized: false, seenIds: new Set() }),
    markNotificationIdsSeen: () => ({ initialized: true, seenIds: new Set() }),
    subscribeNotifications: () => () => {},
    globalThis: {},
  };
  vm.runInNewContext(sourceForTest(), context);
  const hook = context.globalThis.__testExports.useNotifications({
    profile: { tenant_id: "tenant", user_id: "user" },
  });
  return { hook, baselineCalls };
}

test("does not baseline notifications before the automations query succeeds", () => {
  const { baselineCalls } = instantiate({
    data: undefined,
    isLoading: true,
    isSuccess: false,
    error: null,
    refetch: () => {},
  });

  assert.deepEqual(baselineCalls, []);
});

test("baselines the current notification ids after the first successful query", () => {
  const { baselineCalls, hook } = instantiate({
    data: { automations: [] },
    isLoading: false,
    isSuccess: true,
    error: null,
    refetch: () => {},
  });

  assert.deepEqual(baselineCalls, [["message-1"]]);
  assert.equal(hook.unreadCount, 0);
});

