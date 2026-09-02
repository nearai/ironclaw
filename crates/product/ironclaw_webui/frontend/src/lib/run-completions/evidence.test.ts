import assert from "node:assert/strict";
import { test } from "vitest";

import { recordObservedRun, resetCoordinationForTests } from "./coordination";
import { observedThroughSequence, resumeCursorFor } from "./evidence";
import { applyNotice, resetRunCompletionStore } from "./store";
import type { RunCompletionNotice } from "./protocol";

function notice(sequence: string, runId: string, threadId: string): RunCompletionNotice {
  return {
    schema: "webui.run_completion.v1",
    sequence,
    notice_id: `rcn-${sequence}`,
    run_id: runId,
    thread_id: threadId,
    thread_tag: `rct-${threadId}`,
    completed_at: "2026-09-01T00:00:00Z",
    read: false,
    unread_count_for_thread: 1,
  };
}

test("the resume cursor is the JSON-quoted rc token the server hands out", () => {
  assert.equal(resumeCursorFor("42"), '"rc:42"');
  assert.equal(JSON.parse(resumeCursorFor("42")), "rc:42");
});

test("thread-read evidence advances only through replies this tab rendered", () => {
  resetRunCompletionStore();
  resetCoordinationForTests();
  applyNotice(notice("5", "run-5", "thread-a"));
  applyNotice(notice("7", "run-7", "thread-a"));
  applyNotice(notice("9", "run-9", "thread-b"));

  assert.equal(observedThroughSequence("thread-a"), null, "nothing rendered yet");
  recordObservedRun("run-5");
  assert.equal(observedThroughSequence("thread-a"), "5", "the newer reply is not on screen");
  recordObservedRun("run-7");
  assert.equal(observedThroughSequence("thread-a"), "7");
  assert.equal(observedThroughSequence("thread-b"), null, "other threads are untouched");
});
