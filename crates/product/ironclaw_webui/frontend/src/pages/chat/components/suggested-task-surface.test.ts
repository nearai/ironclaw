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
// child card, Button/Icon, and the `useSuggestions` data hook — so we stub the
// hook and assert what the surface renders for each backend generation status.
// Feature-flag gating happens in the eager parent (empty-state.tsx) before this
// lazy chunk is requested — see empty-state.test.ts — so the surface itself is
// unconditional.
function surfaceSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-surface.tsx", import.meta.url),
    "SuggestedTaskSurface",
  );
}

const CARD = {
  id: "sug-1",
  title: "Triage your inbox",
  description: "Reply to routine mail.",
  suggested_prompt: "Triage my inbox.",
};

function renderSurface({
  isLoading = false,
  status = "ready",
  suggestions = [CARD],
  isGenerating = false,
  startingId = null,
  onOpenThread,
  renderRunningIndicator,
  hidden = false,
  onClose,
} = {}) {
  const generateCalls = [];
  const startCalls = [];
  const dismissCalls = [];
  const components = { SuggestedTaskCard() {}, Button() {}, Icon() {} };
  const context = {
    ...components,
    globalThis: {},
    React: {},
    useT: () => (key) => key,
    useSuggestions: () => ({
      isLoading,
      status,
      suggestions,
      isGenerating,
      startingId,
      generate: () => generateCalls.push(true),
      start: (id, options) => startCalls.push({ id, options }),
      dismiss: (id) => dismissCalls.push(id),
    }),
  };
  vm.runInNewContext(surfaceSourceForTest(), context);
  const props = { hidden };
  if (onOpenThread) props.onOpenThread = onOpenThread;
  if (renderRunningIndicator) props.renderRunningIndicator = renderRunningIndicator;
  if (onClose) props.onClose = onClose;
  const tree = context.globalThis.__testExports.SuggestedTaskSurface(props);
  return { tree, components, generateCalls, startCalls, dismissCalls };
}

test("renders nothing when the drawer is section-hidden", () => {
  // The parent (empty-state) dismisses the whole drawer and shows a restore
  // pill; the surface must render nothing in that state.
  const { tree } = renderSurface({ hidden: true });
  assert.equal(tree, null);
});

test("a framed drawer exposes a section-close control wired to onClose", () => {
  const closes = [];
  const { tree, components } = renderSurface({ onClose: () => closes.push(true) });
  // The close control carries the hide-suggestions label.
  assert.ok(
    JSON.stringify(tree).includes("chat.oobe.hideSuggestions"),
    "the drawer header renders a section-close control",
  );
  // Find the raw <button> whose onClick is the close handler and invoke it.
  const handlers = [];
  (function walk(node) {
    if (!node || typeof node !== "object" || !Array.isArray(node.values)) return;
    const strings = node.strings || [];
    node.values.forEach((v, i) => {
      if (typeof v === "function" && /onClick=\s*$/.test(strings[i] || "")) handlers.push(v);
      walk(v);
    });
  })(tree);
  handlers.forEach((h) => h());
  assert.ok(closes.length >= 1, "invoking the header control calls onClose");
  // Guard: components stub keeps findComponent usable elsewhere.
  assert.ok(components.SuggestedTaskCard);
});

test("renders nothing until the first read resolves", () => {
  // Offering a "generate" CTA over a set that may already exist would be a
  // lie, so the surface stays silent while loading.
  const { tree } = renderSurface({ isLoading: true });
  assert.equal(tree, null);
});

test("renders a card per backend suggestion", () => {
  const { tree, components } = renderSurface();
  const card = findComponent(tree, components.SuggestedTaskCard);
  assert.ok(card, "the surface mounts SuggestedTaskCard(s)");
  assert.equal(
    componentProps(card, components.SuggestedTaskCard).suggestion,
    CARD,
    "the card is driven by the backend suggestion object itself",
  );
});

