// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

// Same vm-harness convention: the surface's only free variables are `useT` and
// the child card, so we stub them and assert what the surface renders. Gating
// on the `oobe_suggestions` flag now happens in the eager parent
// (empty-state.tsx) before this lazy chunk is even requested — see
// empty-state.test.ts for that coverage — so this component is unconditional/
// presentational and always renders its content when invoked directly here.
function surfaceSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-surface.tsx", import.meta.url),
    "SuggestedTaskSurface",
  );
}

function renderSurface({
  runningId = null,
  scheduledId = null,
  onApproveTask,
  onAutomationTask,
  renderRunningIndicator,
} = {}) {
  // Capture the ids passed to `setRunningId` / `setScheduledId` so we can assert
  // the optimistic "flip" requests, and control the current `runningId` /
  // `scheduledId` so we can observe the resulting render (the vm harness calls
  // the component once, so state is driven, not re-rendered). The surface calls
  // `useState` twice in order — runningId then scheduledId — so hand back the
  // matching tuple per call index.
  const setRunningIdCalls = [];
  const setScheduledIdCalls = [];
  const states = [
    [runningId, (next) => setRunningIdCalls.push(next)],
    [scheduledId, (next) => setScheduledIdCalls.push(next)],
  ];
  let stateIndex = 0;
  const components = { SuggestedTaskCard() {} };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
    React: {
      useState: () => states[stateIndex++],
    },
  };
  vm.runInNewContext(surfaceSourceForTest(), context);
  const props = {};
  if (onApproveTask) props.onApproveTask = onApproveTask;
  if (onAutomationTask) props.onAutomationTask = onAutomationTask;
  if (renderRunningIndicator) props.renderRunningIndicator = renderRunningIndicator;
  const tree = context.globalThis.__testExports.SuggestedTaskSurface(props);
  return { tree, components, setRunningIdCalls, setScheduledIdCalls };
}

test("surface renders the demo cards", () => {
  const { tree, components } = renderSurface();
  assert.notEqual(tree, null);
  assert.ok(
    findComponent(tree, components.SuggestedTaskCard),
    "the surface should mount SuggestedTaskCard(s)",
  );
});

test("approving a card reports the task upward AND flips that card to running", () => {
  const approved = [];
  // Pass 1: no card is running yet. Invoking a card's Approve must both report
  // the exact task to `onApproveTask` (so the parent runs it through the send
  // path) and request the optimistic flip via setRunningId(task.id).
  const first = renderSurface({
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
  const second = renderSurface({ runningId: props.task.id });
  const runningCard = findComponent(second.tree, second.components.SuggestedTaskCard);
  const runningProps = componentProps(runningCard, second.components.SuggestedTaskCard);
  assert.equal(runningProps.task.state, "running");
});

test("only the running card stays unlocked — every other card locks (§2A change 3)", () => {
  // No job running yet: nothing is locked.
  const idle = renderSurface();
  const idleCard = findComponent(idle.tree, idle.components.SuggestedTaskCard);
  const idleProps = componentProps(idleCard, idle.components.SuggestedTaskCard);
  assert.equal(idleProps.locked, false, "no card locks when nothing is running");
  const cardTaskId = idleProps.task.id;

  // A DIFFERENT card is running: this card must lock (disable connect/approve/
  // automation — no queuing a second job while one is active).
  const otherRunning = renderSurface({ runningId: "a-different-task-id" });
  const otherProps = componentProps(
    findComponent(otherRunning.tree, otherRunning.components.SuggestedTaskCard),
    otherRunning.components.SuggestedTaskCard,
  );
  assert.notEqual(otherProps.task.id, "a-different-task-id");
  assert.equal(otherProps.locked, true, "every other card locks while one job runs");

  // THIS card is the one running: it must stay unlocked so its own
  // running/completed state remains interactive (e.g. still dismissible).
  const selfRunning = renderSurface({ runningId: cardTaskId });
  const selfProps = componentProps(
    findComponent(selfRunning.tree, selfRunning.components.SuggestedTaskCard),
    selfRunning.components.SuggestedTaskCard,
  );
  assert.equal(selfProps.locked, false, "the acting card itself never locks");
});

test("surface forwards renderRunningIndicator straight through to each card", () => {
  // Test through the caller: the surface itself has no opinion on how the
  // running state renders — it just threads the render prop from its own
  // caller (empty-state.tsx) down to SuggestedTaskCard, which is what
  // actually decides what to render for the "running" state.
  const renderRunningIndicator = (label) => `indicator:${label}`;
  const { tree, components } = renderSurface({ renderRunningIndicator });

  const card = findComponent(tree, components.SuggestedTaskCard);
  const props = componentProps(card, components.SuggestedTaskCard);
  assert.equal(props.renderRunningIndicator, renderRunningIndicator);
});

test("+ Automation reports the task upward AND flips that card to scheduled", () => {
  const scheduled = [];
  // Pass 1: no card is scheduled yet. Invoking a card's automation action must
  // both report the exact task to `onAutomationTask` (so the parent submits its
  // `automationPrompt` through the send path) and request the optimistic flip
  // via setScheduledId(task.id).
  const first = renderSurface({
    onAutomationTask: (task) => scheduled.push(task),
  });
  const card = findComponent(first.tree, first.components.SuggestedTaskCard);
  const props = componentProps(card, first.components.SuggestedTaskCard);
  assert.notEqual(props.scheduled, true, "card is not scheduled before automation");

  props.onAutomation();
  assert.deepEqual(scheduled, [props.task], "automation reports the exact task");
  assert.deepEqual(
    first.setScheduledIdCalls,
    [props.task.id],
    "automation requests the optimistic scheduled flip for that card",
  );

  // Pass 2: with that id marked scheduled, the surface passes `scheduled` to
  // THAT card so it swaps the button for the "Automation scheduled" chip.
  const second = renderSurface({ scheduledId: props.task.id });
  const scheduledCard = findComponent(second.tree, second.components.SuggestedTaskCard);
  const scheduledProps = componentProps(scheduledCard, second.components.SuggestedTaskCard);
  assert.equal(scheduledProps.scheduled, true);
});
