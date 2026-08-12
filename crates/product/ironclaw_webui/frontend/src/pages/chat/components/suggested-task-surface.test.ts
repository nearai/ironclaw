// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

// Same vm-harness convention: the surface's only free variables are the feature
// hook, `useT`, and the child card, so we stub the hook to force the gate on/off
// and assert what the surface renders. This is the flag-gating regression: the
// surface must be inert (render null) for real users until `oobe_suggestions`.
function surfaceSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-surface.tsx", import.meta.url),
    "SuggestedTaskSurface",
  );
}

function renderSurface({ enabled, runningId = null, onApproveTask } = {}) {
  // Capture the id passed to `setRunningId` so we can assert the optimistic
  // "flip to running" request, and control the current `runningId` so we can
  // observe the resulting render (the vm harness calls the component once, so
  // state is driven, not re-rendered).
  const setRunningIdCalls = [];
  const components = { SuggestedTaskCard() {} };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
    useOobeSuggestionsEnabled: () => enabled,
    React: {
      useState: () => [runningId, (next) => setRunningIdCalls.push(next)],
    },
  };
  vm.runInNewContext(surfaceSourceForTest(), context);
  const tree = context.globalThis.__testExports.SuggestedTaskSurface(
    onApproveTask ? { onApproveTask } : {},
  );
  return { tree, components, setRunningIdCalls };
}

test("surface renders nothing when the oobe_suggestions flag is off", () => {
  const { tree } = renderSurface({ enabled: false });
  assert.equal(tree, null, "the landing must be unchanged for real users when the flag is off");
});

test("surface renders the demo cards when the flag is on", () => {
  const { tree, components } = renderSurface({ enabled: true });
  assert.notEqual(tree, null);
  assert.ok(
    findComponent(tree, components.SuggestedTaskCard),
    "the flag-on surface should mount SuggestedTaskCard(s)",
  );
});

test("approving a card reports the task upward AND flips that card to running", () => {
  const approved = [];
  // Pass 1: no card is running yet. Invoking a card's Approve must both report
  // the exact task to `onApproveTask` (so the parent runs it through the send
  // path) and request the optimistic flip via setRunningId(task.id).
  const first = renderSurface({
    enabled: true,
    onApproveTask: (task) => approved.push(task),
  });
  const card = findComponent(first.tree, first.components.SuggestedTaskCard);
  const props = componentProps(card, first.components.SuggestedTaskCard);
  assert.notEqual(props.task.state, "running", "card is not running before approval");

  props.onApprove();
  assert.deepEqual(approved, [props.task], "approving reports the exact task");
  assert.deepEqual(
    first.setRunningIdCalls,
    [props.task.id],
    "approving requests the optimistic flip for that card",
  );

  // Pass 2: with that id marked running, the surface renders THAT card in the
  // `running` state (optimistic feedback) — activity streams in the thread.
  const second = renderSurface({ enabled: true, runningId: props.task.id });
  const runningCard = findComponent(second.tree, second.components.SuggestedTaskCard);
  const runningProps = componentProps(runningCard, second.components.SuggestedTaskCard);
  assert.equal(runningProps.task.state, "running");
});
