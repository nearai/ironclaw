# Goal: make Slack a primary Reborn channel with correct routing, identity, gates, and delivery state

This is the Lane-03 launch package for Slack as main channel, reconciled from
`docs/lfd/roadmap-blue-lanes-2026-07-07/03-slack-as-main-channel/goal.md`,
`docs/lfd/roadmap-blue-lanes-2026-07-07/COMMON.md`,
`docs/lfd/roadmap-blue-lanes-2026-07-07/LANE-ADDENDA.md`, and
`lfd/_briefs/slack-channel.md`. Model-1 scope is deliberate: one Slack app,
bot speaks as the selected personal or channel agent, and Slack takeover,
posting as the user, and user-token impersonation are out of scope.

## Stage 0 - Build to spec (inner loop)

Implement `spec.md`. Make the test suite pass. Do not score against the eval
until tests are green. Tests stay green every cycle thereafter.

Stage-0 command list:

1. `cargo fmt --all`
2. `cargo clippy --all --benches --tests --examples --all-features`
3. `cargo test -p ironclaw_slack_v2_adapter`
4. `cargo test --features integration --test integration slack`
5. `cargo test --features integration --test group_triggers slack`
6. `cargo test --features integration --test lfd slack_channel`

The Stage-0 behavior must drive Slack payloads through the adapter and product
workflow seam, not only helper parsing. The LFD profile named `slack_channel`
must execute every dev case with `status: "ran"` before eval descent begins;
the current skeleton returns `unsupported`, which scores 0 by design.

## Target (outer loop)

Metric: Slack routing and delivery contract score, both directions. Recall is
priced by required matchers: eligible Slack DMs, app mentions, threaded replies,
approval/auth replies, setup actions, and triggered delivery events produce the
correct tenant, owner user, agent, conversation/thread, persisted state, gate,
and Slack `chat.postMessage` target. Precision is priced by forbidden matchers:
ineligible Slack traffic must not create turns or send replies; failures must
record typed denial or terminal state rather than silently dropping, crashing,
retrying forever, or leaking secrets/cross-tenant context.

Bar: **0.90 on holdout**. Score with `harness/score.sh`. A VOID result means a
constraint was violated; find and remove the violation, but the harness will
not tell the optimizer which item tripped it. Holdout is aggregate-only, max
3 calls per 24 hours, audit-logged. Acceptance is measured on holdout
exclusively.

Small-eval warning (verbatim per portfolio COMMON): Per-feature evals are
30-60 dev + 10-15 holdout cases: far below the ~200 enumerability threshold.
The compensating controls are (a) contract-style scoring (satisfying a
behavioral contract usually requires the machinery, unlike data-lookup evals),
(b) probe gap as the memorization gauge, (c) feedback capped to aggregate +
<=5 worst case ids, (d) holdout answers off-repo.

## Constraints

- Wall-clock budget: **12 h**. Check `harness/status.sh` every cycle; it shows
  elapsed time, score history, holdout budget, and spend ledger status.
- Spend ceiling: **$15** LLM/API. Scripted evals are deterministic and expected
  to spend $0. Live Slack calls are not part of this Wave-1 package.
- Surface allowlist for the optimizer:
  - Read/write: `crates/ironclaw_slack_v2_adapter/**`,
    `crates/ironclaw_reborn_composition/src/slack_*.rs`,
    Slack-specific integration tests, `lfd/slack-channel/LOG.md`, and
    `tests/integration/lfd/profiles/slack_channel.rs`.
  - Read-only: this `goal.md`, `spec.md`, `harness/**`, `eval/**`,
    `lfd/_shared/**`, and `tests/integration/lfd/**` except the one profile.
  - BANNED: `lfd/slack-channel/harness/answers.dev.json`, anything under
    `$LFD_STATE_ROOT/**`, holdout answers, and any other lane's `lfd/` package.
- Capacity caps are enforced by `harness/caps.json`: Slack id literals in
  `src/**` or `crates/**` diff = 0; eval case-id branching = 0; new
  `#[ignore]`/`#[cfg(never)]` test weakening = 0; trusted inbound minting from
  product/adapters = 0; `credential_name`/`extension_name` confusion = 0.
