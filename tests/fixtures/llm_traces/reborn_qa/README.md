# Reborn QA trajectory artifacts

QA testers can download either the selected run or the full thread from any
finalized assistant reply. Both requests derive tenant/user ownership from the
authenticated session; the browser never supplies a user id.

The exact-run `ironclaw.run_artifact.v1` and complete-thread
`ironclaw.thread_artifact.v1` schemas contain redacted user, assistant, and
tool-result context plus best-effort scoped process logs. Thread messages
retain `run_id`, and the importer emits one candidate fixture turn per run.
Logs are diagnostic only: the buffer is bounded and process-local, so
`logs.complete` is deliberately always `false`. Railway or other node-wide
logs are not part of the self-service export.

Full-thread export is intentionally all-or-nothing: it returns HTTP `413`
instead of producing a silently partial fixture when the thread exceeds 1,000
messages, 16 MiB of stored message data, or 20 MiB after redaction and log
assembly.

Convert a download into a review-required replay candidate:

```bash
python3 scripts/import-reborn-run-artifact.py \
  ~/Downloads/ironclaw-thread-<thread-id>.json \
  /tmp/<scenario>.candidate.json \
  --source-url https://github.com/nearai/ironclaw/issues/<incident> \
  --owning-journey <journey-id>
```

The importer independently rescans the already-redacted artifact and fails
without printing raw matches if a secret, email, or developer path remains. It
records the source URL, scrub status, artifact schema/digest, and owning journey
under `_promotion`. Keep candidates outside the blessed fixture tree: CI rejects
`.candidate.json`, `_review`, empty assertions, or turns without replay steps.

Promotion is one ordered path:

1. Link the production/live failure and export the smallest complete thread
   that reproduces it.
2. Import and review the scrubbed candidate, including every placeholder and
   skipped run.
3. Move the reproduction to the lowest meaningful deterministic seam: a crate
   contract for a local rule, Reborn integration for cross-component behavior,
   a recorded fixture for model choice, or browser/provider coverage only when
   that surface is essential.
4. Add non-empty outcome assertions that fail on the bug, run the test before
   the fix, then implement the fix.
5. Record the exact replay command and successful commit/date in the promotion
   manifest, run `scripts/ci/check-reborn-qa-fixtures.sh`, and commit the
   fixture, test, and fix together.

An exemption is valid only when deterministic reproduction is impossible.
The PR body must state `Regression-test exemption: deterministic reproduction
is impossible because ...`; CI also requires the exemption label and an
approval from someone other than the author.

The importer intentionally produces a candidate, not an automatically blessed
golden fixture. Human QA evidence tells us what happened; a reviewer still owns
the assertion of what must continue happening.

For full-thread downloads, an accepted user message may lack a run id when turn
submission failed before a run was created. The importer excludes only those
incomplete submissions from replay turns and lists their sequence, kind, and
status under `_review.skipped_unscoped_messages` for explicit review. It also
preserves completed turns when another run is still awaiting a finalized
assistant response and reports that run under `_review.skipped_incomplete_runs`.
Malformed run groups still fail the import instead of being silently skipped.

## Replay freshness and live-case retirement

`live_canary/case-manifest.json` mechanically records provenance, scrub status,
fixture schema, journey ownership, last successful replay, and the retained
representative drift set. `scripts/ci/check-regression-promotions.py` fails on
missing metadata, stale replay, too few drift cases, or a retirement without an
existing deterministic fixture, exact replay command, reviewed reason, and
date. Every replayable harvested case must be accounted for as either scheduled
or retired. No-model and quarantined cases must remain scheduled until active
deterministic replay evidence exists.

During the 30-day review, a scheduled live case may be retired once its
permanent purpose is covered deterministically. Retired cases remain available
for explicit attended runs and historical diagnosis. The recurring lane may
retain more than the minimum representative drift set, but a case cannot be
both scheduled and retired.
