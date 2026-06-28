# Reborn Self-Learning Loop: Next PR Plan

Date: 2026-06-23

Status: planning note for the next implementation PR after
[#5163](https://github.com/nearai/ironclaw/pull/5163).

Primary inputs:

- PR #5163: `Reborn memory placement: MemoryService facade + agnostic/native crate split`
- PR #5156: `feat(skill-learning): any-backend distillation, approval gate, learned-only scoping, persisted switches`
- Issue #3537: `Reborn: model memory as a userland extension`
- Illia research note attached in Codex: `IronClaw Reborn - Self-Improvement & Reliability Implementation Plan`
- Illia security note: memory items need source references for permission inheritance/revocation, and default TTLs should be model-suggested and host-clamped.

## Executive Summary

PR #5163 gives the self-learning work the right seam: memory calls now go
through a provider-neutral `MemoryService` contract, while native filesystem
behavior lives behind `ironclaw_memory_native`. That means the self-learning
loop should not talk to native memory internals or legacy workspace memory. It
should be a Reborn subsystem that consumes host-mediated services and can later
bind to native memory, Honcho, mem0, or another provider through capability
profiles.

The next PR should not try to ship the whole research plan. It should land the
safe, scoped spine that all later learning depends on:

1. provenance and TTL contract fields for memory/learning artifacts;
2. a new `ironclaw_reborn_learning` domain crate with scoped learned-failure and
   learned-rule records;
3. prompt injection for accepted learned rules through the existing identity
   context path, behind a feature/config flag;
4. a milestone observer that can capture failed loop outcomes and enqueue a
   candidate learning record without autonomously promoting it yet;
5. verifier interfaces and test doubles, so live verification can be added as a
   narrow follow-up without changing the store model.

This creates the first real "never twice" substrate while avoiding the two
dangerous shortcuts: unscoped global learned state and unverified prompt/memory
writes.

Additional product context from Slack changes the posture for skill-learning
work: users should not be asked to approve routine self-learning forever, but
the system also should not silently activate new prompt/skill behavior before
verification, provenance, TTL, and rollback exist. The intended end state is
"automate everything safe"; the near-term bridge is explicit staging plus a
clear path to verifier-gated auto-promotion.

## What PR #5163 Changes

PR #5163 is a placement/lift PR, not the self-learning implementation. The
important facts to build on are:

- `ironclaw_memory` becomes the provider-neutral contract crate. It owns
  `MemoryService`, operation DTOs, memory scope/path/context value types,
  prompt-safety vocabulary, and audit/event contracts.
- `ironclaw_memory_native` becomes the native provider. It owns the filesystem
  repository/backend, chunking/search/indexing, `/memory` adapter, prompt-write
  safety enforcement, and `NativeMemoryService`.
- Host runtime memory dispatch and prompt context retrieval route through
  `MemoryService`, so learning code should depend on the facade, not on
  provider internals.
- Memory profile binding vocabulary exists in host runtime:
  `memory.context_retrieval.v1`, `memory.interaction_log.v1`,
  `memory.document_store.v1`, and optional `memory.semantic_search.v1`.
- Extension/manifest work is moving toward host-defined memory profiles and
  host ports, but PR #5163 deliberately does not register
  `ironclaw.memory.native` as a full extension manifest or create SQL-backed
  `reborn_memory_*` tables.

Before stacking work on #5163, settle the current review nits:

- `NativeMemoryService::retrieve_context` must filter protected documents the
  same way `search` and `tree` do.
- The absolute-path redaction change should have a regression test.
- The persisted metadata fallback warning should be lowered to debug if still
  present on the branch tip.

## Product Context From Slack And PR #5156

Sergey flagged PR #5156 as a product-decision point, not only an engineering
PR. The external contributor's branch introduces self-learned skills with a
human approval gate: learned skills are saved inactive and pending review until
approved, with persisted switches for extraction, review requirement, and
auto-activation. The branch also describes self-learning as active by default
where wired, while keeping `require_review` on by default.

The product tension is real:

- The desired UX is "automate everything for me"; routine learning should not
  become a chore for the user.
- The current safety requirement is that newly learned prompt/skill behavior is
  trusted code/prompt surface. It cannot be silently activated until the system
  can prove scope, source authority, expiry, rollback, and expected behavioral
  improvement.
- Internal product/architecture should own the default posture. External PRs can
  contribute mechanics, but should not set irreversible defaults such as
  active-by-default extraction or broad auto-activation without product signoff.

Recommended product feedback on #5156:

1. Keep `require_review` on as the temporary safety bridge for that PR.
2. Do not accept active-by-default extraction/learning as the long-term product
   posture until provenance, TTL, budget/accounting, and verifier-gated
   promotion exist. Prefer default-off or experimental-gated learning if #5156
   lands before the Reborn learning substrate.
3. Define the target UX as verifier-gated automation: low-risk learned skills
   auto-promote only after deterministic or live verification passes, with
   provenance/scope/TTL checks and rollback.
4. Keep explicit approval for high-risk learning: new broad capabilities,
   cross-scope/global rules, egress-expanding skills, code/config changes, or
   anything whose source audience cannot be proven usable in the current
   conversation.
5. Align with Sergey/Firat on Thursday, June 25, 2026 on default switches,
   review tiers, egress disclosure copy, and the path from "pending review" to
   "auto-promote verified low-risk changes."

For this next Reborn learning PR, treat #5156 as useful mechanics and prior art
for skill staging, origin metadata, settings, and UI affordances. Do not inherit
its defaults blindly. The plan below keeps the first Reborn slice default-off
and focused on the substrate that makes automation safe.

## Research Constraints To Preserve

Illia's research note has one load-bearing distinction:

- Recorded replay is a regression lock, not proof that a prompt/memory fix
  changed model behavior. Replay reuses canned responses.
- Prompt and memory fixes require a budgeted live model re-run to verify
  behavior.
- Executable skill fixes can be verified by deterministic execution.
- Learned rules/memory must be actively curated and bounded. Unbounded memory
  often hurts agent performance.
- Every learning must be scoped from day one. Scope widening is dangerous;
  narrowing is safe.

This means the first implementation PR should build the trustworthy substrate
and deterministic coverage, then add live verification as an explicit,
budget-governed follow-up.

## Security Model: Source, Scope, TTL

Every memory item and learning artifact must carry source attribution and an
expiry policy. This is not metadata decoration; it is the permission model.

### Source Reference

Add a provider-neutral source/provenance vocabulary shared by memory metadata
and learned records. Suggested shape:

```rust
pub struct SourceProvenance {
    pub source_ref: SourceRef,
    pub extracted_at: Timestamp,
    pub content_digest: String,
    pub source_scope: ResourceScope,
    pub source_audience: SourceAudience,
    pub revocation_ref: Option<RevocationRef>,
    pub extractor_version: String,
}

pub enum SourceRef {
    TurnRun { run_id: TurnRunId },
    ConversationMessage { thread_id: ThreadId, message_ref: String },
    MemoryDocument { path_hash: String, version_hash: Option<String> },
    CapabilityOutput { invocation_id: InvocationId, capability_id: CapabilityId },
    ExternalDocument { provider: String, stable_id: String, version: Option<String> },
}
```

For the first PR, `TurnRun`, `ConversationMessage`, and `MemoryDocument` are
enough. External document audience/revocation can be a later implementation
behind the same enum.

### Permission Inheritance

At use time, not just extraction time, the host checks:

- the current run scope is equal to or narrower than the stored use scope;
- the conversation participants/caller audience is a subset of the source
  audience when the source has one;
- the source has not expired or been revoked;
- the source version/digest still matches when the source supports stable
  version checks.

If an authorizer cannot prove the item is usable, it fails closed by omitting
the memory/rule from context. Providers may store and index the provenance, but
the host-owned prompt/context adapter must be able to enforce the final use
decision so a provider cannot become the sole permission boundary.

### TTL

Add a TTL contract with host clamping:

```rust
pub struct RetentionPolicy {
    pub extracted_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub retention_class: RetentionClass,
    pub model_suggested_ttl_seconds: Option<u64>,
    pub rationale: Option<String>,
}

pub enum RetentionClass {
    Ephemeral,
    Session,
    Project,
    Durable,
    NeverExpires,
}
```

The extractor can suggest TTLs, but host policy clamps them by kind:

- "user is visiting London" -> likely `Ephemeral`, about 2 weeks;
- "user has two children" -> `NeverExpires` or long-lived, but still subject
  to source revocation and user deletion;
- repo/project conventions -> `Project`, expiring or revalidated when the
  source file/repo context changes;
- failure signatures and verified learned rules -> durable while useful, with
  evidence-based retirement instead of time-only deletion.

Expired items are not injected. They can remain in storage for audit/history
until a separate retention policy archives or deletes them.

## Recommended Next PR

Title sketch:

```text
feat(reborn-learning): add scoped learned-rule substrate with provenance and TTL
```

### Goal

Land the minimal, safe vertical slice:

```text
failed/completed run metadata
  -> learning coordinator observes milestone
  -> scoped learned failure/rule store
  -> verified-or-seeded learned rule
  -> prompt injection through identity context
  -> hermetic tests prove scope, TTL, provenance, and injection behavior
```

Do not make the PR auto-promote arbitrary model-generated rules. The PR should
create the rails that make auto-promotion safe in the next slice.

### New Crate

Create `crates/ironclaw_reborn_learning`.

Suggested files:

- `src/lib.rs`: public exports and crate docs.
- `src/error.rs`: sanitized error taxonomy.
- `src/types.rs`: ids, status enums, failure signatures, rule records.
- `src/provenance.rs`: `SourceProvenance`, `SourceAudience`, revocation/use checks.
- `src/ttl.rs`: `RetentionPolicy`, expiry helpers, host clamp policy.
- `src/store.rs`: `LearningStore` trait and in-memory implementation.
- `src/filesystem_store.rs`: typed store over `RootFilesystem` for local/dev and
  backend parity through filesystem backends.
- `src/rules.rs`: rule selection, budget ordering, rendering.
- `src/identity_context.rs`: `LearnedRulesIdentityContextSource`.
- `src/coordinator.rs`: milestone observer/coordinator skeleton.
- `src/verifier.rs`: verifier traits and deterministic fake verifier for tests.

The crate may depend on neutral Reborn contracts such as `ironclaw_host_api`,
`ironclaw_turns`, `ironclaw_loop_support`, `ironclaw_events`, and
`ironclaw_filesystem`. It must not depend on the root `ironclaw` crate, legacy
`src/workspace`, native memory provider internals, product workflow, WebUI
handlers, or raw provider clients.

### Store Records

Use typed records, not `/memory` documents:

```rust
pub struct LearnedFailureRecord {
    pub signature: FailureSignature,
    pub scope: ResourceScope,
    pub source: SourceProvenance,
    pub first_seen_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub occurrence_count: u32,
    pub status: LearnedFailureStatus,
    pub candidate_fix: Option<LearningCandidateRef>,
}

pub struct LearnedRuleRecord {
    pub rule_id: LearnedRuleId,
    pub scope: ResourceScope,
    pub source: SourceProvenance,
    pub retention: RetentionPolicy,
    pub status: LearnedRuleStatus,
    pub rule_text: LoopSafeSummary,
    pub confidence_ppm: u32,
    pub hit_count: u64,
    pub miss_count: u64,
    pub verifier_evidence: Option<VerifierEvidenceRef>,
}
```

Status should distinguish at least:

- `Candidate`: observed/synthesized but not injected;
- `Active`: eligible for prompt injection;
- `Suppressed`: kept for audit but not used;
- `Expired`: no longer eligible because TTL elapsed;
- `Revoked`: no longer eligible because the source/access path revoked.

The store must support scoped lookups and dedupe:

- upsert/increment failure by `(scope owner axes, failure_signature)`;
- list active rules applicable to a run scope;
- mark rule status transitions with CAS or version checks;
- filter expired/revoked records before returning prompt candidates.

### Prompt Injection

Prefer the identity context path for the first PR. It is already part of the
prompt-building flow and gives content refs and budget handling.

Required wiring:

- Add a protected identity file name for learned rules, probably
  `LEARNED_RULES.md`, to the prompt-protected path registry.
- Add a stable class in `PromptProtectedPathClass::as_str`.
- Add tests that `IdentityFileName::new("LEARNED_RULES.md")` succeeds and
  unknown paths still fail.
- Add a generic `CompositeHostIdentityContextSource` in `ironclaw_loop_support`
  so learned rules can compose with `DefaultSystemPromptIdentitySource` instead
  of replacing it.
- Implement `LearnedRulesIdentityContextSource` in the learning crate. It loads
  active, unexpired, authorized rules for the current run scope and renders a
  compact trusted system message.

Rendering rules:

- keep the initial budget small, e.g. 1 KiB within the existing identity budget;
- order by specificity, confidence, recency, and source severity;
- never include source raw content, raw prompts, tool args, secrets, host paths,
  or external document text;
- include short rule text only, plus opaque rule ids if needed for debugging.

### Milestone Observation

The coordinator should wrap or tee the existing `LoopHostMilestoneSink` in
composition. On terminal milestones:

- `Failed`: compute a conservative `FailureSignature`, persist/increment
  `LearnedFailureRecord`, leave any fix as `Candidate`.
- `Completed`: optionally record success metadata for future skill/rule
  distillation, but no prompt write yet.

The signature should be intentionally coarse in the first PR:

```text
loop_failure_kind + last capability id/failure kind if available + run profile + safe surface family
```

It must not store raw messages or raw tool inputs in the signature. If more
detail is needed later, store a redacted `SourceRef` to the durable event or
thread/transcript owner, not the detail itself.

### Verifier Interfaces

Add traits now, wire fake implementations in tests, and keep live model
verification disabled until a follow-up:

```rust
#[async_trait]
pub trait LearningVerifier: Send + Sync {
    async fn verify_candidate(&self, candidate: LearningCandidate)
        -> Result<VerifierDecision, LearningError>;
}
```

The follow-up live verifier can call the existing QA recorder/runtime path, but
the store and coordinator should not need another redesign when that arrives.

### Memory Metadata Extension

On the #5163 branch, extend `ironclaw_memory::DocumentMetadata` with typed
optional fields:

```rust
pub provenance: Option<MemoryProvenance>,
pub retention: Option<RetentionPolicy>,
```

Keep unknown `extra` fields for forward compatibility. Native memory should
persist and inherit these fields with the existing metadata mechanics. First PR
tests should cover serialization, strict parse from tool/service input, and
expired metadata being excluded from context retrieval if context retrieval is
updated in this slice.

If the PR size gets tight, land the typed metadata and store records first, and
defer automatic extraction writes. Do not ship extraction without provenance
and TTL.

### Composition Wiring

Touch the Reborn composition root, not product/web layers:

- `Cargo.toml`: add `ironclaw_reborn_learning` workspace member.
- `crates/ironclaw_reborn_composition/src/runtime_input.rs`: add optional
  learning settings/overrides for tests if the current shape supports it.
- `crates/ironclaw_reborn_composition/src/runtime.rs`: compose the learning
  store, learned-rules identity source, composite identity source, and milestone
  observer when the feature/config flag is enabled.
- `crates/ironclaw_reborn_composition/src/factory.rs` or owning builder file:
  keep raw substrate private; expose facade-shaped handles only if tests need
  `#[cfg(test, feature = "test-support")]` accessors.

Feature/config gate:

```text
IRONCLAW_REBORN_LEARNING=1
```

or the existing Reborn config equivalent if the config layer has a better
feature gate pattern. Default off until live verification and curation land.

## Tests For The First PR

Narrow tests should carry most of the safety proof:

- `cargo test -p ironclaw_reborn_learning`
  - TTL excludes expired rules.
  - revocation/source authorization excludes unusable rules.
  - scope mismatch excludes wrong tenant/user/agent/project rules.
  - failure signature upsert increments instead of duplicating.
  - rule rendering stays within budget and omits source detail.
- `cargo test -p ironclaw_loop_support identity_context`
  - composite identity source preserves source order and resolution.
  - `LEARNED_RULES.md` is accepted as a protected identity file.
- `cargo test -p ironclaw_memory`
  - provenance/retention metadata round-trips.
  - strict untrusted metadata parse rejects malformed fields.
- `cargo test -p ironclaw_reborn_composition learning`
  - runtime wiring injects a seeded learned rule into the model request for
    the matching scope.
  - the same seeded rule does not appear for another tenant/user/agent/project.
  - terminal failed milestone writes/increments a scoped failure record through
    the production caller path, not just helper tests.
- `cargo test -p ironclaw_architecture`
  - new crate boundaries do not let learning depend on native memory internals,
    product/web layers, or the root crate.

Use request-capture assertions from `RebornTraceReplayModelGateway` where
possible to prove the actual prompt request changed.

## Follow-Up PR Ladder

### PR 2: Live Verification Runner

Lift the existing QA recorder path into a budgeted, background-callable verifier.
Prompt/memory fixes can become `Active` only after a live re-run proves the
captured failure no longer reproduces. Save accepted live runs as additional
`tests/fixtures/llm_traces/reborn_qa/*.json` fixtures and add Tier-2 contract
assertions.

### PR 3: Interaction Recording Memory Profile

Implement `memory.interaction_log.v1` through `MemoryService`/profile binding.
The host should record bounded, sanitized interaction summaries, not raw
transcripts. This is the clean path for self-learning to learn from user-facing
turns without reaching into thread internals.

### PR 4: Candidate Synthesis

Add an aux-model diagnostic pass that classifies root cause:

- prompt/rule;
- skill;
- memory fact;
- config/code proposal.

All outputs remain `Candidate` until a verifier promotes them. CODE/CONFIG
stays human-gated.

### PR 5: Skill Path

Use the existing `ironclaw_skill_learning` crate and lessons from PR #5156
instead of building skill distillation from scratch. Add staging and promotion
around it:

- distilled skill -> `candidate`;
- deterministic or live verification -> `active`, with auto-promotion allowed
  only for low-risk, same-scope changes;
- high-risk learned skills -> `pending_review`;
- failure rate regression -> demote back to `candidate`.

### PR 6: Curation And Stateful Eval

Add curation, retirement, and stateful gain measurement:

- consolidate rules under budget;
- archive stale/contradicted facts with supersession links;
- run stateful-vs-stateless eval sequences;
- add live canaries for high-value failure signatures.

### PR 7: Fleet Learning

Only after local scope/revocation/live-verification work is stable:

- telemetry-only failure signatures first;
- executable skills before free-text rules;
- opt-in publishing;
- de-identification;
- corroboration across deployments;
- mandatory local re-verification before activation.

## Non-Goals For The Next PR

- no closed-loop prompt optimizer;
- no automatic promotion based only on an LLM saying "fixed";
- no global learned state;
- no external/fleet sharing;
- no full Honcho/mem0 integration;
- no legacy `prompt_overlay` or legacy mission-loop port;
- no raw transcript, prompt, tool-input, host-path, or secret storage inside
  learning records;
- no default-on production behavior.

## Open Decisions

1. Whether `LearningStore` should initially use a typed `RootFilesystem`
   repository or land libSQL/Postgres repositories immediately. The first PR can
   use a typed filesystem-backed repository behind an off-by-default feature,
   but production defaulting should require explicit backend parity.
2. Whether memory provenance/TTL types live directly in `ironclaw_memory` or in
   a smaller neutral crate consumed by both memory and learning. Starting in
   `ironclaw_memory` is practical after #5163 because `DocumentMetadata` already
   lives there.
3. Whether learned rules should inject through identity context or a first-class
   prompt hook. Identity context is lower risk for the first PR because it
   already resolves message refs and participates in prompt budgeting.
4. What the first live verification budget should be. It should be per
   tenant/owner and disabled by default until operators opt in.
5. What default posture to take if PR #5156 lands before the Reborn learning
   substrate: keep learned-skill extraction default-off/experimental, or allow
   extraction while requiring review and preventing activation by default.
6. Which learning categories can eventually auto-promote without user approval
   once verifier, provenance, TTL, and rollback are in place, versus which
   categories remain human/admin-gated.

## Implementation Order

1. Settle #5163 review nits and rebase the next branch on that PR.
2. Add provenance/TTL contract types and tests.
3. Add `ironclaw_reborn_learning` records, store traits, in-memory store, and
   filesystem-backed typed store.
4. Add learned-rule selection/rendering with scope, source, and TTL filters.
5. Add `LEARNED_RULES.md` protected identity path and composite identity source.
6. Wire learned-rule injection in Reborn composition behind the feature flag.
7. Add milestone observer that records failed-run signatures through the store.
8. Add caller-level integration tests proving prompt injection and failure
   capture through real composition wiring.
9. Run focused tests plus architecture boundaries.
10. Before merging skill-learning work, post internal product feedback on PR
    #5156 or its successor about approval tiers, default-off/experimental
    posture, and verifier-gated auto-promotion.

## Recommended Validation Commands

```bash
cargo test -p ironclaw_memory
cargo test -p ironclaw_reborn_learning
cargo test -p ironclaw_loop_support identity_context
cargo test -p ironclaw_reborn_composition learning
cargo test -p ironclaw_architecture
scripts/ci/check-reborn-qa-fixtures.sh
```

Only run live QA recording manually and attended, with explicit credentials and
budget, after the live verifier PR exists.
