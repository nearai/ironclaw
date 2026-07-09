import assert from "node:assert/strict";
import { test } from "vitest";

import {
  addPending,
  recordAcceptedMessageRef,
} from "./pending-messages";

test("recordAcceptedMessageRef: null and non-msg refs leave pending record unchanged", () => {
  const refs = [null, undefined, 123, "thread:1", "message-1"];

  for (const acceptedMessageRef of refs) {
    const store = new Map();
    const record = {
      id: "pending-1",
      role: "user",
      content: "check my calendar",
      timestamp: "2026-06-02T10:00:00.000Z",
      isOptimistic: true,
    };

    addPending(store, "thread-1", record);

    assert.equal(
      recordAcceptedMessageRef(
        store,
        "thread-1",
        "pending-1",
        acceptedMessageRef,
      ),
      null,
    );
    assert.deepEqual(store.get("thread-1"), [record]);
  }
});
