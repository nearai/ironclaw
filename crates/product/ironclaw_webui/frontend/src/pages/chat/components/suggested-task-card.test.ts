// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

// Same vm-harness convention as empty-state.test.ts / chat-input.test.ts: read
// the real source, strip imports, rename the export, and stub the module's free
// variables (Button/Icon) so the synthetic-JSX tree can be walked directly.
// `useT` returns the key, so we assert on the i18n KEY each state renders.
//
// The card takes a backend `Suggestion` — no app/tool metadata and no connect
// state, because the backend card schema carries none (see
// docs/internal/design/oobe/VISION-RECONCILIATION.md §3). It also does not
// import NearProcessIndicator: the "starting" state calls a
// `renderRunningIndicator` render prop supplied by its caller, keeping the
// lazy surface chunk from pulling that component in a second time.
function cardSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-card.tsx", import.meta.url),
    "SuggestedTaskCard",
  );
}

const SUGGESTION = {
  id: "sug-1",
  title: "Triage your inbox",
  description: "Reply to routine mail and archive newsletters.",
  suggested_prompt: "Triage my inbox.",
};

function renderCard({ suggestion = SUGGESTION, ...props } = {}) {
  const components = { Button() {}, Icon() {}, SuggestionIcon() {} };
  const context = {
    ...components,
    globalThis: {},
    // Echo the key, and append interpolation values so provenance assertions
    // can see the substituted `sources` (real i18n substitutes {sources}).
    useT: () => (key, vars) =>
      vars && Object.keys(vars).length ? `${key}:${Object.values(vars).join(",")}` : key,
    // The card resolves a semantic icon from the suggestion's required `icon`
    // enum; the resolution logic is covered in suggestion-icons.test.ts, so here we
    // just echo `icon` (or 'generic') to observe what the card forwards to
    // SuggestionIcon.
    resolveIconId: (s) => (s && s.icon) || "generic",
    // Provenance formatting is covered in suggestions-api.test.ts; echo a join
    // so we can assert the card renders a "From …" line from `sources`.
    formatSources: (list) => (list || []).join(", "),
  };
  vm.runInNewContext(cardSourceForTest(), context);
  const tree = context.globalThis.__testExports.SuggestedTaskCard({
    suggestion,
    ...props,
  });
  return { tree, components };
}

// Walk the synthetic tree for raw <button> onClick handlers (the dismiss
// control is a plain button, not the design-system Button).
function rawButtonHandlers(node, found = []) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return found;
  const strings = node.strings || [];
  node.values.forEach((value, index) => {
    if (typeof value === "function" && /onClick=\s*$/.test(strings[index] || "")) {
      found.push(value);
    }
    rawButtonHandlers(value, found);
  });
  return found;
}

test("an unstarted card offers Approve as its primary action", () => {
  const { tree, components } = renderCard();
  const button = findComponent(tree, components.Button);
  assert.ok(button, "an unstarted card renders an action button");
  assert.equal(componentProps(button, components.Button).variant, "primary");
  assert.ok(JSON.stringify(tree).includes("chat.oobe.action.approve"));
});

test("Approve reports upward so the surface can start the suggestion server-side", () => {
  const approvals = [];
  const { tree, components } = renderCard({ onApprove: () => approvals.push(true) });
  componentProps(findComponent(tree, components.Button), components.Button).onClick();
  assert.deepEqual(approvals, [true]);
});

test("a card being started shows the running indicator instead of an action", () => {
  // The start call is in flight: the card must not still offer Approve, or a
  // second click would fire a second start.
  const { tree, components } = renderCard({
    starting: true,
    renderRunningIndicator: (label) => `indicator:${label}`,
  });
  assert.equal(findComponent(tree, components.Button), null, "no action button while starting");
  assert.ok(JSON.stringify(tree).includes("indicator:chat.oobe.status.starting"));
});

test("a started card (durable thread binding) offers View in thread, not Approve", () => {
  // `thread_id` is the backend's durable suggestion->thread binding, so a
  // returning user rejoins the run rather than starting it twice.
  const opened = [];
  const { tree, components } = renderCard({
    suggestion: { ...SUGGESTION, thread_id: "thread-9", run_id: "run-9" },
    onOpenThread: () => opened.push(true),
  });
  const serialized = JSON.stringify(tree);
  assert.ok(serialized.includes("chat.oobe.action.openThread"));
  assert.ok(!serialized.includes("chat.oobe.action.approve"), "a started card cannot re-approve");

  componentProps(findComponent(tree, components.Button), components.Button).onClick();
  assert.deepEqual(opened, [true]);
});

test("the card renders the suggestion's own title and description", () => {
  const { tree } = renderCard();
  const serialized = JSON.stringify(tree);
  assert.ok(serialized.includes(SUGGESTION.title));
  assert.ok(serialized.includes(SUGGESTION.description));
});

test("the card shows a 'From <sources>' provenance line when sources are present", () => {
  const { tree } = renderCard({
    suggestion: { ...SUGGESTION, icon: "email", sources: ["Mail", "Chat"] },
  });
  const serialized = JSON.stringify(tree);
  // i18n key + the formatted sources both appear (t() echoes the key here).
  assert.ok(serialized.includes("chat.oobe.from"), "renders the provenance i18n key");
  assert.ok(serialized.includes("Mail, Chat"), "includes the formatted sources");
});

test("the card omits the provenance line when there are no sources", () => {
  const { tree } = renderCard({ suggestion: { ...SUGGESTION, sources: [] } });
  assert.ok(!JSON.stringify(tree).includes("chat.oobe.from"), "no provenance line without sources");
});

test("the card renders a SuggestionIcon for the suggestion's resolved task category", () => {
  const { tree, components } = renderCard({
    suggestion: { ...SUGGESTION, icon: "messaging" },
  });
  const icon = findComponent(tree, components.SuggestionIcon);
  assert.ok(icon, "the card mounts a SuggestionIcon");
  assert.equal(
    componentProps(icon, components.SuggestionIcon).id,
    "messaging",
    "the icon id comes from the suggestion's icon field",
  );
});

test("dismiss reports upward", () => {
  const dismissed = [];
  const { tree } = renderCard({ onDismiss: () => dismissed.push(true) });
  const [dismissHandler] = rawButtonHandlers(tree);
  assert.equal(typeof dismissHandler, "function", "the card renders a dismiss control");
  dismissHandler();
  assert.deepEqual(dismissed, [true]);
});
