# Reborn QA run artifacts

QA testers can download a redacted artifact for any finalized assistant reply
with the **Download run artifact** action beside that reply. The request is
authenticated and derives tenant/user ownership from the session; the browser
never supplies a user id.

The downloaded `ironclaw.run_artifact.v1` JSON contains the exact run's user,
assistant, and tool-result context plus best-effort scoped process logs. Logs
are diagnostic only: the buffer is bounded and process-local, so
`logs.complete` is deliberately always `false`. Railway or other node-wide
logs are not part of the self-service export.

Convert a download into a review-required replay candidate:

```bash
python3 scripts/import-reborn-run-artifact.py \
  ~/Downloads/ironclaw-run-<run-id>.json \
  tests/fixtures/llm_traces/reborn_qa/<scenario>.candidate.json
```

Before renaming or committing the candidate:

1. Review its `_review.required_actions` and every redaction placeholder.
2. Add scenario-specific `expects` and caller-level end-state assertions.
3. Record or mock external HTTP/service exchanges so CI replay is hermetic.
4. Run `scripts/ci/check-reborn-qa-fixtures.sh`.

The importer intentionally produces a candidate, not an automatically blessed
golden fixture. Human QA evidence tells us what happened; a reviewer still owns
the assertion of what must continue happening.
