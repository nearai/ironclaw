# Trace Commons held-trace review

Status: proposed (2026-06-10)
Branch: `trace-commons-agent-onboarding` (PR #4559)

## Problem

When the autonomous turn-end trace capture decides a trace is **not**
eligible for auto-submission, it is silently lost. `capture_turn_trace`
(`crates/ironclaw_reborn_composition/src/trace_capture.rs`) builds the
envelope, gets back `TraceClientAutonomousCaptureOutcome::Held { submission_id,
reason }`, logs it at `debug!`, and returns — the envelope is discarded.

Consequences:

- The user has **no visibility** that a trace was held (the WebUI
  `/api/webchat/v2/traces/credit` response — `TraceCreditReport` — has no
  held field at all).
- There is **no way to authorize** a held trace; even retroactive approval
  is impossible because the candidate is gone.

The most common hold today is the High residual-PII-risk gate
(`trace_autonomous_eligibility`, after the 2026-06-10 change that lets
below-High auto-submit). The held flow is what handles those High-risk
traces that a user may still want to contribute after reviewing them.

## Existing machinery (reuse — do not rebuild)

A held trace already has a durable on-disk representation everywhere
*except* the autonomous capture path:

| Symbol (`contribution.rs`) | Role |
|---|---|
| `TraceQueueHold { submission_id, kind, reason, attempts, next_retry_at }` | The hold record |
| `TraceQueueHoldKind::{PolicyGate, ManualReview, RetryableSubmissionFailure}` | `ManualReview` is the PII/manual-approval hold |
| `queue_trace_envelope_for_scope` | Writes the envelope `.json` into `<scope>/queue/` |
| `write_trace_queue_hold_sidecar_for_path` | Writes the `<envelope>.held.json` sidecar |
| `trace_queue_hold_path_for_envelope_path` | Maps envelope path → `.held.json` path |
| `read_trace_queue_holds_for_scope` | Lists holds from the sidecars |
| `manual_review_hold_count` (in `TraceQueueDiagnostics`) | Count of `ManualReview` holds |

The flush worker **already skips** envelopes that carry a hold sidecar, and
there is already an `fs::remove_file(hold_path)` path used for retry-clear.
So: a held trace = queued `.json` + `.held.json` sidecar; removing the
sidecar promotes it to be submitted on the next flush.

The gap is solely that the **autonomous capture path never writes these for
PII/manual-review holds** — it drops the envelope instead.

## Decisions

- **Authorize = promote as-is.** On authorize, record the user's explicit
  consent and remove the `.held.json` sidecar so the flush worker submits the
  already-deterministically-redacted envelope. No re-redaction pass. The
  authorize action means "I reviewed the residual risk and accept
  submission."
- **Endpoint = extend `/api/webchat/v2/traces/credit`.** Add a held count and
  a held list to the existing trace-credits response so one fetch powers the
  whole card/tab. The authorize action is a new POST.

## Slices (each independently shippable, TDD)

### Slice 1 — Retain held traces (runtime + traces)

`TraceClientAutonomousCaptureOutcome::Held` must carry the built envelope
(today it carries only `submission_id` + `reason`). On `Held`,
`capture_turn_trace` queues the envelope via `queue_trace_envelope_for_scope`
and writes a `ManualReview` hold sidecar (`write_trace_queue_hold_sidecar_for_path`)
with the hold reason. Nothing downstream changes — the flush worker already
skips held sidecars.

- Test (traces): a Held capture leaves a retained envelope `.json` + a
  `ManualReview` `.held.json` sidecar in the scope queue.
- Test (composition, in-memory): `capture_turn_trace` on a held outcome
  persists a reviewable hold rather than dropping.

This slice alone stops the silent data loss.

### Slice 2 — Surface held count + list (endpoint)

Extend the trace-credit facade and the
`/api/webchat/v2/traces/credit` response with:

```jsonc
{
  // ...existing TraceCreditReport fields...
  "manual_review_hold_count": 2,
  "holds": [
    { "submission_id": "uuid", "reason": "manual review required because residual privacy risk is high" }
  ]
}
```

Sourced from `manual_review_holds_for_scope` filtered to
`TraceQueueHoldKind::ManualReview`. No raw trace content in the response — each
`RebornTraceHold` carries only `submission_id` and `reason`.

### Slice 3 — UI (frontend)

- Card (`sidebar-trace-credits.js`): when `manual_review_hold_count > 0`, show
  a "N held for review" line.
- Settings tab (`trace-commons-tab.js`): a held-traces list, each row with an
  **Authorize** button. Reuses the `useTraceCredits` poll (extend hook/api to
  carry holds + an authorize mutation).

### Slice 4 — Authorize (endpoint + action)

`POST /api/webchat/v2/traces/holds/{submission_id}/authorize`:

1. Resolve the scope from the authenticated `WebUiAuthenticatedCaller`
   (never from the body).
2. Record explicit consent (a typed consent record / audit line keyed by
   `submission_id`).
3. Remove the `.held.json` sidecar for that submission so the flush worker
   submits the envelope on its next pass.
4. Return a sanitized result; never echo raw trace content.

Descriptor: small JSON body, per-caller rate limit, bearer-auth (same layer
as the other v2 trace routes).

## Security / invariants

- Held sidecars and the endpoint never expose raw trace payloads — only
  `submission_id`, reason, timestamp, score.
- Authorize derives scope from the verified caller, not the request body
  (matches the product-auth caller-construction rule).
- Authorize is the only path that promotes a `ManualReview` hold; it must
  record consent before removing the sidecar (fail closed if the consent
  write fails).
- High-risk traces remain blocked from *auto*-submission; authorize is an
  explicit, per-trace, user-initiated exception.

## Out of scope

- Re-redaction-on-authorize (deferred; "promote as-is" chosen).
- Bulk authorize / auto-expiry of held traces.
- Surfacing `PolicyGate` / `RetryableSubmissionFailure` holds in the UI (this
  feature is scoped to `ManualReview`).
