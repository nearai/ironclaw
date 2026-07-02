# Enable Reborn final-answer nudges for planned_default and scheduled_trigger

> **Correction (same day):** the original scoping below targeted the literal
> `RunProfileId::interactive_default()` / `interactive_profile()` construct.
> Verification during plan-writing found that real production interactive/
> chat/CLI turns do **not** resolve to that profile: `submit_user_turn`
> (`crates/ironclaw_reborn_composition/src/runtime.rs:2064`, the real submit
> path) calls `turn_coordinator.submit_turn(..)` with
> `requested_run_profile: None`, and the production resolver
> (`default_planned_run_profile_resolver()`) defaults an unspecified request
> to **`planned_default`** (`reborn-planned-default`), proven by the existing
> test at `crates/ironclaw_reborn/src/planned_driver_factory.rs:535`. The raw
> `interactive_profile()` literal is a neutral-contract-layer default in
> `ironclaw_turns`, not what real Serve traffic serves. The corrected target
> is **`planned_default`** (the profile that actually serves real interactive
> turns) and `scheduled_trigger`, with `subagent` staying off — the rest of
> this document has been updated to match.

## Context

Reborn's agent-loop executor has one nudge mechanism, `try_final_answer_nudge`
(`crates/ironclaw_agent_loop/src/executor/loop_exit.rs`). When a run is about
to end in a no-progress failure or hit its iteration/model-call budget with no
real answer, the nudge issues one extra tool-free model call asking the model
to synthesize a closing answer from the work already done, then routes the
reply through the normal `ReplyAdmissionStrategy`. It's capped at one nudge
per run (`state.final_answer_nudges_used`).

The nudge is gated by `SteeringPolicy.allow_driver_specific_nudges`
(`crates/ironclaw_turns/src/run_profile/policy.rs`), a field on every
`RunProfileDefinition`. As of this investigation, every production profile
construction site hardcodes this field to `false` — the nudge is fully wired
and tested but dormant everywhere in production. It's flipped `true` only by
a test builder (`with_driver_nudges_enabled()`).

We want to turn the nudge on for two profiles: `planned_default` (real
interactive chat/CLI/web turns — see correction above) and
`scheduled_trigger` (trigger-fired runs, issue #5505). `subagent` should stay
off. The literal `interactive_default`/`interactive_profile()` construct is
untouched — it doesn't serve real traffic, so it's out of scope.

## Shared base, still not a blind flip

`planned_default`, `subagent`, and `scheduled_trigger` are each built through
one shared helper, `planned_like_profile_definition(...)`
(`crates/ironclaw_reborn/src/planned_driver_factory.rs:230`), which wraps
`RunProfileDefinition::interactive_like(...)` — itself a clone of
`interactive_profile()` (`crates/ironclaw_turns/src/run_profile/resolver.rs`)
that overrides only `profile_id`, `loop_driver`, `checkpoint_schema_id`/
`version`, and `capability_surface_profile_id`; `steering_policy` passes
through unchanged (`false`). Editing the shared helper itself would flip all
three consumers, including `subagent`, which must stay off. So the opt-in
still needs to happen at the two individual profile-definition functions, not
the shared helper or the `interactive_profile()` base — this is actually
simpler than the original (rejected) target, since `interactive_profile()`
itself needs no change at all now.

## Approach

Add a builder method on `RunProfileDefinition`, mirroring the existing
`with_personal_context_policy` pattern already used for the same kind of
per-profile override:

```rust
// crates/ironclaw_turns/src/run_profile/resolver.rs, near with_personal_context_policy
pub fn with_driver_specific_nudges(mut self, enabled: bool) -> Self {
    self.steering_policy.allow_driver_specific_nudges = enabled;
    self
}
```

Chain it at exactly two call sites in
`crates/ironclaw_reborn/src/planned_driver_factory.rs`, both mapping the
`Result<RunProfileDefinition, RunProfileRegistryError>` returned by
`planned_like_profile_definition(...)`:

1. **`planned_default_profile_definition()`** (line 249).
2. **`scheduled_trigger_planned_profile_definition()`** (line 279).

`subagent_planned_profile_definition()` (line 262) is untouched — it keeps
inheriting `false` from the shared helper, with no override needed. This
keeps the blast radius to exactly the two intended profiles and doesn't
require remembering to "undo" anything for the profile that should stay off.

Also reword the now-inaccurate "off in production" comment on
`try_final_answer_nudge` (`loop_exit.rs:32`).

### Alternative considered and rejected

Add a `driver_specific_nudges: bool` parameter to `planned_like_profile_definition(...)`
itself, threaded from each of the three callers. Rejected: it would grow that
helper's already-3-argument signature for a policy axis only two of its three
callers care about, and the builder-chain approach reads the intent
("this profile explicitly opts into nudges") right at the two call sites that
want it, without touching the shared helper or its one caller that doesn't.

## Risk notes (checked, no action needed)

- `RunProfileFingerprint` includes `allow_driver_specific_nudges` in its
  hash, so the fingerprint changes for the two affected profiles. No test
  pins a golden fingerprint value (only inequality checks), and fingerprint
  is model-visible-rendering/observability metadata, not a checkpoint-resume
  compatibility gate. Safe.
- The nudge's one-shot cap and checkpoint slot (`final_answer_nudges_used`)
  are already implemented and tested; no schema change needed.
- Enabling the nudge adds one extra tool-free model call on the no-progress /
  budget-exhaustion failure path for real `planned_default` (interactive) and
  `scheduled_trigger` runs — a small latency/cost tax on an already-rare
  failure path.

## Test plan

- **Unit** (`resolver.rs`): builder defaults to `false`; explicit `true`
  round-trips through `.resolve(...)`.
- **Unit** (`planned_driver_factory.rs`): extend the existing
  `planned_driver_live_default_smoke`-style resolution assertions (or add a
  new focused test) to assert `planned_default` resolves with
  `allow_driver_specific_nudges == true`. Extend the existing
  `scheduled_trigger_profile_resolves_with_denied_surface_id` test to also
  assert `allow_driver_specific_nudges == true`. Add an explicit `false`
  assertion to the `subagent_profile_resolves_to_subagent_planned_driver`
  test as a regression guard.
- **Integration (mandatory per this repo's "test through the caller, not
  just the helper" rule** — the flag gates a real LLM call, not just
  internal state): new test in `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`,
  adapted from the already-passing `repeated_signature_stops_after_rendered_warning_and_no_progress_result`
  pattern in `crates/ironclaw_agent_loop/tests/safety_nets.rs` (same script:
  `ScenarioScript::same_calls_repeated("demo.echo", 4)` with a `completed_no_change`
  outcome on the 4th call — proven to drive `CanonicalAgentLoopExecutor` to a real
  `NoProgressDetected` exit through the full strategy pipeline). The new test
  swaps in a `LoopRunContext` carrying the real production-resolved
  `planned_default` profile (via `default_planned_run_profile_resolver()`,
  the same resolver `planned_driver_live_default_smoke` already uses) instead
  of the default synthetic test context, and asserts the run now
  **completes** with a 5th (nudge) model call instead of failing closed —
  proving the flag set in production profile resolution actually reaches and
  fires the real nudge mechanism, not just that the struct field is `true`.

## Out of scope

- No changes to `long_running_mission_profile()` or any other profile.
- No changes to the nudge mechanism itself (`try_final_answer_nudge`), the
  one-shot cap, or checkpoint schema.
- No changes to the legacy `ironclaw_engine` / v1 nudge systems (unrelated
  code, different mechanism entirely).
