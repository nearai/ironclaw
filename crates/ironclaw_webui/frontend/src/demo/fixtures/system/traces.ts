// Trace Commons fixtures: an enrolled contributor with credit history,
// recent explanations, one held submission awaiting authorization, and a
// submitted-traces list.

import { DAY, HOUR, MINUTE, iso } from "./helpers";

type TraceHold = { submission_id: string; reason: string };

const holds: TraceHold[] = [
  {
    submission_id: "sub-9f21c4d8-hold",
    reason: "Manual review: trace includes a shell transcript with file paths.",
  },
];

const creditState = {
  enrolled: true,
  pending_credit: 12.4,
  final_credit: 86.25,
  delayed_credit_delta: 3.1,
  submissions_submitted: 57,
  submissions_accepted: 51,
  submissions_total: 61,
};

export function traceCreditView() {
  return {
    ...creditState,
    last_submission_at: iso(4 * HOUR),
    last_credit_sync_at: iso(35 * MINUTE),
    recent_explanations: [
      "+2.50 — accepted batch of 5 tool-use traces (quality tier A).",
      "+0.60 — dedup bonus: trace family not previously represented.",
      "-0.75 — delayed adjustment: one trace reclassified as near-duplicate.",
    ],
    holds,
  };
}

export function authorizeTraceHold(submissionId: string): boolean {
  const index = holds.findIndex((hold) => hold.submission_id === submissionId);
  if (index < 0) return false;
  holds.splice(index, 1);
  creditState.submissions_submitted += 1;
  creditState.pending_credit += 0.4;
  return true;
}

export function accountTracesView() {
  return {
    enrolled: true,
    traces: [
      {
        submission_id: "sub-b81e77a0",
        status: "accepted",
        pending_credit: 0,
        final_credit: 1.85,
        received_at: iso(4 * HOUR),
      },
      {
        submission_id: "sub-77c0d914",
        status: "scoring",
        pending_credit: 0.4,
        final_credit: null,
        received_at: iso(9 * HOUR),
      },
      {
        submission_id: "sub-2f4aa6c3",
        status: "accepted",
        pending_credit: 0,
        final_credit: 2.1,
        received_at: iso(DAY + 2 * HOUR),
      },
      {
        submission_id: "sub-9f21c4d8-hold",
        status: "held",
        pending_credit: 0.4,
        final_credit: null,
        received_at: iso(DAY + 6 * HOUR),
      },
      {
        submission_id: "sub-05dd13be",
        status: "rejected",
        pending_credit: 0,
        final_credit: 0,
        received_at: iso(3 * DAY),
      },
    ],
  };
}

export function accountLoginLink() {
  return {
    minted: true,
    enrolled: true,
    url: "https://trace-commons.near.ai/account/session/demo-one-time-link",
  };
}