- Preserve host-mediated security boundaries. The adapter parses untrusted Slack
  payloads; the host verifies Slack signatures, stamps trusted context, resolves
  identity, and mediates egress. Do not mint trusted inbound requests in product
  adapters or product workflow.
- Preserve extension/auth naming invariants: `credential_name =
  "slack_bot_token"` for the bot token, `extension_name = "slack"` for setup
  routing and UI. Do not route setup by credential name.
- `goal.md`, `spec.md`, `harness/`, and `eval/` are read-only during
  optimization. Eval inputs may be read where the harness exposes them; eval
  answers never.

## Cycle protocol

1. Score dev: `lfd/slack-channel/harness/score.sh --outcomes <dir>`.
2. Reflect: run `lfd/slack-channel/harness/probe.sh`, then score the generated
   probe outcomes with `score.sh --probe lfd/slack-channel/eval/probe/map.json`.
   If the probe gap grows, the next change must remove or blind an eval-shaped
   artifact, not add another special case.
3. Status: run `lfd/slack-channel/harness/status.sh` and confirm wall-clock,
   holdout budget, and spend remain.
4. Hypothesize in `LOG.md` before editing: expected score movement, expected
   failure mode, and diagnostic.
5. Make one scoped change, then run the Stage-0 commands relevant to that seam
   plus the scorer again.
6. Checkpoint every cycle with `git commit -am "cycle <n>: <score>"`, gain or
   no gain, so the run is bisectable and crash-safe.

Each cycle must include one positive Slack route and one negative/adversarial
Slack route. If a fix lands for DMs, run the equivalent channel-thread variant
before committing.

## Entropy rules

- Stall rule: if the metric did not move last cycle, the next attempt must be a
  structural change, not the same knob harder.
- Exploration quota: every 5 cycles, try a different Slack surface even if the
  current approach is still improving: DM, public channel, private channel,
  thread reply, approval/auth resume, setup, admission, or triggered delivery.
- If duplicate/idempotency behavior is flat, the next change must alter event
  identity or persistence handling, not retry timing.
- If probe variants fail after text-only changes, remove text special cases and
  fix structured routing over tenant, installation, team, channel, user, and
  thread identifiers.

## Cheat audit

1. Reply to every Slack message -> negative cases require typed denial and
   forbid `chat.postMessage`.
2. Route all Slack traffic to the caller's personal agent -> channel route and
   managed-subject cases assert persisted subject resolution.
3. Ignore team or installation id -> isolation cases reuse Slack ids across
   tenants/installations.
4. Collapse thread and channel conversations -> state contracts assert
   `thread_ts` and stable thread scope.
5. Skip duplicate detection -> duplicate cases require one turn and one egress.
6. Treat unknown users as admins -> unknown-user cases require connection or
   pairing gates, not admin routing.
7. Hold admission permits across delivery polling -> admission cases assert
   durable accept releases the permit before long delivery observation.
8. Mark parked approval/auth delivery as failed -> terminal delivery cases
   require blocked-terminal state after actionable prompt delivery.
9. Hardcode Slack ids or case ids -> caps reject Slack id literals and case-id
   branching in product/test diffs; probe renames ids.
10. Read sealed answers or scorer internals -> canary and pins lint VOID without
    itemized feedback.
11. Confuse credential and extension names -> setup contracts assert both wire
    fields and caps reject the common string mix-up.
12. Implement Slack takeover/user-token posting -> out-of-scope egress and
    impersonation paths get no score and may VOID if they weaken safety.

## Stop conditions

Stop when the holdout aggregate is at least 0.90 with Stage 0 green, any budget
is exhausted, marginal dev gain is < 0.01 for 4 consecutive cycles, a critical
tenant/user/channel/credential isolation issue is discovered, or the scorer is
found invalid and cannot be repaired within budget. On stop, write a final
report in `LOG.md`: best dev score, best holdout score if any, probe gap trend,
what generalized, what was abandoned, remaining risks, and next highest-
leverage Slack seams.
