// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const CLIP_ID = "near-clip-test";

// Load the presentational component the same way connection-status.test.ts does:
// strip imports, expose the function, and run it in a VM with a mocked React so
// `React.useId()` returns a deterministic clipPath id. The vm-tsx-setup shim
// transpiles the JSX into inspectable `{ type, props, children }` nodes.
function loadNearProcessIndicator() {
  const source = readFileSync(
    new URL("./near-process-indicator.tsx", import.meta.url),
    "utf8",
  );
  const body = source
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace(
      "export function NearProcessIndicator",
      "function NearProcessIndicator",
    );
  const context = {
    React: { useId: () => CLIP_ID },
    globalThis: {},
  };
  vm.runInNewContext(
    `${body}\nglobalThis.__testExports = { NearProcessIndicator };`,
    context,
  );
  return context.globalThis.__testExports.NearProcessIndicator;
}

function findNode(value, predicate, seen = new Set()) {
  if (!value || typeof value !== "object" || seen.has(value)) return null;
  seen.add(value);
  if (Array.isArray(value)) {
    for (const candidate of value) {
      const match = findNode(candidate, predicate, seen);
      if (match) return match;
    }
    return null;
  }
  if (predicate(value)) return value;
  for (const key of ["children", "values"]) {
    const candidates = Array.isArray(value[key]) ? value[key] : [];
    for (const candidate of candidates) {
      const match = findNode(candidate, predicate, seen);
      if (match) return match;
    }
  }
  return null;
}

const byClass = (name) => (node) => node.props?.className === name;
const includesClass = (fragment) => (node) =>
  typeof node.props?.className === "string" &&
  node.props.className.includes(fragment);

test("NearProcessIndicator working state chases the NEAR spine with elapsed time", () => {
  const NearProcessIndicator = loadNearProcessIndicator();
  const rendered = NearProcessIndicator({
    state: "working",
    label: "Working…",
    elapsed: "0:03",
  });

  assert.match(rendered.props.className, /\bnear-process\b/);
  assert.match(rendered.props.className, /\bis-busy\b/);

  // The canonical NEAR glyph is filled and dimmed while working.
  const base = findNode(rendered, byClass("near-base"));
  assert.notEqual(base, null, "base glyph should render");
  assert.ok(base.props.d.startsWith("M21.443"), "base uses the NEAR mark path");
  assert.equal(base.props.opacity, 0.24);

  // The comet only exists while working, clipped to the glyph via the useId ref.
  const comet = findNode(rendered, byClass("near-comet"));
  assert.notEqual(comet, null, "comet should render while working");
  assert.ok(comet.props.d.startsWith("M2.6 22.2"), "comet rides the N spine");
  const clipGroup = findNode(rendered, (node) => node.props?.["clip-path"]);
  assert.equal(clipGroup.props["clip-path"], `url(#${CLIP_ID})`);

  // Label is the strong text color; elapsed is shown beside it.
  const label = findNode(rendered, includesClass("text-[var(--v2-text-strong)]"));
  assert.notEqual(label, null, "working label uses strong text color");
  assert.equal(label.children[0], "Working…");
  const elapsed = findNode(rendered, (node) => node.children?.[0] === "0:03");
  assert.notEqual(elapsed, null, "elapsed should render while working");
});

test("NearProcessIndicator done state is a solid, static glyph with a muted label", () => {
  const NearProcessIndicator = loadNearProcessIndicator();
  const rendered = NearProcessIndicator({ state: "done", label: "Done" });

  assert.match(rendered.props.className, /\bis-done\b/);

  const base = findNode(rendered, byClass("near-base"));
  assert.ok(base.props.d.startsWith("M21.443"), "base uses the NEAR mark path");
  assert.equal(base.props.opacity, 1, "glyph is solid when done");

  assert.equal(
    findNode(rendered, byClass("near-comet")),
    null,
    "comet is hidden when done",
  );

  const label = findNode(rendered, includesClass("text-[var(--v2-text-muted)]"));
  assert.notEqual(label, null, "done label uses the muted text color");
  assert.equal(label.children[0], "Done");
});
