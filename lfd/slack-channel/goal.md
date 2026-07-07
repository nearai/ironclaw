# Slack Channel LFD Pilot Goal

## Target

Build and improve the Slack channel integration until the LFD score shows that raw Slack Events API payloads are normalized, admitted, routed, deduplicated, and suppressed correctly across representative Slack surfaces.

This stacked pilot is intentionally narrow. It uses the real `ironclaw_slack_v2_adapter::parse_slack_event` parser and the shared LFD runner/profile/scorer infrastructure from `codex/lfd-blue-lanes`, then admits parsed Slack `UserMessage` payloads into the existing Reborn integration harness. It does not yet exercise the full signed HTTP Slack host route or real Slack Web API delivery sink.

## Constraints

- Stage 0 gate: before optimizing this target, `cargo fmt --check`, `cargo test --test reborn_lfd_runner --no-run`, and the Slack dev score must run locally.
- The optimizer may change Slack implementation code and the Slack profile only when explicitly instructed by a human. The shared scorer, runner core, and answer files are read-only optimization instruments.
- Dev inputs are visible. Dev answers are visible only to the scorer. Holdout answers must stay outside the repo when created.
- Do not add per-case branches keyed by `slack_dev_`, `EvLfdSlack`, or exact eval case ids in product code.
- Do not implement routing, retry, idempotency, admission, or permission checks with model calls. These are deterministic product-runtime decisions.
- Budget ceiling for this pilot is zero paid API dollars; it runs on scripted model replies and local fixtures.

## Instruments

- `tests/integration/lfd/profiles/slack_channel.rs` assembles the parser-backed Slack profile and exposes profile-specific state queries.
- `lfd/slack-channel/eval/dev/cases/*.json` holds visible dev inputs.
- `lfd/slack-channel/harness/answers.dev.json` holds sealed dev contracts used by the scorer.
- `lfd/slack-channel/harness/score.sh` scores outcomes and runs lint first.
- `lfd/slack-channel/harness/probe.sh` generates perturbed dev cases to detect memorization.
- `lfd/slack-channel/harness/status.sh` reports score/spend history.

## Forced Entropy

The dev set is intentionally small, so it is not an acceptance suite. Treat it as a mechanism pilot only. The next stack must add holdouts outside the repo and widen to the remaining old Slack behaviors: thread continuation, unadmitted workspace denial, unknown-user connection gate, channel-connection gate, 429 retry, and permanent terminal delivery failure.

Probe cases perturb visible dev inputs and event ids. A large dev-vs-probe gap means the implementation is fitting fixtures rather than the Slack behavior.

## Cheat Fences

- Literal event-id or case-id branching is capped by `harness/caps.json`.
- Answer canaries and future holdout canaries void scoring when leaked into the repo.
- Lint reports details only under the state root, while scorer stdout says only `VOID: constraint violation`.
- State contracts require both positive and negative directions: accepted routes must produce turns and delivery counts, while no-op messages must produce zero turns and zero posts.
- The duplicate-event case requires exactly one accepted turn and one delivery count despite two inbound payloads.

## Stop Conditions

Stop the pilot when all four dev cases and their probe variants score `1.0000`, or when the profile boundary proves insufficient for the next Slack behavior. In the latter case, stack a follow-up PR that moves the profile from parser-backed synthetic admission to the full signed Slack host route.
