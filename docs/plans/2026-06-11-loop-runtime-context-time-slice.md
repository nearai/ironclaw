# Loop Runtime Context — Time-Only First Slice

**Date:** 2026-06-11
**Status:** approved design
**Parent plan:** `docs/plans/2026-06-01-4149-capability-scoped-runtime-context.md` (PR #4304)
**Issue:** #4149
**Scope:** Reborn loop only (`ironclaw_turns` / `ironclaw_agent_loop` / `ironclaw_reborn`)

## Goal

Surface runtime context in the Reborn agent loop prompt, starting with current
date/time only, through the exact seam the full #4149 plan will later extend.
This is PR 1 of that plan: the typed runtime-context stage exists end-to-end,
carrying a single category of content (time), so later posture/alias/subagent
fields land in the same struct, renderer, and section with zero rework.

## Decisions (locked during design)

1. **Stamp once at loop start.** The timestamp is captured when the loop
   spawns and stays fixed for the duration of the loop. All model calls in the
   loop render the identical prompt section — no per-call fingerprint churn,
   no provider prompt-cache busting. A loop that runs long sees a stale start
   time; the prompt text points the model at the time capability for
   freshness.
2. **UTC + optional user timezone.** UTC is always rendered (RFC3339, minute
   precision). User-local time is rendered only when a user timezone is
   actually known. Never guess the host timezone as the user's — the Reborn
   host may run in a different region than the user.
3. **Distinct runtime section, not `instruction_snippets`.** Locked by the
   parent plan: a new `runtime_context` field on `InstructionBundleRequest`,
   rendered by `InstructionBundleBuilder` as its own fingerprinted section.
4. **Position:** after identity messages, before instruction/skill snippets,
   safety context, and visible surface.
5. **Time enters via the request, not the builder.** The builder stays
   deterministic: same request, same bundle. No `Utc::now()` inside
   `InstructionBundleBuilder`.

## Design

### `LoopRuntimeContext` (new type)

In `crates/ironclaw_turns/src/run_profile/` (alongside the bundle types):

```rust
/// Model-visible runtime context for one loop execution.
///
/// First slice carries only time. The full #4149 plan adds capability
/// posture, scoped-path semantics, and subagent narrowing as additional
/// fields rendered into the same prompt section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopRuntimeContext {
    /// Loop start instant. Rendered at minute precision
    /// (e.g. "2026-06-11T21:32Z").
    pub loop_started_at_utc: chrono::DateTime<chrono::Utc>,
    /// Validated IANA timezone for the user (e.g. `chrono_tz::America::Los_Angeles`),
    /// when known. None = unknown; never a guessed host timezone.
    /// Invalid IANA names are rejected at the producer boundary at parse time, by construction.
    pub user_timezone: Option<chrono_tz::Tz>,
}
```

`InstructionBundleRequest` gains:

```rust
pub runtime_context: Option<LoopRuntimeContext>,
```

`None` produces no section and a bundle byte-identical to today — fully
backward compatible.

### Rendering (`push_runtime_context`)

New section helper in `instruction_bundle.rs`, following the existing
`push_safety_context` pattern: synthetic message ref with section name
`runtime`, `feed_field` fingerprint entries for section/ref/each field,
content validated with `validate_model_safe_text`.

Rendered system message:

- Timezone known (local time computed from the UTC stamp + IANA tz):

  ```text
  Current date/time at loop start: 2026-06-11T21:32Z (14:32 Thu, America/Los_Angeles).
  This was captured when this loop started; for the precise current time use the
  time capability if it is visible.
  ```

- Timezone unknown:

  ```text
  Current date/time at loop start: 2026-06-11T21:32Z.
  The user's timezone is unknown — if local time matters, ask the user or use the
  time capability if it is visible.
  ```

Section order in `InstructionBundleBuilder::build` as actually emitted today:
inline messages → identity → **runtime** → instruction snippets → memory
snippets → safety → surface. The runtime section sits in its locked position
relative to identity and the host sections. Inline-first is a pre-existing
deviation from the parent plan's locked order (inline should come last, after
all system context — it carries subagent handoffs); fixing it changes prompt
order and fingerprints for every bundle with inline messages, so it is
tracked separately in issue #4798 rather than folded into this slice.

### Derivation and wiring

- `HostManagedLoopPromptPort` gains `with_runtime_context(LoopRuntimeContext)`
  (same builder-method style as `with_safety_context`). When set, it is
  attached to every `InstructionBundleRequest` the port builds.
- `crates/ironclaw_reborn/src/loop_driver_host.rs` stamps the context at loop
  spawn (this is the single place wall-clock is read) and passes it into the
  prompt port. Resume-after-pause restamps — "loop start" means this
  execution, not the original turn submission.
- User timezone source: threaded from Reborn composition when available
  (settings/run profile); `None` otherwise. The slice does not invent a new
  settings surface — if no existing source exists, composition passes `None`
  and the tz plumbing is a follow-up.
- `SubagentLoopPromptPort` composes requests and delegates to the inner port,
  so child runs get a runtime section stamped at child spawn. No
  flavor-specific narrowing in this slice (parent plan owns that).

### Safety

- Content is static format + timestamp + IANA name: passes
  `validate_model_safe_text`. No host paths, env vars, secrets.
- IANA timezone is carried as `chrono_tz::Tz` — invalid names are rejected at
  the producer boundary at parse time, by construction; no runtime fallback needed.

## Testing

- `instruction_bundle.rs` unit tests:
  - `Some(runtime_context)` renders the `runtime` section, correct position,
    deterministic fingerprint for equal input.
  - `None` produces a bundle identical to a pre-change bundle (no section, no
    fingerprint fields).
  - Timezone-unknown branch renders the fallback sentence.
  - Invalid IANA names are rejected at the producer boundary (no runtime fallback test needed; the type prevents construction).
- `prompt.rs` (`HostManagedLoopPromptPort`) test: `with_runtime_context`
  attaches the section to built bundles.
- Caller-path test in `ironclaw_reborn` model gateway tests: the final model
  request for a real loop contains the runtime section (parent plan's
  requirement that context is proven through the caller path, not just the
  helper — see `.claude/rules/testing.md`).

## Out of scope (owned by the parent plan)

- Capability posture, scoped-path semantics, network/process placement
- Subagent flavor narrowing and parent-boundary disclosure
- Redaction policy beyond existing `validate_model_safe_text`
- Per-call or per-minute time refresh
