// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import { componentSourceForTest, findComponent } from "../../../lib/vm-component-harness";

// Same vm-harness convention as empty-state.test.ts / chat-input.test.ts: read
// the real source, strip imports, rename the export, and stub the module's free
// variables (Button/Icon/NearProcessIndicator and the app-meta helpers) so the
// synthetic-JSX tree can be walked directly. `useT` returns the key, so we
// assert on the i18n KEY each state renders — exactly what the card wires up.
function cardSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-card.tsx", import.meta.url),
    "SuggestedTaskCard",
  );
}

function renderCard(task, props = {}) {
  const components = {
    Button() {},
    Icon() {},
    NearProcessIndicator() {},
  };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
    appMeta: () => ({
      icon: "spark",
      labelKey: "chat.oobe.app.demo",
      accent: "#000",
    }),
    appChipStyle: () => ({
      color: "#000",
      background: "transparent",
      borderColor: "transparent",
    }),
  };
  vm.runInNewContext(cardSourceForTest(), context);
  const tree = context.globalThis.__testExports.SuggestedTaskCard({
    task,
    onConnect: () => {},
    onApprove: () => {},
    onAutomation: () => {},
    onDismiss: () => {},
    ...props,
  });
  return { tree, components };
}

// Walk the synthetic tree for a scalar (string/number/bool) value anywhere.
function containsScalar(value, expected, seen = new Set()) {
  if (value === expected) return true;
  if (!value || typeof value !== "object" || seen.has(value)) return false;
  seen.add(value);
  return Object.values(value).some((child) =>
    Array.isArray(child)
      ? child.some((item) => containsScalar(item, expected, seen))
      : containsScalar(child, expected, seen),
  );
}

const baseTask = {
  id: "t1",
  app: "gmail",
  title: "Triage your inbox",
  summary: "Reply to routine mail and archive newsletters.",
};

test("unconnected card shows a Connect action and no Approve", () => {
  const { tree } = renderCard({ ...baseTask, state: "unconnected", connectLabel: "Gmail" });
  assert.equal(containsScalar(tree, "chat.oobe.action.connect"), true);
  assert.equal(containsScalar(tree, "chat.oobe.action.approve"), false);
});

test("suggested card shows Approve and never a Modify action (§2A: no Modify)", () => {
  const { tree } = renderCard({ ...baseTask, state: "suggested" });
  assert.equal(containsScalar(tree, "chat.oobe.action.approve"), true);
  assert.equal(containsScalar(tree, "chat.oobe.action.modify"), false);
  assert.equal(containsScalar(tree, "chat.oobe.action.connect"), false);
});

test("running card shows the branded processing indicator and no buttons", () => {
  const { tree, components } = renderCard({ ...baseTask, state: "running" });
  assert.ok(
    findComponent(tree, components.NearProcessIndicator),
    "running state reuses NearProcessIndicator",
  );
  assert.equal(containsScalar(tree, "chat.oobe.status.running"), true);
  // No action buttons while a run is live — activity lives in the thread.
  assert.equal(containsScalar(tree, "chat.oobe.action.approve"), false);
  assert.equal(containsScalar(tree, "chat.oobe.action.automation"), false);
  assert.equal(containsScalar(tree, "chat.oobe.action.connect"), false);
});

test("completed card shows a Completed chip + '+ Automation' and no Revert/Modify (§2A)", () => {
  const { tree } = renderCard({ ...baseTask, state: "completed" });
  assert.equal(containsScalar(tree, "chat.oobe.status.completed"), true);
  assert.equal(containsScalar(tree, "chat.oobe.action.automation"), true);
  assert.equal(containsScalar(tree, "chat.oobe.action.revert"), false);
  assert.equal(containsScalar(tree, "chat.oobe.action.modify"), false);
  assert.equal(containsScalar(tree, "chat.oobe.action.approve"), false);
});

test("failed card shows a 'Couldn't complete' chip + Try again", () => {
  const { tree } = renderCard({ ...baseTask, state: "failed" });
  assert.equal(containsScalar(tree, "chat.oobe.status.failed"), true);
  assert.equal(containsScalar(tree, "chat.oobe.action.tryAgain"), true);
});

test("every state offers a dismiss affordance", () => {
  for (const state of ["unconnected", "suggested", "running", "completed", "failed"]) {
    const { tree } = renderCard({ ...baseTask, state });
    assert.equal(
      containsScalar(tree, "chat.oobe.dismiss"),
      true,
      `${state} card should render the dismiss affordance`,
    );
  }
});

test("locked card is visually disabled (opacity + pointer-events-none)", () => {
  const locked = renderCard({ ...baseTask, state: "suggested" }, { locked: true });
  assert.match(locked.tree.props.className, /pointer-events-none/);
  assert.match(locked.tree.props.className, /opacity-50/);
  assert.equal(locked.tree.props["aria-disabled"], true);

  const unlocked = renderCard({ ...baseTask, state: "suggested" });
  assert.doesNotMatch(unlocked.tree.props.className, /pointer-events-none/);
});
