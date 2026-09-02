import assert from "node:assert/strict";
import { test } from "vitest";

import { noticeFromWire, parseRunCompletionEvent } from "./protocol";

const wireNotice = {
  schema: "webui.run_completion.v1",
  sequence: "7",
  notice_id: "rcn-7",
  run_id: "run-7",
  thread_id: "thread-a",
  thread_tag: "rct-a",
  completed_at: "2026-09-01T00:00:00Z",
  read: false,
  unread_count_for_thread: 2,
};

test("notice events parse with a stringified sequence; malformed ones are dropped", () => {
  const parsed = parseRunCompletionEvent({
    type: "run_completion",
    event: { type: "notice", ...wireNotice },
  });
  assert.equal(parsed?.type, "notice");
  assert.equal(parsed && parsed.type === "notice" ? parsed.notice.sequence : null, "7");
  assert.equal(noticeFromWire({ ...wireNotice, run_id: "" }), null, "an empty id is malformed");
  assert.equal(noticeFromWire("not a record"), null);
});

test("grants need their identity fields; unknown event types are ignored, not failed", () => {
  const grant = {
    type: "grant",
    sequence: "7",
    notice_id: "rcn-7",
    grant_id: "rcg-1",
    browser_instance_id: "rbi-1",
    state_revision: 5,
    surface: "in_app",
    expires_at: "2026-09-01T00:00:02Z",
  };
  assert.equal(
    parseRunCompletionEvent({ type: "run_completion", event: grant })?.type,
    "grant",
  );
  assert.equal(
    parseRunCompletionEvent({
      type: "run_completion",
      event: { ...grant, browser_instance_id: undefined },
    }),
    null,
    "a grant that names no browser cannot be applied",
  );
  assert.equal(
    parseRunCompletionEvent({
      type: "run_completion",
      event: { ...grant, state_revision: "5" },
    }),
    null,
    "state_revision must be numeric",
  );
  assert.equal(
    parseRunCompletionEvent({ type: "run_completion", event: { type: "future_kind" } }),
    null,
  );
  assert.equal(parseRunCompletionEvent({ type: "thread", event: { type: "notice" } }), null);
  assert.equal(
    parseRunCompletionEvent({
      type: "run_completion",
      event: { type: "clear", notice_id: "rcn-7", thread_id: "thread-a" },
    })?.type,
    "clear",
  );
});
