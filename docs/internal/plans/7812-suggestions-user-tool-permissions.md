# Plan: suggestion generation over the user's no-approval, read-only tools

Issue: nearai/ironclaw#7812. Revision 2 — incorporates the approach audit
(issue comment `approach-audit:v1 issue:7812`) and the follow-up verification
that refuted one of its findings.

## Requirement

Suggestion generation must run with the user's **active connected tools**,
narrowed to those that (a) would raise **no approval prompt** under that user's
own permission settings, and (b) are **read-only**.

## What shipped: approval-scoped only, read-only by prompt

**Product decision (owner: Henry Park).** The autonomous run reaches every
capability the user has auto-approved — built-in, extension, or MCP alike — and
read/list-only behaviour is carried by the prompt, not by the surface.

`require_no_approval` therefore does exactly one thing: drop capabilities whose
authorization resolves to `RequireApproval`, because an approval gate with
nobody to answer it parks the run rather than asking it anything.

### The stricter posture that was considered and rejected

An earlier revision of this plan also narrowed `allowed_effects` to the non-write
set. That was **deliberately dropped**. The rationale for it is preserved here
because the residual risk is real and a future reader deserves it:

- The approval hard floor is only `Financial | ModifyApproval | ModifyBudget`
  (`profile_gate_policy.rs:70-75`). Slack's post capabilities declare
  `external_write` (`packages/slack/manifest.toml:186`), which is not on that
  floor, so they fall through to global auto-approve (default ON) and resolve to
  `Allow`.
- Measured consequence: the shipped surface includes `builtin.shell`,
  `builtin.write_file`, `builtin.apply_patch`, `builtin.outbound_deliver`,
  `builtin.extension_install`, `builtin.extension_remove`, and
  `ironclaw.memory.write`. The prompt is the only thing restraining them.

