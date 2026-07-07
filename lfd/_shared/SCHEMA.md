# LFD Shared Schema v1

Single source of truth for the three JSON artifacts that couple the eval
runner (`tests/lfd/`), the scorer (`lfd/_shared/scorer/`), and every
per-feature eval. Version every breaking change (`schema_version` field).

## 1. Case (eval INPUT — visible to the optimizer)

One file per case: `lfd/<feature>/eval/dev/cases/<case_id>.json`
(holdout equivalents live outside the repo).

```json
{
  "schema_version": 1,
  "case_id": "slack_dev_001",
  "profile": "slack_channel",
  "title": "inbound mention routes to correct tenant thread",
  "setup": {
    "extensions": ["slack"],
    "secrets": [{"credential_name": "slack_bot_token", "value": "test-token-1"}],
    "memory_docs": [],
    "triggers": [],
    "http_stubs": [{"key": "slack.postMessage", "status": 200, "body": {"ok": true}}],
    "profile_extra": {}
  },
  "llm_script": [
    {"turn": 1, "steps": [
      {"tool": "builtin.some_tool", "params": {"x": 1}},
      {"text": "final reply text"}
    ]}
  ],
  "inbound": [
    {"channel": "slack", "payload": {"type": "app_mention", "user": "U123", "text": "..."}}
  ],
  "state_queries": [
    {"id": "thread_scope", "kind": "thread_record", "params": {"index": 0}}
  ],
  "live": false
}
```

- `profile`: selects the runner profile (harness assembly). One profile per
  feature; registry in `tests/lfd/profiles/`.
- `llm_script`: scripted model output (FIFO, one entry per model turn). For
  system-behavior evals the model's behavior is part of the INPUT. Omitted
  when `live: true` (live-model mode uses a real provider).
- `setup.profile_extra`: profile-specific setup, schema owned by the profile.
- `state_queries`: declarative reads the runner executes AFTER the scenario
  (against real persisted state, not in-memory echoes).

## 2. Outcome (emitted by the runner — one per case)

Written to `$LFD_OUT/<case_id>.outcome.json`.

```json
{
  "schema_version": 1,
  "case_id": "slack_dev_001",
  "status": "ran",
  "error": null,
  "replies": [{"channel": "slack", "text": "...", "seq": 7}],
  "tool_invocations": [{"name": "builtin.some_tool", "params_json": "{...}", "ok": true, "seq": 2}],
  "egress": [{"method": "POST", "url": "https://slack.com/api/chat.postMessage", "seq": 5}],
  "events": [{"kind": "TurnStarted", "seq": 1}],
  "gates": [{"kind": "approval", "resolution": "approved", "seq": 4}],
  "state": {"thread_scope": {"tenant": "t1", "agent": "a1"}},
  "leaks": {"secret_scan_hits": 0},
  "meta": {"profile": "slack_channel", "runner_hash": "<sha256 of pinned runner sources>", "duration_ms": 812}
}
```

- `status`: `ran` | `error` (harness raised) | `unsupported` (profile can't
  execute this case yet — scores 0, never skipped silently).
- `leaks.secret_scan_hits`: scrubber (pattern set from
  `scripts/ci/check-reborn-qa-fixtures.sh`) run over replies + events +
  transcripts; any injected secret literal appearing in output counts.
- `state`: results keyed by `state_queries[].id`, read from persisted
  storage (DB/filesystem) after the run — not from runner-local variables.

## 3. Contract (eval ANSWER — sealed)

Dev: `lfd/<feature>/harness/answers.dev.json` (read-BANNED for the
optimizer, canary-tokened, lint-enforced).
Holdout: `$LFD_STATE_ROOT/holdout/<feature>/answers.holdout.json`
(outside the repo entirely).

```json
{
  "schema_version": 1,
  "canary_token": "LFDC-slack-9f3a1c2e",
  "contracts": [
    {
      "case_id": "slack_dev_001",
      "hard_fail_on_error": true,
      "required": [
        {"id": "r1", "type": "tool", "name": "builtin.some_tool", "params_contains": {"x": 1}, "weight": 2},
        {"id": "r2", "type": "reply_contains", "channel": "slack", "substrings_any": ["done", "posted"], "weight": 1},
        {"id": "r3", "type": "state_pred", "query": "thread_scope", "path": "tenant", "eq": "t1", "weight": 3},
        {"id": "r4", "type": "egress", "method": "POST", "url_contains": "chat.postMessage", "weight": 1},
        {"id": "r5", "type": "event", "kind_any": ["TurnCompleted"], "weight": 1},
        {"id": "r6", "type": "gate", "kind": "approval", "resolution": "approved", "weight": 1}
      ],
      "forbidden": [
        {"type": "egress", "url_contains": "admin.slack.com"},
        {"type": "leak"},
        {"type": "tool", "name": "builtin.secrets_read"}
      ],
      "ordered": [["r1", "r4", "r2"]]
    }
  ]
}
```

Matcher types (closed set, scorer rejects unknown): `tool`,
`reply_contains`, `reply_regex`, `state_eq`, `state_pred`, `event`,
`egress`, `gate`, `leak` (forbidden-only; satisfied iff
`secret_scan_hits > 0`), `status` (e.g. required `status == "error"` for
fail-closed cases), `transcript_wer` (word-level WER of a state-query
string vs a sealed reference, satisfied iff `wer <= max` — for
speech-to-text scoring).

## 4. Scoring (implemented in `score_core.py`, stated here once)

```
case_score = (Σ weight of satisfied required / Σ all required weights)
           × 0.5^(number of distinct forbidden matchers observed)
           × (0 if any `ordered` sequence violated else 1)
status == "error" and hard_fail_on_error → case_score = 0
status == "unsupported"                  → case_score = 0
feature_score = mean(case_score over all cases in the set)
```

Both failure directions priced: missing required behavior starves the
numerator; spurious/forbidden behavior halves the score per violation
class; fabricating errors away is impossible (errors zero the case).

## 5. Feedback resolution (leak budget, stated once)

- Dev mode prints: aggregate score, per-case PASS/FAIL for ≤ 5 worst
  cases (case ids only — inputs are visible anyway), and NOTHING about
  which matcher failed or what it expected.
- Holdout mode prints: one aggregate number. Max 3 holdout calls per 24 h window;
  every call appends to `$LFD_STATE_ROOT/audit/<feature>.log`.
- Lint violations: scorer prints `VOID: constraint violation` and exits.
  Detailed findings go to `$LFD_STATE_ROOT/lint-reports/`
  (outside the optimizer's surface, for the human).

## 6. Runner trust boundary

- `tests/lfd/**` and `tests/integration/support/**` are hash-pinned in
  `lfd/<feature>/harness/pins.json`; the scorer recomputes and embeds the
  hash in every outcome's `meta.runner_hash` audit trail.
- Exactly ONE agent-writable runner file per feature:
  `tests/lfd/profiles/<feature>.rs` (harness assembly only). Outcome
  EXTRACTION lives in pinned code reading harness recorders and persisted
  state; profiles cannot fabricate outcomes through the supported API.
- Holdout scoring is only accepted from a pristine runner (pins match
  `git show` of the LFD-creation commit).
