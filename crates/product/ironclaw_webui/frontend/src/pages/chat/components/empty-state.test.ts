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

function renderEmptyState({
  oobeSuggestionsEnabled = true,
  drawerState = "open",
  ...props
} = {}) {
  const components = {
    Icon() {},
    ChatInput() {},
    SuggestedTaskSurface() {},
    OobeRestorePill() {},
    NearProcessIndicator() {},
  };
  // Two module-scope React.lazy() calls, in source order: the surface, then the
  // restore pill. Hand each its own stub identity so findComponent can tell
  // them apart.
  const lazyComponents = [components.SuggestedTaskSurface, components.OobeRestorePill];
  let lazyIndex = 0;
  const requestedKeys = [];
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => {
      requestedKeys.push(key);
      return key;
    },
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
      lazy: () => lazyComponents[lazyIndex++] ?? components.SuggestedTaskSurface,
      Suspense: ({ children }) => children,
      // EmptyState owns the drawer-visibility state (open/dismissed/gone). Drive
      // it from the test arg so we can observe what it forwards to the surface
      // and whether the restore pill renders.
      useState: () => [drawerState, () => {}],
    },
  };
  vm.runInNewContext(emptyStateSourceForTest(), context);
  const tree = context.globalThis.__testExports.EmptyState({
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
  return { tree, components, requestedKeys };
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

test("EmptyState no longer renders the static below-composer suggestion list", () => {
  // The three hardcoded "Map the current gateway state" / "Review recent
  // thread activity" / "Draft an extension readiness check" rows are
  // deprecated (superseded by the backend-driven OOBE suggestion surface
  // above the composer) — pin that the retired copy keys are never
  // requested and the component no longer needs an onSuggestion callback.
  const { requestedKeys } = renderEmptyState();

  for (const key of [
    "chat.suggestion1",
    "chat.suggestion1Desc",
    "chat.suggestion2",
    "chat.suggestion2Desc",
    "chat.suggestion3",
    "chat.suggestion3Desc",
  ]) {
    assert.ok(
      !requestedKeys.includes(key),
      `retired suggestion key ${key} must not be requested`,
    );
  }
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

test("EmptyState forwards onOpenThread to the suggestion surface so a started card can navigate", () => {
  // Test through the caller: EmptyState must hand the navigation callback down
  // to the surface (which wires it onto each card), exactly as it forwards the
  // commands list to ChatInput above. Starting a suggestion is a server-side
  // operation that returns a thread binding; this is how the browser is told
  // where to go.
  const onOpenThread = () => {};
  const { tree, components } = renderEmptyState({ onOpenThread });

  const surface = findComponent(tree, components.SuggestedTaskSurface);
  const props = componentProps(surface, components.SuggestedTaskSurface);
  assert.equal(props.onOpenThread, onOpenThread);
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

test("with the drawer open, EmptyState mounts the surface unhidden and shows no restore pill", () => {
  const { tree, components } = renderEmptyState({ drawerState: "open" });
  const surface = findComponent(tree, components.SuggestedTaskSurface);
  assert.ok(surface, "surface mounts when open");
  assert.equal(
    componentProps(surface, components.SuggestedTaskSurface).hidden,
    false,
    "surface is not hidden while the drawer is open",
  );
  assert.equal(
    findComponent(tree, components.OobeRestorePill),
    null,
    "no restore pill while the drawer is open",
  );
});

test("with the drawer dismissed, EmptyState hides the surface and shows the restore pill", () => {
  // Section-dismiss: the surface is told to hide, and the in-composer
  // "Show suggestions" pill appears so the user can bring it back.
  const { tree, components } = renderEmptyState({ drawerState: "dismissed" });
  const surface = findComponent(tree, components.SuggestedTaskSurface);
  assert.equal(
    componentProps(surface, components.SuggestedTaskSurface).hidden,
    true,
    "surface is hidden once the drawer is dismissed",
  );
  const pill = findComponent(tree, components.OobeRestorePill);
  assert.ok(pill, "the restore pill is mounted when dismissed");
  const pillProps = componentProps(pill, components.OobeRestorePill);
  assert.equal(typeof pillProps.onRestore, "function", "pill can reopen the drawer");
  assert.equal(typeof pillProps.onDismiss, "function", "pill can dismiss fully");
});

test("EmptyState hands the surface an onClose that dismisses the drawer section", () => {
  const { tree, components } = renderEmptyState({ drawerState: "open" });
  const surface = findComponent(tree, components.SuggestedTaskSurface);
  const props = componentProps(surface, components.SuggestedTaskSurface);
  assert.equal(typeof props.onClose, "function", "surface receives a section-close callback");
});
