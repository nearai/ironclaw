import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../test-support/vm-module-harness";
import { CEREMONY_OUTCOME } from "../../lib/ledger/ceremony";

const HASH = "77".repeat(32);

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const child of node) visit(child, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  visit(node.children, fn);
  visit(node.props?.children, fn);
}

function nodeBy(root, predicate, description) {
  let match = null;
  visit(root, (node) => {
    if (!match && predicate(node)) match = node;
  });
  assert.ok(match, `expected ${description}`);
  return match;
}

function findByTestId(root, testId) {
  return nodeBy(
    root,
    (node) => node.props?.["data-testid"] === testId,
    `a node with data-testid=${testId}`,
  );
}

/** Concatenate all string content under a node. */
function textOf(node) {
  let text = "";
  visit(node, (child) => {
    for (const candidate of [child.children, child.props?.children].flat()) {
      if (typeof candidate === "string") text += candidate;
    }
  });
  return text;
}

/**
 * Load the page module with its imports injected, and a React double whose
 * `useState`/`useEffect` run synchronously so a rendered tree can be asserted
 * without a DOM.
 */
function load({
  fetchResult,
  fetchError,
  signingContext = { clear_signing: "unavailable" },
  params = { intentId: "intent-1" },
}: {
  fetchResult?: unknown;
  fetchError?: unknown;
  signingContext?: unknown;
  params?: { intentId: string };
}) {
  const effects = [];
  // One slot per useState call site, keyed by call order within a render —
  // the page has more than one piece of state, and a single shared slot would
  // silently alias them.
  const slots = [];
  let slotIndex = 0;
  const context = {
    React: {
      useState: (initial) => {
        const index = slotIndex++;
        if (slots.length <= index) slots.push(initial);
        return [
          slots[index],
          (next) => {
            slots[index] = next;
          },
        ];
      },
      useEffect: (effect) => effects.push(effect),
      useCallback: (fn) => fn,
    },
    useParams: () => params,
    // The harness strips imports, so the page's collaborators are injected.
    CEREMONY_OUTCOME,
    runCeremony: async () => ({ outcome: CEREMONY_OUTCOME.unsupported }),
    createDevicePort: () => null,
    useT: () => (key, vars) =>
      vars ? `${key}:${JSON.stringify(vars)}` : key,
    fetchIntentDetail: () =>
      fetchError ? Promise.reject(fetchError) : Promise.resolve(fetchResult),
    fetchSigningContext: () =>
      signingContext instanceof Error
        ? Promise.reject(signingContext)
        : Promise.resolve(signingContext),
    Date,
    Math,
    Number,
    Object,
    JSON,
    Promise,
  };

  const exports = runVmModuleForTest(
    "./review-page.tsx",
    [
      "ReviewPage",
      "TransactionHash",
      "DetailRow",
      "decodedRows",
      "SigningCeremony",
      "groupHex",
      "millisRemaining",
      "minutesRemaining",
    ],
    context,
    import.meta.url,
  );

  return {
    exports,
    /** Render once, flush the fetch effect, then render again. */
    async render() {
      slotIndex = 0;
      exports.ReviewPage();
      for (const effect of effects.splice(0)) effect();
      // Drain the microtask queue: the rejection path runs one `.then` deeper
      // than the resolve path, so a fixed number of ticks would be flaky.
      await new Promise((resolve) => setImmediate(resolve));
      slotIndex = 0;
      return exports.ReviewPage();
    },
  };
}

const INTENT = {
  intent_id: "intent-1",
  state: "pending",
  chain_id: "eip155:11155111",
  approved_tx_hash: HASH,
  expires_at_ms: Date.now() + 30 * 60 * 1000,
  decoded_tx: { chain: "evm", nonce: 7, gas_limit: 21000 },
};

/**
 * THE property of this page. A Ledger shows the hash it is about to sign, and
 * the human compares it to what IronClaw says it asked for. An abbreviated
 * `0xab…cd` defeats that: an attacker who chooses the transaction can usually
 * grind the visible ends, so a truncated render matches a tampered transaction
 * just as happily as the real one.
 */
