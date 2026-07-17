// @ts-nocheck
//
// SSE wire-contract fixture round-trip: proves the Rust emission side
// (`crates/ironclaw_webui_v2/tests/webui_v2_schema_contract.rs`'s
// `sse_wire_contract_fixtures_match_committed_json`, plus the `error`
// fixture in `webui_v2_handlers_contract.rs`) and this frontend's SSE
// parsing agree on wire shape.
//
// Today that agreement is pinned by nothing but a comment in `useSSE.ts`
// ("mirror WebChatV2Event::event_name() in schema.rs") — a renamed or
// dropped field on either side would only surface nightly, in Playwright.
// This test drives each committed fixture through the REAL `useSSE` +
// `useChatEvents` hooks (via the shared `vm`-context harnesses already
// built for their own unit tests — imported, not reimplemented) and
// asserts every field the Rust side emits actually lands in observable
// state.
//
// Fixtures are the shared contract artifact: the same JSON files under
// `../../../../../tests/fixtures/sse_wire_contract/` are read here AND by
// the Rust test above. Regenerate both together after an intentional wire
// change:
//   UPDATE_SSE_FIXTURES=1 cargo test -p ironclaw_webui_v2
// then re-run this suite (`pnpm test`) to confirm the frontend still
// consumes every field.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

import { createHarness } from "./useSSE.test";
import { createUseChatEventsHarness } from "./useChatEvents.test";
import { gateFromEvent, gateFromProjectionGate } from "./gates";
import { toolCardFromActivity, toolCardFromPreview } from "./history-messages";

const FIXTURE_NAMES = [
  "final_reply",
  "running",
  "capability_progress",
  "capability_activity",
  "capability_display_preview",
  "gate",
  "auth_required",
  "projection_snapshot",
  "projection_update",
  "keep_alive",
  "error",
];

function loadFixture(name) {
  const raw = readFileSync(
    new URL(`../../../../../tests/fixtures/sse_wire_contract/${name}.json`, import.meta.url),
    "utf8",
  );
  return JSON.parse(raw);
}

const fixtures = Object.fromEntries(FIXTURE_NAMES.map((name) => [name, loadFixture(name)]));

// Values `useChatEvents.ts` builds from object literals inside its own
// source are constructed in the `vm.runInNewContext` sandbox's realm, not
// this file's — `assert.deepEqual` treats same-shape values from different
// realms as unequal (different `Object`/`Array` intrinsics). A JSON
// round-trip strips that, matching `useChatEvents.test.ts`'s own `plain()`
// helper. Values threaded through an *injected* real function (e.g.
// `gateFromEvent`, imported in this file's realm) don't need it — the
// closure's literals bind to the realm where the function was defined.
function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

test("every committed SSE fixture has a corresponding named event listener in useSSE", () => {
  // Catches a fixture added on the Rust side without the matching
  // `V2_EVENT_NAMES` entry on the frontend — the exact class of silent
  // desync this whole test exists to prevent.
  const { streams } = createHarness();
  const stream = streams[0];
  for (const name of FIXTURE_NAMES) {
    assert.ok(
      typeof stream.listener(name) === "function",
      `useSSE never registered a listener for SSE event "${name}"`,
    );
  }
});

// Dispatches `fixtureName`'s raw JSON through the REAL useSSE `dispatchFrame`
// closure (via the named listener the hook registered with `EventSource`)
// and returns exactly what useSSE hands to `onEvent` — i.e. exactly what
// `useChat.ts` wires into `useChatEvents`'s handler in production.
function dispatchFixtureThroughUseSSE(fixtureName) {
  const captured = [];
  const { streams } = createHarness({ onEvent: (envelope) => captured.push(envelope) });
  const stream = streams[0];
  const listener = stream.listener(fixtureName);
  assert.ok(listener, `no useSSE listener registered for "${fixtureName}"`);
  listener({
    data: JSON.stringify(fixtures[fixtureName]),
    lastEventId: "cursor:wire-contract-test",
  });
  assert.equal(captured.length, 1, `dispatchFrame did not call onEvent for "${fixtureName}"`);
  return captured[0];
}

for (const name of FIXTURE_NAMES) {
  test(`useSSE round-trips the committed "${name}" fixture without dropping fields`, () => {
    const envelope = dispatchFixtureThroughUseSSE(name);
    // `frame.type` is the canonical source; frames with no `type` field
    // (the `error` frame) fall back to the SSE `event:` name — the one
    // behavior only observable by driving the real named listener rather
    // than calling a parsing function directly.
    assert.equal(envelope.type, fixtures[name].type || "error");
    assert.deepEqual(envelope.frame, fixtures[name]);
  });
}

test('useChatEvents "final_reply": every reply field lands in the rendered message', () => {
  const envelope = dispatchFixtureThroughUseSSE("final_reply");
  const harness = createUseChatEventsHarness();
  harness.handleEvent(envelope);

  const { reply } = fixtures.final_reply;
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].content, reply.text);
  assert.equal(harness.messages[0].timestamp, reply.generated_at);
  assert.equal(harness.messages[0].turnRunId, reply.turn_run_id);
  assert.equal(harness.messages[0].isFinalReply, true);
  assert.equal(harness.isProcessing, false);
});

