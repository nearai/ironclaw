// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// Load the page source, drop its imports, and expose LogsPage so we can invoke
// it with mocked dependencies and inspect the lightweight TSX tree shape emitted
// by the VM test JSX factory.
function logsPageSourceForTest() {
  const source = readFileSync(new URL("./logs-page.tsx", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { LogsPage, LogEntry };`;
}

function createLogsPageHarness(overrides = {}) {
  const logs = {
    entries: [],
    totalCount: 0,
    paused: false,
    togglePause: () => {},
    clearEntries: () => {},
    levelFilter: "all",
    setLevelFilter: () => {},
    targetFilter: "",
    setTargetFilter: () => {},
    autoScroll: true,
    setAutoScroll: () => {},
    serverLevel: null,
    changeServerLevel: () => {},
    scope: { active: [] },
    isLoading: false,
    error: null,
    needsThreadScope: false,
    ...overrides,
  };
  const hookValues = [];
  let hookCursor = 0;
  function ConfirmDialog() {}
  const context = {
    ConfirmDialog,
    globalThis: {},
    React: {
      useRef: (initial) => {
        const index = hookCursor;
        hookCursor += 1;
        if (!(index in hookValues)) hookValues[index] = { current: initial };
        return hookValues[index];
      },
      useEffect: () => {},
      useCallback: (fn) => fn,
      useState: (initial) => {
        const index = hookCursor;
        hookCursor += 1;
        if (!(index in hookValues)) {
          hookValues[index] = typeof initial === "function" ? initial() : initial;
        }
        return [
          hookValues[index],
          (next) => {
            hookValues[index] =
              typeof next === "function" ? next(hookValues[index]) : next;
          },
        ];
      },
    },
    useT: () => (key) => key,
    useOutletContext: () => ({ isAdmin: true, threadsState: null }),
    useLogs: () => logs,
  };
  vm.runInNewContext(logsPageSourceForTest(), context);
  return {
    ConfirmDialog,
    render() {
      hookCursor = 0;
      return context.globalThis.__testExports.LogsPage();
    },
  };
}

function renderLogsPage(overrides = {}) {
  return createLogsPageHarness(overrides).render();
}

// Render a single LogEntry with a mocked React context. Returns the captured
// TSX node, the `setExpanded` spy, and the row's onClick handler. `expanded`
// seeds only the *first* useState call (the expansion state) and later useState
// calls fall back to a React-like initializer, so the harness stays correct if
// LogEntry adds more state. `window` injects a stub so the onClick selection
// guard can be exercised (the vm sandbox has no `window` otherwise, which is
// the production-safe SSR/no-window path).
function renderLogEntry(entry, { expanded = false, window: windowStub } = {}) {
  const setExpanded = (() => {
    const fn = (arg) => {
      fn.calls.push(arg);
    };
    fn.calls = [];
    return fn;
  })();
  let firstUseState = true;
  const context = {
    globalThis: {},
    React: {
      useRef: (initial) => ({ current: initial }),
      useEffect: () => {},
      useCallback: (fn) => fn,
      useState: (initial) => {
        if (firstUseState) {
          firstUseState = false;
          return [expanded, setExpanded];
        }
        return [typeof initial === "function" ? initial() : initial, () => {}];
      },
    },
    useT: () => (key) => key,
    useOutletContext: () => ({ isAdmin: true, threadsState: null }),
    useLogs: () => ({}),
  };
  if (windowStub) context.window = windowStub;
  vm.runInNewContext(logsPageSourceForTest(), context);
  const node = context.globalThis.__testExports.LogEntry({ entry });
  return { node, setExpanded, onClick: rowClickHandler(node) };
}

// The row's onClick is the only function interpolated into the LogEntry markup
// (the collapsed row renders no other handlers), so find it by type.
function rowClickHandler(node) {
  const handler = firstValueMatching(node, (value) => typeof value === "function");
  assert.ok(handler, "expected an onClick handler on the log entry row");
  return handler;
}

function firstValueMatching(node, predicate) {
  if (Array.isArray(node)) {
    for (const item of node) {
      const found = firstValueMatching(item, predicate);
      if (found) return found;
    }
    return undefined;
  }
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return undefined;
  for (const value of node.values) {
    if (predicate(value)) return value;
    const found = firstValueMatching(value, predicate);
    if (found) return found;
  }
  return undefined;
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  visit(node.values, fn);
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

function nativeNodeByText(root, tagName, text) {
  let match;
  visit(root, (node) => {
    if (!match && node.type === tagName && flattenMarkup(node).includes(text)) {
      match = node;
    }
  });
  return match;
}

// Build a window stub whose getSelection() reports a selection rooted at the
// given node (or no selection when `node` is null/omitted).
function selectionWindow(node) {
  return {
    getSelection: () =>
      node
        ? { isCollapsed: false, anchorNode: node, focusNode: node, toString: () => "selected" }
        : { isCollapsed: true, anchorNode: null, focusNode: null, toString: () => "" },
  };
}

// Flatten a captured TSX node into the concatenation of every literal segment
// and every interpolated string value, recursing into nested nodes.
// This surfaces className strings that are built via `[...].join(" ")` (passed
// as interpolated values, not literal segments).
function flattenMarkup(node) {
  if (node == null) return "";
  if (typeof node === "string") return node;
  if (Array.isArray(node)) return node.map(flattenMarkup).join(" ");
  if (typeof node === "object" && Array.isArray(node.strings)) {
    return [...node.strings, ...node.values.map(flattenMarkup)].join(" ");
  }
  return "";
}

const SAMPLE_ENTRY = {
  timestamp: "2026-06-29T12:00:00.000Z",
  level: "info",
  target: "ironclaw::agent",
  message: "selectable log message body",
  threadId: "thread-1",
};

// A log line's text must be user-selectable so it can be copied. The clickable
// row previously carried Tailwind's `select-none` (→ user-select: none), which
// blocked selecting the timestamp/level/target/message while still leaving the
// expanded context chips (a sibling node) copyable — exactly the reported bug.
test("LogEntry row text is selectable (no select-none)", () => {
  const markup = flattenMarkup(renderLogEntry(SAMPLE_ENTRY).node);
  assert.ok(
    !/\bselect-none\b/.test(markup),
    "log entry row must not use select-none, or its text cannot be copied",
  );
  assert.match(markup, /\bselect-text\b/, "log entry row should opt into select-text");
});

// The onClick guard must toggle on a plain click but suppress the toggle only
// when the click ends a text selection *inside this row* — a selection
// elsewhere on the page (the document-global getSelection() trap) must not
// block the toggle.
test("LogEntry row toggles on a plain click with no selection", () => {
  const { onClick, setExpanded } = renderLogEntry(SAMPLE_ENTRY, { window: selectionWindow(null) });
  onClick({ currentTarget: { contains: () => false } });
  assert.equal(setExpanded.calls.length, 1, "a plain click should toggle the row");
});

test("LogEntry row does not toggle when a selection ends inside the row", () => {
  const rowNode = { id: "in-row" };
  const { onClick, setExpanded } = renderLogEntry(SAMPLE_ENTRY, {
    window: selectionWindow(rowNode),
  });
  onClick({ currentTarget: { contains: (n) => n === rowNode } });
  assert.equal(setExpanded.calls.length, 0, "selecting text in the row must not toggle it");
});

test("LogEntry row still toggles when the selection is in another element", () => {
  const elsewhere = { id: "elsewhere" };
  const { onClick, setExpanded } = renderLogEntry(SAMPLE_ENTRY, {
    window: selectionWindow(elsewhere),
  });
  // currentTarget is this row; the selection lives elsewhere, so it must not block.
  onClick({ currentTarget: { contains: () => false } });
  assert.equal(setExpanded.calls.length, 1, "a selection in another element must not block the toggle");
});

// Class tokens on the root opening tag (the first `<div className="...">` the
// page renders). Asserting on this tag's token set — rather than an exact
// class string — keeps the test robust to class reordering and to unrelated
// class additions.
function rootClassTokens(markup) {
  const match = markup.match(/className="([^"]*)"/);
  assert.ok(match, "expected the root element to carry a className");
  return new Set(match[1].split(/\s+/).filter(Boolean));
}

// The page's root element must fill its parent <main>, which is a *block*
// element (no `display:flex`). `flex-1` only resolves against a flex parent,
// so a `flex-1` root would collapse to content height, overflow <main>, get
// clipped by its `overflow-hidden`, and leave the inner scroll container with
// no bounded height — i.e. no scrollbar. `h-full` is what fills the block
// parent (matching the jobs/missions/routines pages). Regression: #5278.
test("LogsPage root fills its parent so the log list can scroll", () => {
  const tokens = rootClassTokens(renderLogsPage().strings.join(""));

  // Root fills <main> via h-full...
  assert.ok(tokens.has("h-full"), "root must use h-full to fill its block parent");
  // ...and must not rely on flex-1, which is a no-op under the block <main>
  // and reintroduces the unscrollable, clipped page (regardless of class order).
  assert.ok(!tokens.has("flex-1"), "root must not use flex-1 for height");
});

test("LogsPage keeps the scrollable log output container", () => {
  const markup = flattenMarkup(renderLogsPage());
  // The inner output region is the actual scroll surface.
  assert.match(markup, /min-h-0 flex-1 overflow-y-auto/);
});

test("LogsPage clears entries only after confirming the shared dialog", () => {
  let clearCount = 0;
  const harness = createLogsPageHarness({
    clearEntries: () => {
      clearCount += 1;
    },
  });

  let rendered = harness.render();
  const clearButton = nativeNodeByText(rendered, "button", "logs.clear");
  assert.ok(clearButton, "expected the clear logs button");
  clearButton.props.onClick();
  assert.equal(clearCount, 0);

  rendered = harness.render();
  let [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, true);
  assert.equal(dialog.title, "logs.confirmClear");

  dialog.onCancel();
  rendered = harness.render();
  [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, false);
  assert.equal(clearCount, 0);

  nativeNodeByText(rendered, "button", "logs.clear").props.onClick();
  rendered = harness.render();
  [dialog] = componentProps(rendered, harness.ConfirmDialog);
  dialog.onConfirm();
  assert.equal(clearCount, 1);
});