test("the approved transaction hash renders in full, never truncated", () => {
  const { exports } = load({ fetchResult: INTENT });

  const rendered = textOf(exports.TransactionHash({ hash: HASH }));
  assert.ok(rendered.includes(HASH), "the hash must render complete");
  assert.ok(!rendered.includes("…") && !rendered.includes("..."), "no ellipsis");
  // Belt and braces: no CSS/JS truncation of the element that carries it.
  const element = findByTestId(exports.TransactionHash({ hash: HASH }), "review-approved-tx-hash");
  assert.equal(
    element.children.join(""),
    HASH,
    "the hash element holds the whole value",
  );
  assert.ok(
    !/truncate|text-ellipsis|line-clamp/.test(element.props.className || ""),
    "the hash must not be visually truncated either",
  );
});

/** The page must hand the whole hash to that component, not a prepared excerpt. */
test("the page passes the unabridged hash down", async () => {
  const page = load({ fetchResult: INTENT });
  const tree = await page.render();

  const node = nodeBy(
    tree,
    (candidate) => candidate.props?.hash !== undefined,
    "the hash component",
  );
  assert.equal(node.props.hash, HASH);
});

/** Grouping is for readability and must never drop a character. */
test("hex grouping preserves every character", () => {
  const { exports } = load({ fetchResult: INTENT });
  assert.equal(exports.groupHex(HASH, 8).join(""), HASH);
  assert.equal(exports.groupHex("abcdef", 4).join(""), "abcdef");
  // Length rather than deepEqual: the module runs in its own VM realm, so its
  // arrays are not reference-comparable with this file's.
  assert.equal(exports.groupHex("", 4).length, 0);
});

/**
 * The server answers unknown / not-yours / expired with one bodyless 404 so it
 * is not an oracle. Rendering a distinguishing message would undo that on the
 * client.
 */
test("a 404 renders the single unavailable state, not a reason", async () => {
  const error: Error & { status?: number } = new Error("Not Found");
  error.status = 404;
  const page = load({ fetchError: error });
  const tree = await page.render();

  const unavailable = findByTestId(tree, "review-unavailable");
  const text = textOf(unavailable);
  assert.ok(text.includes("review.unavailable"), "shows the uniform message");
  for (const leak of ["approver", "tenant", "expired", "exists"]) {
    assert.ok(!text.includes(leak), `must not hint at ${leak}`);
  }
});

/** A real failure is distinguishable from a refusal — it is not a leak. */
test("a non-404 failure renders the error state", async () => {
  const error: Error & { status?: number } = new Error("gateway exploded");
  error.status = 500;
  const page = load({ fetchError: error });
  const tree = await page.render();
  assert.ok(findByTestId(tree, "review-error"), "error state rendered");
});

/**
 * A field this page does not understand must still be shown. Silently dropping
 * one hands an attacker a field they can set for free — it would be in the
 * signed bytes but absent from what the human checked.
 */
test("every decoded field is rendered, including unrecognized ones", () => {
  const { exports } = load({ fetchResult: INTENT });
  const rows = exports.decodedRows({
    nonce: 7,
    some_future_field: "surprise",
    nested: { a: 1 },
  });
  const keys = rows.map((row) => row.key).sort();
  assert.equal(keys.join(","), "nested,nonce,some_future_field");
  assert.equal(rows.find((row) => row.key === "nested").value, '{"a":1}');
});

/** Empty and absent values are dropped; zero and false are NOT. */
test("falsy-but-real decoded values survive", () => {
  const { exports } = load({ fetchResult: INTENT });
  const rows = exports.decodedRows({ nonce: 0, value: "", missing: null, ok: false });
  const keys = rows.map((row) => row.key).sort();
  assert.equal(
    keys.join(","),
    "nonce,ok",
    "a zero nonce is meaningful and must render",
  );
});

test("the countdown floors at zero rather than going negative", () => {
  const { exports } = load({ fetchResult: INTENT });
  assert.equal(exports.millisRemaining(1_000, 5_000), 0);
  assert.equal(exports.minutesRemaining(1_000, 5_000), 0);
  assert.equal(exports.minutesRemaining(5_000 + 3 * 60_000, 5_000), 3);
  // A malformed expiry must not render NaN into the page.
  assert.equal(exports.minutesRemaining(undefined, 5_000), 0);
});