for (const name of ["running", "capability_progress"]) {
  test(`useChatEvents "${name}": progress.turn_run_id drives the active run`, () => {
    const envelope = dispatchFixtureThroughUseSSE(name);
    const harness = createUseChatEventsHarness();
    harness.handleEvent(envelope);

    const { progress } = fixtures[name];
    assert.deepEqual(plain(harness.activeRun), {
      runId: progress.turn_run_id,
      threadId: "thread-1",
      status: "running",
    });
    assert.equal(harness.isProcessing, true);
  });
}

test('useChatEvents "capability_activity": activity fields reach the tool-activity card', () => {
  const envelope = dispatchFixtureThroughUseSSE("capability_activity");
  const harness = createUseChatEventsHarness();
  harness.handleEvent(envelope);

  const { activity } = fixtures.capability_activity;
  const expectedCard = toolCardFromActivity(activity);
  const message = harness.messages.find(
    (candidate) => candidate.id === `tool-${activity.invocation_id}`,
  );
  assert.ok(message, "expected a tool_activity message for the fixture's invocation_id");
  assert.equal(message.role, "tool_activity");
  assert.equal(message.invocationId, activity.invocation_id);
  assert.equal(message.turnRunId, activity.turn_run_id);
  assert.equal(message.capabilityId, activity.capability_id);
  assert.equal(message.toolStatus, expectedCard.toolStatus);
  assert.equal(message.toolDetail, expectedCard.toolDetail);
});

test('useChatEvents "capability_display_preview": preview fields reach the tool-activity card', () => {
  const envelope = dispatchFixtureThroughUseSSE("capability_display_preview");
  const harness = createUseChatEventsHarness();
  harness.handleEvent(envelope);

  const { preview } = fixtures.capability_display_preview;
  const expectedCard = toolCardFromPreview(preview);
  const message = harness.messages.find(
    (candidate) => candidate.id === `tool-${preview.invocation_id}`,
  );
  assert.ok(message, "expected a tool_activity message for the fixture's invocation_id");
  assert.equal(message.invocationId, preview.invocation_id);
  assert.equal(message.turnRunId, preview.turn_run_id);
  assert.equal(message.toolStatus, expectedCard.toolStatus);
  assert.equal(message.toolName, expectedCard.toolName);
  assert.equal(message.capabilityId, preview.capability_id);
  assert.equal(message.toolDetail, expectedCard.toolDetail);
  assert.equal(message.resultRef, expectedCard.resultRef);
  assert.equal(message.truncated, expectedCard.truncated);
  assert.equal(message.outputBytes, expectedCard.outputBytes);
  assert.equal(message.outputKind, expectedCard.outputKind);
});

for (const name of ["gate", "auth_required"]) {
  test(`useChatEvents "${name}": prompt fields reach pendingGate via the real gateFromEvent mapping`, () => {
    const envelope = dispatchFixtureThroughUseSSE(name);
    const harness = createUseChatEventsHarness({ gateFromEvent });
    harness.handleEvent(envelope);

    const { prompt } = fixtures[name];
    // Computed from the real mapping function on the same fixture data,
    // rather than a hand-duplicated expected shape, so this assertion
    // tracks `gates.ts` instead of drifting from it.
    assert.deepEqual(harness.pendingGate, gateFromEvent(name, prompt));
    assert.equal(harness.isProcessing, false);
  });
}

for (const name of ["projection_snapshot", "projection_update"]) {
  test(`useChatEvents "${name}": run_status/text/gate items all land in observable state`, () => {
    const envelope = dispatchFixtureThroughUseSSE(name);
    const harness = createUseChatEventsHarness({ gateFromProjectionGate });
    harness.handleEvent(envelope);

    const [runStatusItem, textItem, gateItem] = fixtures[name].state.items;

    assert.deepEqual(plain(harness.activeRun), {
      runId: runStatusItem.run_status.run_id,
      threadId: "thread-1",
      status: runStatusItem.run_status.status,
    });
    assert.equal(
      harness.messages.some(
        (message) =>
          message.id === `text-${textItem.text.id}` && message.content === textItem.text.body,
      ),
      true,
      "expected the projection text item to render as an assistant message",
    );
    assert.deepEqual(harness.pendingGate, gateFromProjectionGate(gateItem.gate));
  });
}

test('useChatEvents "keep_alive": mutates no observable state', () => {
  const envelope = dispatchFixtureThroughUseSSE("keep_alive");
  const harness = createUseChatEventsHarness();
  harness.handleEvent(envelope);

  assert.deepEqual(harness.messages, []);
  assert.equal(harness.pendingGate, null);
  assert.equal(harness.isProcessing, false);
  assert.equal(harness.activeRun, null);
});

test('useChatEvents "error": error/kind/retryable reach onStreamError and the transcript', () => {
  const envelope = dispatchFixtureThroughUseSSE("error");
  const seenErrors = [];
  const harness = createUseChatEventsHarness({
    onStreamError: (payload) => seenErrors.push(payload),
  });
  harness.handleEvent(envelope);

  const fixture = fixtures.error;
  assert.deepEqual(plain(seenErrors), [
    { error: fixture.error, kind: fixture.kind, retryable: fixture.retryable },
  ]);
  assert.equal(harness.messages.length, 1);
  assert.equal(harness.messages[0].role, "error");
  assert.equal(
    harness.messages[0].content,
    `stream:${fixture.error}:${fixture.kind}:${fixture.retryable}`,
  );
});
