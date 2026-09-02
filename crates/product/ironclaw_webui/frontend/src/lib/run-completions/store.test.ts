import assert from "node:assert/strict";
import { test } from "vitest";

import type { RunCompletionNotice } from "./protocol";
import {
  applyClear,
  applyNotice,
  applySettled,
  rebaseFromSnapshot,
  resetRunCompletionStore,
  runCompletionSnapshot,
  unreadForThread,
} from "./store";

function notice(sequence: string, overrides: Partial<RunCompletionNotice> = {}): RunCompletionNotice {
  return {
    schema: "webui.run_completion.v1",
    sequence,
    notice_id: `rcn-${sequence}`,
    run_id: `run-${sequence}`,
    thread_id: "thread-a",
    thread_tag: "rct-thread-a",
    completed_at: "2026-09-01T00:00:00Z",
    read: false,
    unread_count_for_thread: 1,
    ...overrides,
  };
}

test("rebase seeds the cache newest-first and resumes past every seeded sequence", () => {
  resetRunCompletionStore();
  rebaseFromSnapshot([notice("3"), notice("12"), notice("7", { read: true })], "5");
  const snapshot = runCompletionSnapshot();
  assert.deepEqual(
    snapshot.notices.map((entry) => entry.sequence),
    ["12", "3"],
    "read notices never enter the unread cache; ordering is by sequence",
  );
  assert.equal(snapshot.unreadCount, 2);
  assert.equal(snapshot.resumeSequence, "12", "resume from the greatest known sequence");
});

test("clears and settled ids advance the resume position without inventing read state", () => {
  resetRunCompletionStore();
  rebaseFromSnapshot([notice("3"), notice("4")], "4");
  applyClear("rcn-3", "9");
  assert.deepEqual(
    runCompletionSnapshot().notices.map((entry) => entry.notice_id),
    ["rcn-4"],
  );
  assert.equal(runCompletionSnapshot().resumeSequence, "9");
  applySettled(["rcn-4", "rcn-never-seen"]);
  assert.equal(runCompletionSnapshot().unreadCount, 0);
  applyNotice(notice("10", { read: true }));
  assert.equal(runCompletionSnapshot().unreadCount, 0, "a read notice is not unread");
  assert.equal(runCompletionSnapshot().resumeSequence, "10");
});

test("the cache keeps at most 250 unread notices, evicting the oldest", () => {
  resetRunCompletionStore();
  for (let sequence = 1; sequence <= 260; sequence += 1) {
    applyNotice(notice(String(sequence), { thread_id: `thread-${sequence % 3}` }));
  }
  const snapshot = runCompletionSnapshot();
  assert.equal(snapshot.unreadCount, 250);
  assert.equal(snapshot.notices[0].sequence, "260");
  assert.equal(snapshot.notices[249].sequence, "11", "the ten oldest were evicted");
  assert.equal(snapshot.resumeSequence, "260");
  assert.ok(unreadForThread("thread-0").every((entry) => entry.thread_id === "thread-0"));
});
