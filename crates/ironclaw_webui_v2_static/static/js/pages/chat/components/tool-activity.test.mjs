import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function toolActivitySourceForTest() {
  const source = readFileSync(new URL("./tool-activity.js", import.meta.url), "utf8");
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
        .replace("export const TOOL_RUN_COLLAPSE_AFTER", "const TOOL_RUN_COLLAPSE_AFTER")
        .replace("export function ToolActivity", "function ToolActivity")
        .replace("export function ToolRun", "function ToolRun"),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ToolActivityCard };`;
}

function renderToolCard(activity) {
  return createToolCardHarness().render(activity);
}

function createToolCardHarness() {
  const hooks = {
    state: [],
    refs: [],
    setters: [],
    effectDeps: [],
    hookIndex: 0,
    pendingEffects: [],
    changed: false,
  };
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon() {},
    React: {
      useEffect: (effect, deps) => {
        const index = hooks.hookIndex++;
        const previous = hooks.effectDeps[index];
        hooks.effectDeps[index] = deps;
        if (!previous || depsChanged(previous, deps)) hooks.pendingEffects.push(effect);
      },
      useId: () => "tool-card-detail",
      useMemo: (factory) => factory(),
      useRef: (initial) => {
        const index = hooks.hookIndex++;
        if (!(index in hooks.refs)) {
          hooks.refs[index] = { current: initial };
        }
        return hooks.refs[index];
      },
      useState: (initial) => {
        const index = hooks.hookIndex++;
        if (!(index in hooks.state)) {
          hooks.state[index] = typeof initial === "function" ? initial() : initial;
        }
        const setter = (value) => {
          const next = typeof value === "function" ? value(hooks.state[index]) : value;
          if (Object.is(hooks.state[index], next)) return;
          hooks.state[index] = next;
          hooks.changed = true;
        };
        hooks.setters[index] = setter;
        return [hooks.state[index], setter];
      },
    },
    useT: () => (key) => key,
  };

  vm.runInNewContext(toolActivitySourceForTest(), context);
  const ToolActivityCard = context.globalThis.__testExports.ToolActivityCard;

  return {
    collapse() {
      hooks.setters[0]?.(false);
    },
    render(activity) {
      let tree = null;
      for (let attempt = 0; attempt < 5; attempt += 1) {
        hooks.hookIndex = 0;
        hooks.pendingEffects = [];
        hooks.changed = false;
        tree = ToolActivityCard({ activity });
        for (const effect of hooks.pendingEffects) effect();
        if (!hooks.changed) return tree;
      }
      return tree;
    },
  };
}

function depsChanged(previous, next) {
  if (!Array.isArray(previous) || !Array.isArray(next)) return true;
  if (previous.length !== next.length) return true;
  return next.some((value, index) => !Object.is(value, previous[index]));
}

function runningActivity(overrides = {}) {
  return {
    toolName: "search",
    toolStatus: "running",
    toolDetail: null,
    toolError: null,
    toolDurationMs: null,
    toolParameters: null,
    toolResultPreview: null,
    ...overrides,
  };
}

test("ToolActivityCard auto-expands running activity when live details arrive", () => {
  const tree = renderToolCard(
    runningActivity({
      toolDetail: "deployment status",
      toolParameters: "query: deployment status",
    }),
  );

  assert.ok(hasValue(tree, "true"));
  assert.ok(hasComponentNamed(tree, "ToolDetailPanel"));
});

test("ToolActivityCard keeps empty running activity collapsed", () => {
  const tree = renderToolCard(runningActivity());

  assert.ok(hasValue(tree, "false"));
  assert.equal(hasComponentNamed(tree, "ToolDetailPanel"), false);
});

test("ToolActivityCard expands when live details arrive on an existing running card", () => {
  const harness = createToolCardHarness();
  const initial = harness.render(runningActivity());

  assert.ok(hasValue(initial, "false"));
  assert.equal(hasComponentNamed(initial, "ToolDetailPanel"), false);

  const updated = harness.render(
    runningActivity({
      toolDetail: "deployment status",
      toolParameters: "query: deployment status",
    }),
  );

  assert.ok(hasValue(updated, "true"));
  assert.ok(hasComponentNamed(updated, "ToolDetailPanel"));
});

test("ToolActivityCard keeps a manually collapsed running card collapsed on later live updates", () => {
  const harness = createToolCardHarness();
  const running = runningActivity({
    toolDetail: "deployment status",
    toolParameters: "query: deployment status",
  });

  assert.ok(hasComponentNamed(harness.render(running), "ToolDetailPanel"));

  harness.collapse();
  const collapsed = harness.render(running);
  assert.ok(hasValue(collapsed, "false"));
  assert.equal(hasComponentNamed(collapsed, "ToolDetailPanel"), false);

  const updated = runningActivity({
    toolDetail: "deployment status: still running",
    toolParameters: "query: deployment status",
    toolResultPreview: "waiting for deployment",
  });
  const stillCollapsed = harness.render(updated);
  assert.ok(hasValue(stillCollapsed, "false"));
  assert.equal(hasComponentNamed(stillCollapsed, "ToolDetailPanel"), false);
});

test("ToolActivityCard expands when a successful preview arrives", () => {
  const harness = createToolCardHarness();
  const initial = harness.render(runningActivity());

  assert.ok(hasValue(initial, "false"));
  assert.equal(hasComponentNamed(initial, "ToolDetailPanel"), false);

  const updated = harness.render(
    runningActivity({
      toolStatus: "success",
      toolDetail: "gmail",
      toolParameters: '{\n  "query": "gmail"\n}',
      toolResultPreview: '{ "ok": true }',
    }),
  );

  assert.ok(hasValue(updated, "true"));
  assert.ok(hasComponentNamed(updated, "ToolDetailPanel"));
});

test("ToolActivityCard reopens a manually collapsed card when it fails", () => {
  const harness = createToolCardHarness();
  const running = runningActivity({
    toolDetail: "deployment status",
    toolParameters: "query: deployment status",
  });

  assert.ok(hasComponentNamed(harness.render(running), "ToolDetailPanel"));

  harness.collapse();
  const collapsed = harness.render(running);
  assert.ok(hasValue(collapsed, "false"));
  assert.equal(hasComponentNamed(collapsed, "ToolDetailPanel"), false);

  const failed = {
    ...running,
    toolStatus: "error",
    toolError: "request_failed",
  };
  const reopened = harness.render(failed);
  assert.ok(hasValue(reopened, "true"));
  assert.ok(hasComponentNamed(reopened, "ToolDetailPanel"));

  const declinedHarness = createToolCardHarness();
  assert.ok(hasComponentNamed(declinedHarness.render(running), "ToolDetailPanel"));

  declinedHarness.collapse();
  const collapsedDeclinedSource = declinedHarness.render(running);
  assert.ok(hasValue(collapsedDeclinedSource, "false"));
  assert.equal(hasComponentNamed(collapsedDeclinedSource, "ToolDetailPanel"), false);

  const declined = {
    ...running,
    toolStatus: "declined",
    toolError: "user_declined",
  };
  const declinedReopened = declinedHarness.render(declined);
  assert.ok(hasValue(declinedReopened, "true"));
  assert.ok(hasComponentNamed(declinedReopened, "ToolDetailPanel"));
});

function hasValue(node, expected) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return false;
  return node.values.some((value) => {
    if (value === expected) return true;
    if (Array.isArray(value)) return value.some((item) => hasValue(item, expected));
    return hasValue(value, expected);
  });
}

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
