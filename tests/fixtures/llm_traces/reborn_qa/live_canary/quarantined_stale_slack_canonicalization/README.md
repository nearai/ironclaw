# Quarantined stale-slack-canonicalization traces

These 10 files are unmodified model traces harvested from live-canary run
`29837220214` at commit `c918d91943a84071726924b4e3e9a47d33d8f695`.
Each trace invokes a `slack.*` tool with a pre-canonicalization argument shape
(`channel`, `thread_ts`, `user_id`, `types`, or `count`/`sort`).

Retired 2026-07-28: the standardized messaging framework canonicalized Slack
tool inputs (`channel`→`conversation`, `thread_ts`→`thread`, `user_id`→`user_ref`,
`types`→`kinds`, `count`/`sort`→`limit`/`cursor`) and closed every messaging
input schema (`additionalProperties: false`,
`crates/contracts/ironclaw_host_api/schemas/messaging/*.input.v1.json`). Replayed as
recorded, these traces would fail pre-dispatch schema validation instead of
reaching the provider, so they no longer describe an executable model/tool
contract.

They are preserved for provenance but are not active model/tool-choice
contracts. Rewriting a recorded tool call or response would fabricate model
evidence. Replace a quarantined case only by re-recording the scenario with
live Slack credentials against the current canonical schema (follow-up),
importing it as a review-required candidate, reviewing its expectations and
external-service doubles, and passing
`scripts/ci/check-reborn-qa-fixtures.sh`.

The promoted inventory remains authoritative in the parent
`case-manifest.json`; its `quarantined_model_cases` list must exactly account
for the JSON traces in this directory.
