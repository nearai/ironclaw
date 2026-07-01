# Scheduled-trigger runs cannot create triggers (#5505)

- **Issue:** #5505 `[QA] Routine creation prompt is embedded inside the created routine`
- **Date:** 2026-07-01
- **Scope:** Reborn only. No legacy v1/v2.
- **Status:** Plan — pending review.

## Problem

Ask Reborn "every 30 minutes email me a summary of my next meeting" and the
created routine's stored prompt is self-referential: it contains
`"Create a routine that…"` meta-instructions instead of the per-run action
(`"Check calendar, summarize, send email"`). When that routine fires, the
model re-runs creation logic → a routine that creates routines ("routine
inside a routine"). Each fire re-triggers creation instead of doing the task.

## Root cause (two layers)

1. **Mechanism:** A scheduled trigger fire runs through the *same* agent loop
   as an interactive chat turn. `builtin.trigger_create` is in the model-visible
   tool surface during a triggered run, so a self-referential prompt can (and
   does) re-invoke creation. Confirmed: trigger fire and interactive turn both
   resolve the interactive-default run profile; nothing strips trigger-mutator
   tools for a fire.
2. **Generation:** `trigger_create`'s `prompt` parameter description
   (`crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs:477`) tells
   the model *what* the field is but not to transform the user's scheduling
   request into direct action steps. The model copies the user's
   "create a routine that…" phrasing verbatim into the field.

## Chosen design — Option B: dedicated run profile + profile-keyed host deny-map

A triggered run differs from an interactive turn in **exactly one way**: its
tool set is missing the trigger mutators. It must otherwise run identically
(same planner, budget, context, model). That is the *capability-surface* axis,
not the *loop-family* axis.

- Give trigger fires a dedicated **run profile** whose
  `capability_surface_profile_id = "scheduled_trigger"`. The profile **reuses
  the existing default planned loop driver / family** — no new family, driver,
  or replay digest.
- Generalize the existing host-level `DisabledCapabilitiesDecorator`
  (`crates/ironclaw_reborn/src/runtime.rs:714-740`) from a single global
  deny-list into a **global list + per-surface-profile deny-map** keyed on
  `run_context.resolved_run_profile.capability_surface_profile_id`. For the
  `scheduled_trigger` surface, deny the trigger mutators.
- Extend the `trigger_create.prompt` schema description with a negative
  constraint (generation-time defense).

Enforcement uses `CapabilitySurfaceDenyFilter`, the one mechanism with native
subtractive ("all-except") semantics — new tools flow to trigger runs
automatically; only the named mutators are removed. This also activates
`capability_surface_profile_id`, which is inert today (read only by the
fingerprint hash), and is precisely the field meant to select a per-profile
tool surface.

### Why not a dedicated loop family

A loop family governs loop *behavior* (planner, budget, iteration/wall-clock,
context, checkpoint). Trigger fires need none of that changed. A family also
costs a new `LoopFamilyId` + family factory + `CapabilityStrategy` struct +
blake3 digest + new driver id + driver registration + profile + readiness
snapshot across ~7 files / 2 crates — ~250 prod LOC to remove 3 tools, with
~10 duplicated default strategy slots. Reserve a family for when triggered
runs need different loop *behavior*; promote later if that day comes (YAGNI).

## Non-goals

- No new loop family / driver / family digest.
- No semantic prompt-content validation (rejected `trigger_create`-in-prompt
  heuristic — issue Q3, declined).
- No change to interactive turns, subagent runs, or the subagent capability
  resolver.
- No touch to legacy v1/v2 routine code.

## Edits

### Edit 0 — shared typed profile-id contract (layering)

`crates/ironclaw_conversations` must name the profile it requests, but the
profile is *defined* in `crates/ironclaw_reborn`. Do not hardcode a literal in
conversations, and do not introduce a stringly `pub const` — the codebase
already has the typed convention: `RunProfileId::interactive_default()` /
`RunProfileId::long_running_mission()` (`crates/ironclaw_turns/src/run_profile/resolver.rs:76,85`).
Add a sibling **typed assoc constructor** in `ironclaw_turns`:

```rust
// crates/ironclaw_turns/src/ids.rs — inside the existing `impl RunProfileId`
// block (~line 294-310), next to interactive_default() / long_running_mission().
// NOT resolver.rs (those lines are call sites, not the impl).
impl RunProfileId {
    pub fn scheduled_trigger() -> Self { /* mirror interactive_default() */ }
}
```

This is the single source of truth for the id. Both the conversations request
(Edit 1) and the reborn registration (Edit 2) reference
`RunProfileId::scheduled_trigger()`.

The `capability_surface_profile_id` value stays a plain
`CapabilitySurfaceProfileId::new("scheduled_trigger")` constructed at its use
sites — consistent with how `"interactive_tools"`/`"subagent_tools"` are built
today (no assoc-ctor convention exists for that type). Keep the string in one
`const SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID: &str = "scheduled_trigger"`
in `planned_driver_factory.rs` so Edit 2 (registration) and Edit 3 (deny lookup)
agree.

### Edit 1 — request the profile at the trigger boundary

`crates/ironclaw_conversations/src/inbound.rs:401`,
`trusted_inbound_request_from_trigger`. Field is already threaded end-to-end
(`InboundTurnRequest.requested_run_profile` → `AcceptInboundMessageRequest` →
`AcceptedInboundMessage` → `SubmitTurnRequest` → `RunProfileResolutionRequest`
→ `resolve_run_profile`); today it is hardcoded `None`.

```rust
requested_run_profile: Some(
    RunProfileRequest::new(RunProfileId::scheduled_trigger().as_str())
        .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })?,
),
```

Plus add `RunProfileRequest` and `RunProfileId` to the `ironclaw_turns::{…}`
import (inbound.rs:11). ~5 LOC. This is a conversations-layer statement of fact
— a trusted trigger fire (already stamped `TrustedInboundKind::Trigger` here)
runs under the scheduled-trigger profile. (`RunProfileRequest` and
`RunProfileId` are distinct bounded strings; the resolver matches the request
string against the registered `profile_id`, so both must derive from the same
`scheduled_trigger()` source.)

### Edit 2 — register the `scheduled_trigger` run profile (+ dedup the scaffold)

`crates/ironclaw_reborn/src/planned_driver_factory.rs`.
`planned_default_profile_definition()` and `subagent_planned_profile_definition()`
(lines 217-254) are near-clones — the field clone is already handled by
`RunProfileDefinition::interactive_like(...)` (resolver.rs:209-224), but the
~15-line orchestration/`map_err` scaffold is copy-pasted twice. Adding a third
copy for `scheduled_trigger` is the trigger point to extract a helper (code-judo:
net-negative LOC even after adding the profile):

```rust
fn planned_like_profile_definition(
    profile_id: RunProfileId,
    descriptor: AgentLoopDriverDescriptor,
    capability_surface_profile_id: &str,
) -> Result<RunProfileDefinition, RunProfileRegistryError> { … }
```

The helper **wraps** the existing 5-arg `RunProfileDefinition::interactive_like`
(resolver.rs:209-224), hardcoding the two checkpoint args
(`planned_driver_checkpoint_schema_id()` / `..._version()`) that all three
callers already pass identically, plus the shared `.map_err(… InvalidProfile
…)` scaffold. Route all three (default, subagent, scheduled_trigger) through it. The
`scheduled_trigger` profile:

- `profile_id = RunProfileId::scheduled_trigger()`.
- `descriptor = planned_driver_descriptor()?` (`planned_driver_factory.rs:114`)
  — **reuse the default driver** (same family). Do NOT add a new driver.
- `capability_surface_profile_id = SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID`.

Register it in `default_planned_run_profile_resolver()` (lines 268-279)
alongside default + subagent. Required: without this, Edit 1 makes trigger
fires fail resolution with `ProfileUnavailable`. ~10-15 net LOC (helper offsets
the third clone).

### Edit 3 — generalize the host deny-decorator to a per-profile deny-map

`crates/ironclaw_reborn/src/runtime.rs:714-740`.

Precompute the deny vecs at construction (fields, not a `HashMap` — only one
per-profile entry exists; a `match` is equally extensible and needs no runtime
hash/alloc, matching this plan's own YAGNI stance). `decorate()` returns
`Arc<…>` (not `Result`), so the fallible `CapabilityId::new` calls happen once
at construction, never inside `decorate()`.

```rust
struct DisabledCapabilitiesDecorator {
    global_denied: Vec<CapabilityId>,           // spawn_subagent (all profiles)
    scheduled_trigger_denied: Vec<CapabilityId>, // trigger mutators (scheduled_trigger surface only)
}
// scheduled_trigger_denied built once from
//   [TRIGGER_CREATE, TRIGGER_REMOVE, TRIGGER_PAUSE, TRIGGER_RESUME]
//   (read-only TRIGGER_LIST stays available)

impl LoopCapabilityPortDecorator for DisabledCapabilitiesDecorator {
    fn decorate(&self, run_context: &LoopRunContext, inner: Arc<dyn LoopCapabilityPort>)
        -> Arc<dyn LoopCapabilityPort> {
        let mut denied = self.global_denied.clone();
        match run_context.resolved_run_profile.capability_surface_profile_id.as_str() {
            SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID =>
                denied.extend(self.scheduled_trigger_denied.iter().cloned()),
            _ => {}
        }
        Arc::new(CapabilitySurfaceDenyFilter::new(inner, denied))
    }
}
```

Wire the deny vecs at construction (runtime.rs:610-621). `decorate()` already
receives `LoopRunContext` (currently `_run_context`, unused). ~35 LOC. Promote
to a `HashMap`/table only when a second per-profile entry actually lands.

**Construction-guard fix (footgun):** today the decorator is only added
`if !disabled.is_empty()` (runtime.rs:618-621) — that guard exists because
`DISABLED_CAPABILITY_IDS` is documented as emptyable to re-enable
`spawn_subagent`. Once the decorator also carries `scheduled_trigger_denied`,
that guard would silently drop the trigger-mutator deny whenever the global
list is emptied. **Always construct the decorator** (or guard on
`!global_denied.is_empty() || !scheduled_trigger_denied.is_empty()`) so an
unrelated spawn_subagent toggle can never re-enable `trigger_create` for
scheduled fires. Add a regression test pinning this: empty global list →
`scheduled_trigger` surface still excludes the mutators.

Composition confirmed: the executor applies the family `CapabilityStrategy`
(default = All here) on top of the host surface, and the host surface has
already run this deny-filter — all layers are `retain`-based intersections, so
the mutators are excluded from the final model-visible surface.

### Edit 4 — generation-time guidance

`crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs:477`. Extend the
`prompt` description, mirroring the existing delivery-routing negative
constraint:

> "Write only the action to perform when the trigger fires — direct imperative
> steps (e.g. 'Check the calendar for the next meeting, summarize it, email the
> summary'). Do not describe creating, scheduling, or configuring the trigger
> itself; rewrite the user's scheduling request into the run-time action."

~4 LOC.

## Trigger-mutator capability id set

Confirmed full set (`trigger_management.rs:32-36`): `TRIGGER_CREATE_CAPABILITY_ID`
=`builtin.trigger_create`, `TRIGGER_LIST_CAPABILITY_ID`=`builtin.trigger_list`,
`TRIGGER_REMOVE_CAPABILITY_ID`=`builtin.trigger_remove`,
`TRIGGER_PAUSE_CAPABILITY_ID`=`builtin.trigger_pause`,
`TRIGGER_RESUME_CAPABILITY_ID`=`builtin.trigger_resume`. **No** `update`/`delete`
capability exists.

Deny the **mutators** for the `scheduled_trigger` surface:
`trigger_create`, `trigger_remove`, `trigger_pause`, `trigger_resume` — a
scheduled fire must not manage the trigger fleet (creating is the reported bug;
remove/pause/resume are the same "a fire mutates triggers" class and are
excluded together). Keep the read-only `trigger_list` available. Import these
consts from `ironclaw_host_runtime::first_party_tools` (re-exported at
`first_party_tools/mod.rs:71-72`) rather than re-typing the string literals.

## Test plan (test-first; test through the caller)

Red before green. Drive the caller, not just the helper.

1. **Host/decorator (integration):** a run whose resolved profile has
   `capability_surface_profile_id = "scheduled_trigger"` yields a model-visible
   capability surface that **excludes** `builtin.trigger_create` (+ other
   mutators); a run with `interactive_tools` **includes** it, and
   `builtin.trigger_list` remains present in both. Drive through the composed
   capability port / host `visible_capabilities`, not `decorate()` in isolation.
2. **Profile resolution:** requesting `SCHEDULED_TRIGGER_RUN_PROFILE_ID` resolves
   a profile carrying the `scheduled_trigger` surface id (not `ProfileUnavailable`).
3. **Trigger boundary:** `trusted_inbound_request_from_trigger` sets
   `requested_run_profile = Some("scheduled_trigger")` and it survives to the
   `SubmitTurnRequest` (assert via the recording coordinator already used in
   `inbound.rs` tests).
4. **Schema:** `trigger_create` schema `prompt` description contains the new
   negative-constraint text (cheap guard against silent revert).

## Risks / open questions

- **O1 — resolved.** Profile id = typed `RunProfileId::scheduled_trigger()` in
  `ironclaw_turns::run_profile` (both crates already depend on it). Surface-id
  string const lives in `planned_driver_factory.rs`.
- **O2 — idempotent replay.** The conversations guardrail requires reusing the
  *original run-profile request* until submitted. Since the value is now set at
  the single accept point and threaded on `AcceptedInboundMessage`, redelivery
  replays the same `scheduled_trigger` request — verify no path resets it to
  `None`.
- **O3 — profile-id naming divergence.** Existing reborn profiles are
  `reborn-planned-*`; the new one is the neutral `scheduled_trigger` because it
  is a cross-crate contract. Justified, but call out in review.
- **O4 — existing malformed routines.** This fixes *fire-time* behavior for all
  existing routines immediately (they resolve the new profile at fire). No data
  migration. Edit 4 only improves *newly created* prompts.

## Rollback

Set `inbound.rs` request back to `None` (trigger fires revert to interactive
profile) and/or empty the `scheduled_trigger` entry in the per-profile deny-map.
Profile registration and the schema text are inert without Edit 1.

## Sequencing for implementation

Edits are near-independent but resolution will fail if Edit 1 lands without
Edit 2. Land order: Edit 0 → Edit 2 (register) → Edit 3 (deny-map) → Edit 1
(request) → Edit 4 (schema). Tests authored first per edit.
