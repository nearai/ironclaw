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

// The page's root element must fill its parent <main>, which is a *block*
// element (no `display:flex`). `flex-1` only resolves against a flex parent,
// so a `flex-1` root would collapse to content height, overflow <main>, get
// clipped by its `overflow-hidden`, and leave the inner scroll container with
// no bounded height — i.e. no scrollbar. `h-full` is what fills the block
// parent (matching the jobs/missions/routines pages). Regression: #5278.
test("LogsPage root fills its parent so the log list can scroll", () => {
  const markup = renderLogsPage().strings.join("");

  // Root fills <main> via h-full, not flex-1.
  assert.match(markup, /flex h-full min-h-0 flex-col overflow-hidden/);
  // The broken `flex-1` root (which produced an unscrollable, clipped page)
  // must not come back.
  assert.doesNotMatch(markup, /flex min-h-0 flex-1 flex-col overflow-hidden/);
});

test("LogsPage keeps the scrollable log output container", () => {
  const markup = renderLogsPage().strings.join("");
  // The inner output region is the actual scroll surface.
  assert.match(markup, /min-h-0 flex-1 overflow-y-auto/);
});
