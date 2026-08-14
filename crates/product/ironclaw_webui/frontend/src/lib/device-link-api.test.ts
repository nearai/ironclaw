// @ts-nocheck
import assert from "node:assert/strict";
import { beforeEach, test, vi } from "vitest";

vi.mock("./api", () => ({
  apiFetch: vi.fn(async () => ({})),
}));

import { apiFetch } from "./api";
import {
  cancelDeviceLink,
  pollDeviceLink,
  startDeviceLink,
  submitDeviceLinkInput,
} from "./device-link-api";

beforeEach(() => {
  apiFetch.mockClear();
});

function sentBody() {
  return JSON.parse(apiFetch.mock.calls.at(-1)[1].body);
}

// The regression: the chat auth-gate card renders from a gate model that has
// no `threadId` at all and leaves `invocationId` null, so it handed this
// module empty strings. Every one of these is parsed host-side into a
// validated newtype (`ThreadId`, `InvocationId`, `TurnRunRef`, `AuthGateRef`)
// that rejects a blank value, so the start route answered `400
// invalid_request` and the link could never begin. An absent id must be
// omitted from the body, never sent blank.
test("startDeviceLink omits blank optional ids instead of sending empty strings", () => {
  startDeviceLink({
    provider: "telegram",
    extensionName: "telegram",
    mode: "default",
    threadId: "",
    runId: "",
    gateRef: "",
    invocationId: "",
    resumeFlowId: "",
  });

  assert.deepEqual(sentBody(), {
    provider: "telegram",
    extension_name: "telegram",
    mode: "default",
  });
});

test("startDeviceLink keeps the ids it actually has", () => {
  startDeviceLink({
    provider: "telegram",
    extensionName: "telegram",
    runId: "run-1",
    gateRef: "gate-1",
    invocationId: "inv-1",
  });

  assert.deepEqual(sentBody(), {
    provider: "telegram",
    extension_name: "telegram",
    run_id: "run-1",
    gate_ref: "gate-1",
    invocation_id: "inv-1",
  });
});

// Required fields are deliberately NOT filtered: a blank provider must reach
// the host and be rejected, rather than vanishing into a request that would
// mean something else entirely.
test("startDeviceLink still sends a blank required field so the host rejects it", () => {
  startDeviceLink({ provider: "", extensionName: "" });

  assert.deepEqual(sentBody(), { provider: "", extension_name: "" });
});

test("poll, input, and cancel omit a blank invocation id", () => {
  pollDeviceLink({ flowId: "flow-1", invocationId: "" });
  assert.deepEqual(sentBody(), { flow_id: "flow-1" });

  submitDeviceLinkInput({
    flowId: "flow-1",
    revision: 2,
    kind: "code",
    value: "12345",
    invocationId: "",
  });
  assert.deepEqual(sentBody(), {
    flow_id: "flow-1",
    revision: 2,
    kind: "code",
    value: "12345",
  });

  cancelDeviceLink({ flowId: "flow-1", invocationId: "" });
  assert.deepEqual(sentBody(), { flow_id: "flow-1" });
});

// `revision: 0` is a real frame revision and must survive the filter — it is
// only ever applied to optional identifiers, never to a numeric field.
test("submitDeviceLinkInput preserves a zero revision", () => {
  submitDeviceLinkInput({
    flowId: "flow-1",
    revision: 0,
    kind: "code",
    value: "12345",
  });

  assert.equal(sentBody().revision, 0);
});
