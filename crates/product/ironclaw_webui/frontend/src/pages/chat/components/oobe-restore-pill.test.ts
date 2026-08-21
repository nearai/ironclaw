// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import { componentSourceForTest } from "../../../lib/vm-component-harness";

// vm-harness: render the pill with stubbed Icon/useT and assert it shows the
// "Show suggestions" label and wires its two actions.
function pillSourceForTest() {
  return componentSourceForTest(
    new URL("./oobe-restore-pill.tsx", import.meta.url),
    "OobeRestorePill",
  );
}

function renderPill(props) {
  const context = { Icon() {}, globalThis: {}, useT: () => (key) => key };
  vm.runInNewContext(pillSourceForTest(), context);
  return context.globalThis.__testExports.OobeRestorePill(props);
}

// Collect raw <button> onClick handlers in document order.
function buttonHandlers(node, found = []) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return found;
  const strings = node.strings || [];
  node.values.forEach((v, i) => {
    if (typeof v === "function" && /onClick=\s*$/.test(strings[i] || "")) found.push(v);
    buttonHandlers(v, found);
  });
  return found;
}

test("renders the Show suggestions label", () => {
  const tree = renderPill({ onRestore: () => {}, onDismiss: () => {} });
  assert.ok(JSON.stringify(tree).includes("chat.oobe.showSuggestions"));
});

test("first button restores, second dismisses", () => {
  const events = [];
  const tree = renderPill({
    onRestore: () => events.push("restore"),
    onDismiss: () => events.push("dismiss"),
  });
  const handlers = buttonHandlers(tree);
  assert.equal(handlers.length, 2, "a restore button and a dismiss button");
  handlers[0]();
  handlers[1]();
  assert.deepEqual(events, ["restore", "dismiss"]);
});
