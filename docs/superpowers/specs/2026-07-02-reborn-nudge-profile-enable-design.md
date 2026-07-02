# Enable Reborn final-answer nudges for interactive_default and scheduled_trigger

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

We want to turn the nudge on for two profiles: `interactive_default` (normal
chat/CLI turns) and `scheduled_trigger` (trigger-fired runs, issue #5505).
`planned_default` and `subagent` should stay off.

## Why this is not a one-line flip

All four of `interactive_default`, `planned_default`, `subagent`, and
`scheduled_trigger` ultimately derive from one function,
`interactive_profile()` (`crates/ironclaw_turns/src/run_profile/resolver.rs`):

- `interactive_default` is `interactive_profile()` returned as-is, registered
  in `InMemoryRunProfileRegistry::with_builtin_profiles()`.
- `planned_default`, `subagent`, and `scheduled_trigger` are each built by
  `RunProfileDefinition::interactive_like(...)`, which clones
  `interactive_profile()` and overrides only `profile_id`, `loop_driver`,
  `checkpoint_schema_id`/`version`, and `capability_surface_profile_id` — it
  does not touch `steering_policy`.

So flipping the literal inside `interactive_profile()` turns the nudge on for
all four profiles, not just the two we want. We need an opt-in mechanism that
doesn't change the shared base.

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

Chain it at exactly two call sites:

1. **`interactive_default`** — `with_builtin_profiles()` (resolver.rs):
   `interactive_profile().with_driver_specific_nudges(true)`.
2. **`scheduled_trigger`** — `scheduled_trigger_planned_profile_definition()`
   (`crates/ironclaw_reborn/src/planned_driver_factory.rs`): map the
   `planned_like_profile_definition(...)` result through
   `.with_driver_specific_nudges(true)`.

`planned_default_profile_definition()` and `subagent_planned_profile_definition()`
are untouched — they keep inheriting `false` from the base, with no override
needed. This keeps the blast radius to exactly the two intended profiles and
doesn't require remembering to "undo" anything for the profiles that should
stay off, which is the failure mode a "flip base + override back" approach
would carry forward if a fifth profile is added later.

Also reword the now-inaccurate "off in production" comment on
`try_final_answer_nudge` (`loop_exit.rs:32`).

### Alternative considered and rejected

Flip `interactive_profile()`'s field to `true` directly, then add explicit
`false` overrides in `planned_default_profile_definition()` and
`subagent_planned_profile_definition()` to cancel the inherited value. This
touches one more call site, and structurally relies on every non-target
consumer of the shared base remembering to opt back out — the builder
approach makes opt-in explicit at the two sites that want it and leaves
everything else alone by construction.

## Risk notes (checked, no action needed)

- `RunProfileFingerprint` includes `allow_driver_specific_nudges` in its
  hash, so the fingerprint changes for the two affected profiles. No test
  pins a golden fingerprint value (only inequality checks), and fingerprint
  is model-visible-rendering/observability metadata, not a checkpoint-resume
  compatibility gate. Safe.
- The nudge's one-shot cap and checkpoint slot (`final_answer_nudges_used`)
  are already implemented and tested; no schema change needed.
- Enabling the nudge adds one extra tool-free model call on the no-progress /
  budget-exhaustion failure path for real interactive and scheduled-trigger
  runs — a small latency/cost tax on an already-rare failure path.

## Test plan

- **Unit** (`resolver.rs`): builder defaults to `false`; explicit `true`
  round-trips through `.resolve(...)`.
- **Unit** (`resolver.rs`): `with_builtin_profiles()` resolves
  `interactive_default` with `allow_driver_specific_nudges == true`.
- **Unit** (`planned_driver_factory.rs`): extend the existing
  `scheduled_trigger_profile_resolves_with_denied_surface_id` test to also
  assert `allow_driver_specific_nudges == true`. Add explicit `false`
  assertions to the `planned_default` and `subagent` resolution tests as a
  regression guard.
- **Integration (mandatory per this repo's "test through the caller, not
  just the helper" rule** — the flag gates a real LLM call, not just
  internal state): new test in `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`
  using the scripted-provider harness (`tests/support/reborn/scripted_provider.rs`
  + `harness.rs`) that drives a real `interactive_default` (or
  `scheduled_trigger`) run into `NoProgressDetected` or `IterationLimit` exit
  and asserts the nudge actually fires — an extra tool-free model call
  happens and the run completes with a synthesized reply, instead of
  finalizing a `LoopFailureKind`.

## Out of scope

- No changes to `long_running_mission_profile()` or any other profile.
- No changes to the nudge mechanism itself (`try_final_answer_nudge`), the
  one-shot cap, or checkpoint schema.
- No changes to the legacy `ironclaw_engine` / v1 nudge systems (unrelated
  code, different mechanism entirely).