**Why it was dropped anyway:** the decision-maker's position is that an
autonomous run should reach whatever the user has auto-approved, and that
effect-based restriction is the wrong lever because effect declarations vary in
quality across runtimes. That second point is partly true — see the synthetic
gap below — though MCP is in fact the best-covered lane
(`hosted_mcp_discovery.rs:240-253` synthesizes `ExternalWrite` from
`destructiveHint`/`sideEffectsHint` and makes unannotated tools inherit the
provider's write-capability).

**If the posture is revisited**, `CapabilitySurfacePolicy` still has the seam:
add a `without_write_effects()` builder beside `without_approval_gated()` and
one line in `create_capability_port`. The integration test pins the full surface,
so the change would show up immediately as a diff in that expectation.

## Where enforcement actually happens (verified, incl. adversarial check)

Precise account — an earlier draft overstated this twice, so it is spelled out:

1. **Listing.** `host_runtime/src/surface.rs:148-155` filters candidates by
   `policy.allows_effects(&descriptor.effects)`; `:205-215` drops
   `Decision::RequireApproval` when `include_requires_approval == false`.
2. **Staging.** During `stream_model`, `loop_host/src/lib.rs:1834-1843` wraps the
   port in `CapabilitySurfaceVisibleFilter(visible_capability_ids)`, which gates
   `register_provider_tool_call` / `validate_provider_tool_call`. A capability
   the policy excluded never becomes a `CapabilityCallCandidate`.
3. **Dispatch.** `executor/capabilities.rs:237` -> `loop_driver_host.rs:2538-2545`
   -> the **durable** port, wrapped by `CapabilitySurfacePolicyFilter`
   (`capability_surface_filter.rs:54-56`), which checks
   `policy.permits_capability_id` **only** — not effects, not approval.

So effects/approval are enforced at listing+staging, NOT at the terminal
dispatch. That is sufficient here because every dispatch in this run originates
from a staged model tool call, and the resume paths that skip staging
(`prompt.rs:265-305`) cannot carry an excluded capability: with
`include_requires_approval=false` no approval-gated tool is ever offered, so no
such resume can be pending.

**Accepted trade-off (defense in depth).** Today `suggestions.rs` declares four
ids, so `policy.capability_ids` narrows dispatch too — two gates. Declaring no
tools leaves `capability_ids` at `AllExcept(UNBOUND_DENIED_CAPABILITY_IDS)`, so
dispatch is gated once. The remaining gate is the one derived from the real
authorizer, and the unbound denies still apply, but this IS a reduction and a
reviewer should weigh it. Upgrade path if we want it back: attach a durable
visible-id filter at port construction (the type already exists and is used
transiently). Marked in code with `ponytail:`.

**The flag is not exactly "the user's no-ask set".** `visible_capabilities` also
excludes on `is_model_visible`, `allows_runtime`, `plan_capability(...)`,
missing `provider_trust`, and a bare `Decision::Deny` (no grant). All are
fail-closed and none of those tools were callable anyway, but the set is
narrower than "tools the user marked no-ask" and should be described that way.

## Why this shape (forced seam, not analogy)

`unbound_turn.rs:215` sets `product_context: None`. The unbound lane therefore
carries **no `TurnExecutionPolicy`** — the multi-caller narrowing bag used by
triggers and conversations at `runtime.rs:1006-1013`. Prepared-turn declarations
are the **only** per-run channel this lane has. That is the forced seam.

(Revision 1 justified the placement by analogy to `TurnLimits`. That analogy is
false: `TurnLimits` has zero production setters — both real constructors pass
`limits: Default::default()` — and it is enforced in a different crate by
`DeclaredLimitsNarrowingResolver`. The forced-seam argument above is the real
justification and also answers "only one caller sets it".)

## Rejected alternatives

- **Suggestion-scoped surface profile.** Unreachable: `accept_and_submit`
  hardcodes `requested_run_profile: None` (`unbound_turn.rs:200`) and
  `coordinator.rs:377-386` rejects any non-`unbound_default` hint. Forcing it
  needs a non-unbound profile, which loses `UNBOUND_DENIED_CAPABILITY_IDS`
  ("a background run minting more background work is the runaway class this lane
  must not open", `runtime.rs:966-970`). #7498 built this and was closed
  unmerged 2026-08-17; #7694 landed the declarations route the next day.
- **Product-side computed allowlist.** `effective_tool_permission`
  (`reborn_services.rs:1864`) is a Settings-UI projection lacking one-shot
  leases, the origin-gate matrix, and hard floors that `profile_gate.rs:268-427`
  applies — a third copy of the approval decision.
- **`AuthorityCeiling.allowed_effects`** (proposed by the audit). Infeasible as
  a per-run control: the ceiling is **per-provider**, built as
  `effects_by_provider` from each extension's manifest
  (`extension_host/src/capability_surface.rs:118-145`) and carried in a
  `TrustDecision` keyed by `ExtensionId`.
- **Prompt-only guardrails** (issue body's original line). See "Why both
  narrowings" — prompt-only would expose write tools, and #7786 already recorded
  that "a prompt rule is unenforceable server-side".

## Implementation

### 1. `crates/contracts/ironclaw_host_api/src/prepared_context.rs`

One `bool` on `PreparedTurnDeclarations` — no new type:

```rust
    /// No human is present for this run: narrow the surface to capabilities
    /// that need no approval AND declare no write effect. Narrowing-only;
    /// `false` (the default) changes nothing, so persisted declarations and
    /// other unbound callers are unaffected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_no_approval: bool,
```

A single flag rather than two: the only combination anything needs is "both",
and the two dangerous combinations ("writes allowed, approvals skipped") should
not be expressible. Intent lives in the contract, policy in the runtime — if the
definition of unattended-safe changes, callers do not.

One consuming builder on `CapabilitySurfacePolicy` (matching the
`deny_capability_ids` / `narrow_to_capability_ids` idiom every neighbouring
narrowing step in `create_capability_port` uses):

```rust
    pub fn without_approval_gated(mut self) -> Self {
        self.include_requires_approval = false;
        self
    }

```

Also correct the `tools` doc comment (`:59-61`): it says "Empty means *no
tools*" but `runtime.rs:1051` implements empty as "keep the profile's default
surface". Code is the shipped behaviour; the comment is wrong.

### 2. `crates/loop/ironclaw_turn_runner/src/runtime.rs:1046-1058`

Declarations are already read here (`:1037`):

```rust
if let Some(declarations) = declarations {
    if !declarations.tools.is_empty() { /* unchanged intersection */ }
    if declarations.require_no_approval {
        // ponytail: enforced at listing+staging, not at the terminal
        // invoke_capability (which gates on capability_ids only). Sufficient
        // because every dispatch here originates from a staged model tool
        // call. Upgrade path: attach CapabilitySurfaceVisibleFilter durably
        // at port construction.
        policy = policy.without_approval_gated();
    }
}
```

`UNBOUND_DENIED_CAPABILITY_IDS` still subtracts afterwards.

### 3. Callers

- `unbound_turn.rs`: `UnboundTurnSubmission` gains `require_no_approval`, passed into
  `PreparedTurnDeclarations` at `:174`. `requested_run_profile` stays `None`.
- `suggestions.rs`: delete `suggestion_tool_allowlist()` (`:403-415`), declare
  no tools, set `require_no_approval: true`.
- `openai_compat_serve.rs:287-303`: add `require_no_approval: false`.

### 4. Compile sites for the new field (complete list)

1. `prepared_context.rs:58` (struct def)
2. `prepared_context.rs:131` (round-trip test)
3. `unbound_turn.rs:173` (production)
4. `unbound_turn.rs:470` (in-crate test)
5. `ironclaw_threads/tests/session_thread_contract.rs:4598`
6. `ironclaw_threads/tests/filesystem_session_thread_contract.rs:7486`
7. `tests/integration/unbound_turns.rs:100` (`structured_declarations()`)

(Enumerated with `..Default::default()` fallbacks excluded, across BOTH
`crates/` and `tests/` — revision 2 searched only `crates/` and missed #7.)

### 5. Prompt

`prompts/suggestion_generation.md` — sharpen in place (#7786 convention). The
model now has real read tools; tell it to ground suggestions in what it reads.
Read-only is enforced by the surface, so the prompt states intent, not the
guardrail.

## Known limitation: synthetic capabilities are exempt from surface narrowing

Found by measurement, and it is broader than an effects-declaration gap.

`SyntheticCapabilityPort::visible_capabilities`
(`crates/loop/ironclaw_loop_host/src/synthetic_capability.rs:472-506`) delegates
to the inner port — which is where `host_runtime::surface` applies
`allows_effects`, `include_requires_approval`, runtime, trust, and id filtering
— and then appends every synthetic descriptor unconditionally:

```rust
surface.descriptors.extend(synthetic_descriptors);
```

The outer `apply_capability_surface_policy` (`runtime.rs:1088`) re-filters the
surface, but `apply_policy_filter_to_surface`
(`capability_surface_filter.rs:388-399`) retains on capability **ID** only.

**Consequence: `require_no_approval` cannot gate a synthetic capability.** Only
an ID allow/deny list can. This is visible in
`disabling_global_auto_approve_shrinks_the_autonomous_surface`: with nothing
auto-approved, the ONLY capabilities left are synthetics —
`project_create`, `skill_activate`, `notification_channels_set`,
`capability_info`, `result_read`, `outbound_delivery_targets_list`,
`trace_commons.onboard`, `memory.profile_set`.

Three of those mutate state. `notification_channels_set` already hand-rolls its
own approval dance rather than declaring an effect, which is the same gap seen
from the other side.

An earlier draft of this plan attributed this to `allows_effects(&[])` being
vacuously true for an empty effect set. That was wrong: synthetics never reach
that check. The real cause is the unconditional append.

Not fixed here — a real fix means either declaring effects on synthetic
descriptors AND routing them through the surface policy, or filtering the
appended set — both of which change classification for every run. Contained
meanwhile by pinning the whole surface in the integration test.

## Test strategy

Integration tier, extending `tests/integration/suggestions.rs`.

1. **Red first**: `:354-368` asserts the exact four declared ids → flip to
   assert empty `tools` + `require_no_approval: true`.
2. **Behavioural**: `:575-591` (`captured_tool_definitions()` — what the model
   actually saw). Extend the fixture with a connected extension exposing
   (a) always-allow read, (b) ask-each-time read, (c) always-allow write.
   Assert only (a) reaches the model.
3. `cargo test -p ironclaw_architecture_tests` (contract field).

## Commit labelling

- **Structural** (no behaviour change; permissive default): contract field +
  doc-comment correction + `unbound_turn` passthrough + `openai_compat` default
  + the four compile-site fixups.
- **Behavioural**: `runtime.rs` application, `suggestions.rs` allowlist
  deletion, prompt, and the tests pinning them.
