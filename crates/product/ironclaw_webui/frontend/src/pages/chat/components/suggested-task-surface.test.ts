// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

// Same vm-harness convention: the surface's free variables are `useT`, the
// child card, the extensions catalog hook, the connect resolver, and the lazy
// setup modal — so we stub them and assert what the surface renders/forwards.
// Gating on the `oobe_suggestions` flag happens in the eager parent
// (empty-state.tsx) before this lazy chunk is even requested — see
// empty-state.test.ts — so this component is unconditional/presentational and
// always renders its content when invoked directly here.
function surfaceSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-surface.tsx", import.meta.url),
    "SuggestedTaskSurface",
  );
}

function renderSurface({
  runningId = null,
  scheduledId = null,
  connectingTask = null,
  connectedIds = [],
  onApproveTask,
  onAutomationTask,
  renderRunningIndicator,
  extensions = [],
  registry = [],
  // Default resolver stub: pretend every app resolves to a fake extension, so
  // Connect opens the modal unless a test overrides this to return null.
  resolveConnectExtension = (app) => ({ packageRef: { id: app }, displayName: app }),
} = {}) {
  // Capture the ids/values passed to each setter so we can assert the
  // optimistic "flip" requests, and control the current state so we can observe
  // the resulting render (the vm harness calls the component once, so state is
  // driven, not re-rendered). useState is called in body order:
  // runningId, scheduledId, connectingTask, connectedIds.
  const setRunningIdCalls = [];
  const setScheduledIdCalls = [];
  const setConnectingTaskCalls = [];
  const setConnectedIdsCalls = [];
  const states = [
    [runningId, (next) => setRunningIdCalls.push(next)],
    [scheduledId, (next) => setScheduledIdCalls.push(next)],
    [connectingTask, (next) => setConnectingTaskCalls.push(next)],
    [connectedIds, (next) => setConnectedIdsCalls.push(next)],
  ];
  let stateIndex = 0;
  const components = { SuggestedTaskCard() {}, ConfigureModal() {} };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
    useExtensions: () => ({ extensions, registry }),
    resolveConnectExtension,
    React: {
      useState: () => states[stateIndex++],
      // The surface wraps the setup modal in React.lazy()/React.Suspense so it
      // loads as its own chunk only when Connect is clicked; the harness never
      // resolves a lazy import, so hand back the exact stub identity that
      // findComponent()/componentProps() key off of.
      lazy: () => components.ConfigureModal,
      Suspense: ({ children }) => children,
    },
  };
  vm.runInNewContext(surfaceSourceForTest(), context);
  const props = {};
  if (onApproveTask) props.onApproveTask = onApproveTask;
  if (onAutomationTask) props.onAutomationTask = onAutomationTask;
  if (renderRunningIndicator) props.renderRunningIndicator = renderRunningIndicator;
  const tree = context.globalThis.__testExports.SuggestedTaskSurface(props);
  return {
    tree,
    components,
    setRunningIdCalls,
    setScheduledIdCalls,
    setConnectingTaskCalls,
    setConnectedIdsCalls,
  };
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

  const second = renderSurface({ runningId: props.task.id });
  const runningCard = findComponent(second.tree, second.components.SuggestedTaskCard);
  const runningProps = componentProps(runningCard, second.components.SuggestedTaskCard);
  assert.equal(runningProps.task.state, "running");
});

test("only the running card stays unlocked — every other card locks (§2A change 3)", () => {
  const idle = renderSurface();
  const idleCard = findComponent(idle.tree, idle.components.SuggestedTaskCard);
  const idleProps = componentProps(idleCard, idle.components.SuggestedTaskCard);
  assert.equal(idleProps.locked, false, "no card locks when nothing is running");
  const cardTaskId = idleProps.task.id;

  const otherRunning = renderSurface({ runningId: "a-different-task-id" });
  const otherProps = componentProps(
    findComponent(otherRunning.tree, otherRunning.components.SuggestedTaskCard),
    otherRunning.components.SuggestedTaskCard,
  );
  assert.notEqual(otherProps.task.id, "a-different-task-id");
  assert.equal(otherProps.locked, true, "every other card locks while one job runs");

  const selfRunning = renderSurface({ runningId: cardTaskId });
  const selfProps = componentProps(
    findComponent(selfRunning.tree, selfRunning.components.SuggestedTaskCard),
    selfRunning.components.SuggestedTaskCard,
  );
  assert.equal(selfProps.locked, false, "the acting card itself never locks");
});

test("surface forwards renderRunningIndicator straight through to each card", () => {
  const renderRunningIndicator = (label) => `indicator:${label}`;
  const { tree, components } = renderSurface({ renderRunningIndicator });
  const card = findComponent(tree, components.SuggestedTaskCard);
  const props = componentProps(card, components.SuggestedTaskCard);
  assert.equal(props.renderRunningIndicator, renderRunningIndicator);
});

