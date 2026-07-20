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
