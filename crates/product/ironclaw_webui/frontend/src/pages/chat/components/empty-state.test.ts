// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

// Same vm-harness convention as chat-input.test.ts (shared plumbing lives in
// lib/vm-component-harness.ts): read the real source, strip imports, rename
// the export, and stub the module's free variables (including its child
// components) so the vm can evaluate it directly. This is the only way to
// observe what EmptyState actually forwards to its internal ChatInput —
// chat.test.ts stubs both components as no-ops and so cannot see prop
// plumbing between them.
function emptyStateSourceForTest() {
  return componentSourceForTest(
    new URL("./empty-state.tsx", import.meta.url),
    "EmptyState",
  );
}

function renderEmptyState({ oobeSuggestionsEnabled = true, ...props } = {}) {
  const components = {
    Icon() {},
    ChatInput() {},
    SuggestedTaskSurface() {},
    NearProcessIndicator() {},
  };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
    // EmptyState reads the `oobe_suggestions` flag itself (eagerly, before
    // deciding whether to mount the lazy surface at all) — see
    // empty-state.tsx and suggested-task-surface.test.ts, which used to stub
    // this same hook before that gating moved here.
    useOobeSuggestionsEnabled: () => oobeSuggestionsEnabled,
    // EmptyState wraps SuggestedTaskSurface in React.lazy()/React.Suspense
    // (see empty-state.tsx) so it loads as its own chunk instead of padding
    // every /chat page load. The vm harness never actually resolves a lazy
    // import, so stub React.lazy to hand back the exact same stub function
    // component identity that findComponent()/componentProps() below key
    // off of — this keeps the harness observing the real prop plumbing
    // between EmptyState and the surface, lazy-loaded or not. Mounting is
    // still conditional on the flag: with it off, EmptyState never renders
    // the Suspense/lazy subtree, so the surface is never reached.
    React: {
      lazy: () => components.SuggestedTaskSurface,
      Suspense: ({ children }) => children,
    },
  };
  vm.runInNewContext(emptyStateSourceForTest(), context);
  const tree = context.globalThis.__testExports.EmptyState({
    onSuggestion: () => {},
    onSend: () => {},
    disabled: false,
    sendDisabled: false,
    initialText: "",
    resetKey: "",
    draftKey: "draft-key",
    context: {},
    statusText: "",
    canCancel: false,
    onCancel: () => {},
    ...props,
  });
  return { tree, components };
}

test("EmptyState forwards a non-empty commands list to its composer so the menu is reachable from the landing view", () => {
  const commands = [
    { name: "status", title: "Status", description: "d", usage: "/status" },
  ];
  const { tree, components } = renderEmptyState({ commands });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.deepEqual(props.commands, commands);
});

test("EmptyState mounts the OOBE suggestion surface when the oobe_suggestions flag is on", () => {
  const { tree, components } = renderEmptyState({ oobeSuggestionsEnabled: true });

  // EmptyState now owns the feature-flag gate itself (hoisted from the lazy
  // surface so the flag check stays eager while the surface's own weight
  // stays lazy) — with the flag on, the surface must be mounted and reachable.
  assert.ok(
    findComponent(tree, components.SuggestedTaskSurface),
    "the suggestion surface must be mounted in the landing view when the flag is on",
  );
});

test("EmptyState never mounts the OOBE suggestion surface when the oobe_suggestions flag is off", () => {
  const { tree, components } = renderEmptyState({ oobeSuggestionsEnabled: false });

  // With the flag off, EmptyState must not even attempt to mount the lazy
  // Suspense/surface subtree — the landing is unchanged for real users, and
  // the lazy chunk is never requested.
  assert.equal(
    findComponent(tree, components.SuggestedTaskSurface),
    null,
    "the suggestion surface must not be reachable when the flag is off",
  );
});

test("EmptyState forwards onApproveTask to the suggestion surface so approving a card runs it", () => {
  // Test through the caller: EmptyState must hand the approve callback down to
  // the surface (which wires it onto each card), exactly as it forwards the
  // commands list to ChatInput above.
  const onApproveTask = () => {};
  const { tree, components } = renderEmptyState({ onApproveTask });

  const surface = findComponent(tree, components.SuggestedTaskSurface);
  const props = componentProps(surface, components.SuggestedTaskSurface);
  assert.equal(props.onApproveTask, onApproveTask);
});

test("EmptyState forwards onAutomationTask to the suggestion surface so scheduling a card runs it", () => {
  // Test through the caller: EmptyState must hand the automation callback down
  // to the surface (which wires it onto each completed card), exactly as it
  // forwards onApproveTask above.
  const onAutomationTask = () => {};
  const { tree, components } = renderEmptyState({ onAutomationTask });

  const surface = findComponent(tree, components.SuggestedTaskSurface);
  const props = componentProps(surface, components.SuggestedTaskSurface);
  assert.equal(props.onAutomationTask, onAutomationTask);
});

test("EmptyState supplies a renderRunningIndicator that renders NearProcessIndicator with the given label", () => {
  // NearProcessIndicator is already eager-reachable via
  // typing-indicator.tsx -> message-list.tsx -> chat.tsx, so EmptyState (also
  // eager) imports it directly and hands the surface a render prop instead of
  // letting the lazy surface/card chunk import it a second time (which would
  // force the bundler to extract it into its own standalone chunk — the
  // 0.3 KB bundle-budget regression this hoist fixes).
  const { tree, components } = renderEmptyState();

  const surface = findComponent(tree, components.SuggestedTaskSurface);
  const props = componentProps(surface, components.SuggestedTaskSurface);
  assert.equal(typeof props.renderRunningIndicator, "function");

  const rendered = props.renderRunningIndicator("Working…");
  assert.ok(
    findComponent(rendered, components.NearProcessIndicator),
    "renderRunningIndicator must render NearProcessIndicator",
  );
  const indicatorProps = componentProps(rendered, components.NearProcessIndicator);
  assert.equal(indicatorProps.state, "working");
  assert.equal(indicatorProps.label, "Working…");
});