/** A resolved intent shows its outcome and stops offering a countdown. */
test("a terminal intent renders its state without a countdown", async () => {
  const page = load({
    fetchResult: { ...INTENT, state: "approved" },
  });
  const tree = await page.render();

  assert.ok(textOf(findByTestId(tree, "review-state")).includes("approved"));
  let countdown = null;
  visit(tree, (node) => {
    if (node.props?.["data-testid"] === "review-countdown") countdown = node;
  });
  assert.equal(countdown, null, "a decided intent has nothing to count down to");
});


// --- Clear signing: no descriptor, no device path (§D3 / §D5) ---

/**
 * THE fail-closed property, at the surface where it matters. Without a
 * descriptor the device can only show a bare hash, which the human cannot
 * meaningfully check — blind signing wearing a hardware wallet. The sign
 * affordance must be ABSENT, not disabled: a disabled control is one patch away
 * from being enabled, and an absent one cannot be clicked by a user in a hurry.
 */
test("no descriptor renders the blocked state and no path to the device", () => {
  const { exports } = load({ fetchResult: INTENT });
  const blocked = exports.SigningCeremony({
    state: { status: "unavailable" },
    terminal: false,
  });

  assert.ok(findByTestId(blocked, "review-ceremony-blocked"), "blocked UX renders");
  let signAction = null;
  visit(blocked, (node) => {
    if (node.props?.["data-testid"] === "review-sign-action") signAction = node;
  });
  assert.equal(signAction, null, "there must be no sign affordance at all");
});

/** Anything that is not an explicit "available" blocks — including a failure. */
test("a failed or malformed signing-context response blocks", async () => {
  for (const context of [
    new Error("upstream down"),
    {},
    { clear_signing: "unknown-future-value" },
    null,
  ]) {
    const page = load({ fetchResult: INTENT, signingContext: context });
    const tree = await page.render();
    const ceremony = nodeBy(
      tree,
      (node) => node.props?.state?.status !== undefined,
      "the ceremony section",
    );
    assert.notEqual(
      ceremony.props.state.status,
      "available",
      `${JSON.stringify(context)} must not resolve to available`,
    );
  }
});

/** The ready branch exists, so the blocked branch is a real decision. */
test("an available descriptor offers the device path", () => {
  const { exports } = load({ fetchResult: INTENT });
  const ready = exports.SigningCeremony({
    state: { status: "available", descriptor: { context: {} } },
    terminal: false,
  });
  assert.ok(findByTestId(ready, "review-sign-action"), "the sign affordance appears");
});

/** A decided intent offers no ceremony regardless of descriptor state. */
test("a terminal intent never offers the device path", () => {
  const { exports } = load({ fetchResult: INTENT });
  assert.equal(
    exports.SigningCeremony({
      state: { status: "available", descriptor: {} },
      terminal: true,
    }),
    null,
  );
});

/**
 * The button must actually run the ceremony, and report a non-signed outcome
 * to the user. A control that looks live but is wired to nothing is worse than
 * an absent one — it teaches the user the flow works.
 */
test("the sign control runs the ceremony and surfaces its outcome", () => {
  const { exports } = load({ fetchResult: INTENT });
  const clicks = [];
  const ready = exports.SigningCeremony({
    state: { status: "available", descriptor: {} },
    terminal: false,
    running: false,
    outcome: null,
    onSign: () => clicks.push("clicked"),
  });

  const button = findByTestId(ready, "review-sign-action");
  button.props.onClick();
  assert.deepEqual(clicks, ["clicked"], "the control is wired to the ceremony");

  // And a completed run reports itself.
  const afterwards = exports.SigningCeremony({
    state: { status: "available", descriptor: {} },
    terminal: false,
    running: false,
    outcome: "unsupported",
    onSign: () => {},
  });
  const message = textOf(findByTestId(afterwards, "review-ceremony-outcome"));
  assert.ok(
    message.includes("unsupported"),
    "a browser that cannot reach a device must say so",
  );
});

/** While the device is being driven, the control must not re-fire. */
test("the sign control is disabled while a ceremony is running", () => {
  const { exports } = load({ fetchResult: INTENT });
  const ready = exports.SigningCeremony({
    state: { status: "available", descriptor: {} },
    terminal: false,
    running: true,
    outcome: null,
    onSign: () => {},
  });
  assert.equal(findByTestId(ready, "review-sign-action").props.disabled, true);
});
