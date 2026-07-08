import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// Load the pure helpers out of the component module without executing its
// imports (html/hooks/design-system are browser-side). Mirrors the harness in
// provider-components.test.mjs: strip imports, de-export, expose test exports.
function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

const context = { html: () => null };
context.globalThis = context;
vm.createContext(context);
vm.runInContext(
  sourceForTest("./trace-commons-tab.js", [
    "tracesSectionMode",
    "formatCredit",
    "formatTimestamp",
  ]),
  context
);
const { tracesSectionMode, formatCredit, formatTimestamp } = context.__testExports;

const TRACE = { submission_id: "s1", status: "accepted" };

test("traces load error always surfaces, never hides behind an empty state", () => {
  assert.equal(
    tracesSectionMode({ isError: true, enrolled: true, traces: [TRACE] }),
    "error"
  );
  // Error wins even for unenrolled/empty views — a backend failure must not
  // render as "no traces".
  assert.equal(
    tracesSectionMode({ isError: true, enrolled: false, traces: [] }),
    "error"
  );
});

test("enrolled contributor with traces renders the trace list", () => {
  assert.equal(
    tracesSectionMode({ isError: false, enrolled: true, traces: [TRACE] }),
    "list"
  );
});

test("section hides for empty or unenrolled states", () => {
  assert.equal(
    tracesSectionMode({ isError: false, enrolled: true, traces: [] }),
    "hidden"
  );
  assert.equal(
    tracesSectionMode({ isError: false, enrolled: false, traces: [TRACE] }),
    "hidden"
  );
  assert.equal(
    tracesSectionMode({ isError: false, enrolled: false, traces: undefined }),
    "hidden"
  );
});

test("credit and timestamp formatting used by trace rows", () => {
  assert.equal(formatCredit(1), "1.00");
  assert.equal(formatCredit("2.5"), "2.50");
  assert.equal(formatCredit(null), "0.00");
  assert.equal(formatCredit("not-a-number"), "0.00");

  const t = (key) => key;
  assert.equal(formatTimestamp(null, t), "traceCommons.never");
  assert.equal(formatTimestamp("garbage", t), "traceCommons.never");
  assert.notEqual(formatTimestamp("2026-06-25T00:00:00Z", t), "traceCommons.never");
});
