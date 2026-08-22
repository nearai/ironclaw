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
  const components = { SuggestedTaskCard() {}, Button() {}, Icon() {}, Link() {} };
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

// Refresh + connect (issue #7815 F1/F2). The synthetic-JSX factory records
// `{ type, props }` on every element node, so walking for the design-system
// `Button` reference and reading its props is enough to assert which control
// is which without depending on markup order.
function findAllByType(node, type, found = []) {
  if (Array.isArray(node)) {
    node.forEach((entry) => findAllByType(entry, type, found));
    return found;
  }
  if (!node || typeof node !== "object") return found;
  if (node.type === type) found.push(node);
  (Array.isArray(node.values) ? node.values : []).forEach((value) =>
    findAllByType(value, type, found),
  );
  return found;
}

const EXTENSIONS_ROUTE = "/extensions";

function refreshControl(tree) {
  // The compact drawer header uses raw controls (the design-system Button
  // applies its size class through a plain string join, so a height override
  // in className would lose to it) — so match the element, not the component.
  return findAllByType(tree, "button").find(
    (button) => button.props["aria-label"] === "chat.oobe.action.refresh",
  );
}

test("a ready set can be refreshed in place", () => {
  // Before this, the generate CTA only existed at zero cards, so a user
  // holding a stale set had no way to ask for another one short of dismissing
  // every card. Refresh reuses the same `generate` mutation.
  const { tree, generateCalls } = renderSurface();
  const refresh = refreshControl(tree);
  assert.ok(refresh, "the drawer header exposes a refresh control");
  assert.equal(refresh.props.disabled, false, "refresh is live on a ready set");

  refresh.props.onClick();
  assert.deepEqual(generateCalls, [true], "refresh re-generates");
});

test("refresh is inert while a generation is already in flight", () => {
  // `generate` is idempotent per client_action_id, but a live control during
  // generation reads as "nothing happened" — the request is claimed, not
  // queued. Disable it instead.
  const { tree } = renderSurface({
    status: "generating",
    isGenerating: true,
    suggestions: [CARD],
  });
  const refresh = refreshControl(tree);
  assert.ok(refresh, "the refresh control stays mounted during regeneration");
  assert.equal(refresh.props.disabled, true);
});

test("the drawer offers a connect entry into the extensions surface", () => {
  // First leg of the flow (connect tools -> ask for suggestions). Connect is a
  // separate surface, so this is a route entry, not an in-drawer OAuth panel.
  const { tree, components } = renderSurface();
  const connect = findAllByType(tree, components.Link).find(
    (link) => link.props.to === EXTENSIONS_ROUTE,
  );
  assert.ok(connect, "the drawer links to the connect surface client-side");
  assert.ok(JSON.stringify(tree).includes("chat.oobe.action.connect"));
  // The header label is hidden below `sm` (three controls plus the label wrap
  // the heading at 375px), so the accessible name has to come from the
  // attribute rather than the text node.
  assert.equal(connect.props["aria-label"], "chat.oobe.action.connect");
  const connectLabel = findAllByType(connect, "span").find(
    (span) => span.props.className === "hidden sm:inline",
  );
  assert.ok(
    connectLabel,
    "the header label stays hidden below sm, or the 375px wrap returns",
  );
});

test("the empty CTA pairs generate with the connect entry, generate first", () => {
  // Ordering is load-bearing: generate stays the primary action of the empty
  // state, and the existing CTA tests read the first Button.
  const { tree, components, generateCalls } = renderSurface({
    status: "empty",
    suggestions: [],
  });
  const buttons = findAllByType(tree, components.Button);
  assert.equal(buttons.length, 2, "generate + connect");

  buttons[0].props.onClick();
  assert.deepEqual(generateCalls, [true], "the first CTA is generate");
  assert.equal(buttons[1].props.to, EXTENSIONS_ROUTE, "the second is connect");
  assert.equal(
    buttons[1].props.as,
    components.Link,
    "connect navigates client-side rather than rendering a plain button",
  );
});

test("the failed CTA keeps the connect entry alongside retry", () => {
  // A failed generation is one of the likeliest moments to go connect
  // something, and retry must stay the first Button for the retry test.
  const { tree, components } = renderSurface({
    status: "failed",
    suggestions: [],
  });
  const buttons = findAllByType(tree, components.Button);
  assert.equal(buttons.length, 2);
  assert.equal(buttons[1].props.to, EXTENSIONS_ROUTE);
  assert.equal(buttons[1].props.as, components.Link);
});