test("empty status offers a generate CTA and never auto-generates", () => {
  // Generation costs a real model run, so it must be user-initiated —
  // rendering the surface must not fire it.
  const { tree, components, generateCalls } = renderSurface({
    status: "empty",
    suggestions: [],
  });
  assert.deepEqual(generateCalls, [], "rendering does not generate");
  assert.ok(JSON.stringify(tree).includes("chat.oobe.action.generate"));

  componentProps(findComponent(tree, components.Button), components.Button).onClick();
  assert.deepEqual(generateCalls, [true], "the CTA generates on click");
});

test("generating status shows the anticipatory indicator (V3)", () => {
  const { tree, components } = renderSurface({
    status: "generating",
    suggestions: [],
    isGenerating: true,
    renderRunningIndicator: (label) => `indicator:${label}`,
  });
  assert.ok(JSON.stringify(tree).includes("indicator:chat.oobe.status.generating"));
  assert.equal(
    findComponent(tree, components.SuggestedTaskCard),
    null,
    "no cards render while generating an empty set",
  );
});

test("an existing set survives a regeneration rather than blanking", () => {
  // Cards win over a transient generating status: replacing a set the user is
  // reading with a spinner would blank the surface mid-use.
  const { tree, components } = renderSurface({
    status: "generating",
    isGenerating: true,
    suggestions: [CARD],
  });
  assert.ok(
    findComponent(tree, components.SuggestedTaskCard),
    "existing cards keep rendering during regeneration",
  );
});

test("failed status offers a retry", () => {
  const { tree, components, generateCalls } = renderSurface({
    status: "failed",
    suggestions: [],
  });
  const serialized = JSON.stringify(tree);
  assert.ok(serialized.includes("chat.oobe.status.generateFailed"));
  assert.ok(serialized.includes("chat.oobe.action.tryAgain"));

  componentProps(findComponent(tree, components.Button), components.Button).onClick();
  assert.deepEqual(generateCalls, [true], "retry re-generates");
});

test("Approve starts the suggestion server-side and navigates to the returned thread", () => {
  // The backend creates the thread/run and returns the binding; the browser
  // only navigates to what it is handed — it never injects the prompt.
  const opened = [];
  const { tree, components, startCalls } = renderSurface({
    onOpenThread: (threadId) => opened.push(threadId),
  });
  const cardProps = componentProps(
    findComponent(tree, components.SuggestedTaskCard),
    components.SuggestedTaskCard,
  );

  cardProps.onApprove();
  assert.equal(startCalls.length, 1);
  assert.equal(startCalls[0].id, CARD.id, "starts the exact suggestion");

  // Drive the mutation's success callback the way react-query would.
  startCalls[0].options.onSuccess({
    suggestion_id: CARD.id,
    thread_id: "thread-7",
    run_id: "run-7",
  });
  assert.deepEqual(opened, ["thread-7"], "navigates to the bound thread");
});

test("a card mid-start is marked starting", () => {
  const { tree, components } = renderSurface({ startingId: CARD.id });
  assert.equal(
    componentProps(
      findComponent(tree, components.SuggestedTaskCard),
      components.SuggestedTaskCard,
    ).starting,
    true,
  );
});

test("an already-started card opens its durable thread binding", () => {
  const opened = [];
  const started = { ...CARD, thread_id: "thread-3", run_id: "run-3" };
  const { tree, components } = renderSurface({
    suggestions: [started],
    onOpenThread: (threadId) => opened.push(threadId),
  });
  componentProps(
    findComponent(tree, components.SuggestedTaskCard),
    components.SuggestedTaskCard,
  ).onOpenThread();
  assert.deepEqual(opened, ["thread-3"]);
});

test("dismiss reports the suggestion id to the backend-backed hook", () => {
  const { tree, components, dismissCalls } = renderSurface();
  componentProps(
    findComponent(tree, components.SuggestedTaskCard),
    components.SuggestedTaskCard,
  ).onDismiss();
  assert.deepEqual(dismissCalls, [CARD.id]);
});
