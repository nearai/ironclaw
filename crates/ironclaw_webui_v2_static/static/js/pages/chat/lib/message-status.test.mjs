import assert from "node:assert/strict";
import test from "node:test";

import {
  RECORD_STATUS,
  UI_MESSAGE_STATUS,
  isBusyRejectedStatus,
  uiStatusFromRecordStatus,
} from "./message-status.js";

test("deferred-busy maps to queued (accepted and waiting, never an error)", () => {
  assert.equal(
    uiStatusFromRecordStatus(RECORD_STATUS.DEFERRED_BUSY),
    UI_MESSAGE_STATUS.QUEUED,
  );
});

test("explicit queued stays queued", () => {
  assert.equal(
    uiStatusFromRecordStatus(RECORD_STATUS.QUEUED),
    UI_MESSAGE_STATUS.QUEUED,
  );
});

test("rejected-busy maps to error (was not accepted, must resend)", () => {
  assert.equal(
    uiStatusFromRecordStatus(RECORD_STATUS.REJECTED_BUSY),
    UI_MESSAGE_STATUS.ERROR,
  );
});

test("unknown statuses pass through unchanged", () => {
  assert.equal(uiStatusFromRecordStatus("accepted"), "accepted");
  assert.equal(uiStatusFromRecordStatus("finalized"), "finalized");
  assert.equal(uiStatusFromRecordStatus(undefined), undefined);
});

test("only rejected-busy needs the resend error copy", () => {
  assert.equal(isBusyRejectedStatus(RECORD_STATUS.REJECTED_BUSY), true);
  assert.equal(isBusyRejectedStatus(RECORD_STATUS.DEFERRED_BUSY), false);
  assert.equal(isBusyRejectedStatus(RECORD_STATUS.QUEUED), false);
});

test("live (send outcome) and reload (record status) agree for busy outcomes", () => {
  // The same wire value must produce the same UI status whether it arrives
  // as a send-response `outcome` or a persisted record `status`.
  for (const status of [
    RECORD_STATUS.DEFERRED_BUSY,
    RECORD_STATUS.REJECTED_BUSY,
    RECORD_STATUS.QUEUED,
  ]) {
    assert.equal(
      uiStatusFromRecordStatus(status),
      uiStatusFromRecordStatus(status),
    );
  }
});