test("clicking Connect records the clicked task so the setup modal can open (slice 3)", () => {
  // Test through the caller: the card's Connect must report the exact task
  // upward so the surface can resolve its extension and open ConfigureModal.
  const { tree, components, setConnectingTaskCalls } = renderSurface();
  const card = findComponent(tree, components.SuggestedTaskCard);
  const props = componentProps(card, components.SuggestedTaskCard);
  assert.equal(typeof props.onConnect, "function", "cards expose a Connect action");

  props.onConnect();
  assert.equal(
    setConnectingTaskCalls.length,
    1,
    "Connect requests the surface open the setup flow for that task",
  );
  assert.equal(setConnectingTaskCalls[0], props.task, "it opens for the exact clicked task");
});

test("with a connecting task that resolves, the surface mounts the real ConfigureModal for that extension", () => {
  // The one real connect path: resolve app -> catalog extension, then open the
  // EXISTING extensions setup/OAuth modal (no cloned OAuth logic).
  const connectingTask = { id: "demo-gmail-connect", app: "gmail", state: "unconnected" };
  const fakeExtension = { packageRef: { id: "gmail" }, displayName: "Gmail" };
  const { tree, components, setConnectedIdsCalls, setConnectingTaskCalls } = renderSurface({
    connectingTask,
    resolveConnectExtension: (app) => (app === "gmail" ? fakeExtension : null),
  });

  const modal = findComponent(tree, components.ConfigureModal);
  assert.ok(modal, "ConfigureModal must be mounted while a resolvable task is connecting");
  const modalProps = componentProps(modal, components.ConfigureModal);
  assert.equal(
    modalProps.extension,
    fakeExtension,
    "the modal drives the resolved real extension (packageRef + displayName)",
  );

  // onSaved flips the card unconnected -> suggested (via connectedIds) and
  // closes the modal; onClose just closes it.
  modalProps.onSaved();
  assert.equal(setConnectedIdsCalls.length, 1, "a successful connect records the connected task");
  // setConnectedIds is called with an updater; apply it to see the next state.
  // Compare element-wise (not deepEqual): the array is built inside the vm
  // realm, so its prototype differs from a test-realm array literal.
  const nextConnected = setConnectedIdsCalls[0]([]);
  assert.equal(nextConnected.length, 1);
  assert.equal(nextConnected[0], connectingTask.id, "the connected id is appended");
  assert.equal(
    setConnectingTaskCalls.at(-1),
    null,
    "a successful connect closes the modal",
  );
});

test("a connected unconnected-card flips to suggested so Approve becomes available", () => {
  // With the gmail task's id in connectedIds, its `unconnected` card renders as
  // `suggested` (Approve), even though the static demo state is `unconnected`.
  const { tree, components } = renderSurface({
    connectedIds: ["demo-gmail-connect"],
  });
  // The demo gmail card is the first card; find all cards is awkward with the
  // harness (componentProps collapses to the last match), so assert via a
  // targeted resolver: re-render with only that behavior observable is overkill.
  // Instead, assert the flip logic by checking the FIRST card in document order
  // is no longer "unconnected".
  const firstCard = findComponent(tree, components.SuggestedTaskCard);
  assert.ok(firstCard, "cards render");
  // The surface maps in order; the gmail (unconnected) card is index 0. Its
  // effective state must be "suggested" now that it's connected.
  const gmailProps = firstCardProps(tree, components.SuggestedTaskCard);
  assert.equal(gmailProps.task.id, "demo-gmail-connect");
  assert.equal(gmailProps.task.state, "suggested", "connected card flips to suggested");
});

test("Connect on an app with no catalog match shows a notice instead of an empty modal", () => {
  const connectingTask = { id: "demo-gmail-connect", app: "gmail", state: "unconnected" };
  const { tree, components } = renderSurface({
    connectingTask,
    resolveConnectExtension: () => null,
  });
  assert.equal(
    findComponent(tree, components.ConfigureModal),
    null,
    "no modal opens when the app resolves to no installable extension",
  );
  // The unavailable notice text is rendered (harness renders string children
  // inline, so assert the source produced the key somewhere in the tree).
  assert.ok(
    JSON.stringify(tree).includes("chat.oobe.connectUnavailable"),
    "an unavailable notice is shown instead of a dead button",
  );
});

test("+ Automation reports the task upward AND flips that card to scheduled", () => {
  const scheduled = [];
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

  const second = renderSurface({ scheduledId: props.task.id });
  const scheduledCard = findComponent(second.tree, second.components.SuggestedTaskCard);
  const scheduledProps = componentProps(scheduledCard, second.components.SuggestedTaskCard);
  assert.equal(scheduledProps.scheduled, true);
});

// Read the props of the FIRST occurrence of `component` in document order.
// (componentProps keys off the node it's given; findComponent returns the
// nearest node whose values contain the component, i.e. the first match in a
// depth-first walk.)
function firstCardProps(tree, component) {
  const node = findComponent(tree, component);
  return componentProps(node, component);
}
