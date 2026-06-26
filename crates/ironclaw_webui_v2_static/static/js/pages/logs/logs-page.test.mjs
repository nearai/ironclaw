import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// Load the page source, drop its imports, and expose LogsPage so we can
// invoke it with mocked dependencies and inspect the markup it returns.
// The `html` mock captures the tagged-template `strings` (the literal
// segments, which include every static className) and `values`.
function logsPageSourceForTest() {
  const source = readFileSync(new URL("./logs-page.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { LogsPage };`;
}

function renderLogsPage(overrides = {}) {
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
  const context = {
    globalThis: {},
    React: {
      useRef: (initial) => ({ current: initial }),
      useEffect: () => {},
      useCallback: (fn) => fn,
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useT: () => (key) => key,
    useOutletContext: () => ({ isAdmin: true, threadsState: null }),
    useLogs: () => logs,
  };
  vm.runInNewContext(logsPageSourceForTest(), context);
  return context.globalThis.__testExports.LogsPage();
}

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
  const markup = renderLogsPage().strings.join("");
  // The inner output region is the actual scroll surface.
  assert.match(markup, /min-h-0 flex-1 overflow-y-auto/);
});
