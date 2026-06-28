# Reborn Self-Learning — Phased Engineering Implementation Plan

> **For agentic workers:** This is a PR-roadmap-level engineering plan, not a
> per-line TDD task list. Each numbered PR is an independently reviewable,
> independently testable unit. When you implement a PR, follow the repo rules in
> `AGENTS.md` / `CLAUDE.md` and `superpowers:test-driven-development`
> (write the failing caller-level test first). Steps use checkbox (`- [ ]`)
> syntax where a sequence matters.

**Goal:** Build a general, out-of-band *self-learning substrate* for Reborn that
can manage learned memory, learned rules, learned skills, failure signatures,
verifier evidence, and human/admin-gated proposals — every artifact scoped,
sourced, TTL'd, verified, and use-time authorized — without ever blocking or
endangering the user-facing turn.

**Architecture:** A new background domain crate (`ironclaw_reborn_learning`) that
depends only on provider-neutral Reborn contracts. It subscribes to post-turn
milestones, builds candidate learnings asynchronously, verifies them by the
right method per artifact type (deterministic execution for skills, budgeted
live re-run for prompt/memory behavior, replay only as a regression lock), and
routes promoted artifacts to the systems that own them (memory via
`MemoryService`, rules via the identity-context/prompt path, skills via the
skill lifecycle) behind use-time authorization that is fail-closed.

**Tech stack:** Rust (tokio async), the post-#5163 memory contract crate
`ironclaw_memory`, `ironclaw_host_api` scope primitives, `ironclaw_turns`
milestone/loop-exit ports, `ironclaw_hooks` before-prompt seam,
`ironclaw_loop_support` identity context, `ironclaw_filesystem` scoped storage,
`ironclaw_triggers` cron poller, `ironclaw_approvals`, `ironclaw_safety`
redaction/leak-detection, and the existing reborn QA recorder / replay harness.

---

## Global Constraints

These apply to **every** PR below. A PR that violates one is not mergeable.

- **#5163 is the base.** This plan assumes PR #5163
  (`reborn/memory-placement-m2-lift`, currently **open**) has merged. After
  #5163: `ironclaw_memory` is the **provider-neutral memory contract** crate
  (`MemoryService`, DTOs, `MemoryDocumentScope`/`MemoryDocumentPath`/
  `MemoryContext`, prompt-safety vocab, audit/event contracts) and
  `ironclaw_memory_native` is the native filesystem provider
  (`NativeMemoryService`, repos, chunking, indexer). Learning code depends on
  `ironclaw_memory` (contract) and **never** on `ironclaw_memory_native`
  internals. *(Naming note: an earlier planning note guessed the contract crate
  would be `ironclaw_memory_contract`; the merged name is `ironclaw_memory`.)*
- **Learning is out-of-band.** User-facing turns emit bounded learning signals
  and return. No diagnosis, synthesis, verification, curation, or promotion runs
  inside the turn hot path.
- **Learning must not block or fail the turn.** If the learning system is
  disabled, backed up, budget-exhausted, or broken, normal turns still complete.
  The worst normal outcome is "learns later," never "user waits / run fails."
- **No default-on production behavior** until product + security posture is
  explicit. Master gate `IRONCLAW_REBORN_LEARNING` defaults **off**; live
  verification and fleet have their own sub-gates, also default off.
- **Every learned artifact carries source provenance, scope, TTL/retention, and
  status.** No exceptions, including seeded/test artifacts.
- **Use-time authorization is mandatory and fail-closed.** If the host cannot
  prove a learning is allowed in the current run, it is omitted.
- **Source permissions follow the learning.** Revoked/changed source or a
  conversation audience that is not a superset of the source audience ⇒ omit.
- **Scope defaults narrow; broadening requires evidence/policy.** Key learnings
  to the trusted `TurnOwner`, never the raw `TurnActor` (sender).
- **No global/unscoped learned state.** Never reuse the legacy global
  `tool_failures` table (`UNIQUE(tool_name)`, `migrations/V3__tool_failures.sql`).
- **No raw transcripts, secrets, host paths, or raw tool inputs** stored in
  learned artifacts. Store stable `SourceRef`s to durable evidence instead.
- **No unbounded learned memory or learned prompt rules.** Budgets are
  load-bearing (4 KiB before-prompt hook envelope / ~8000-token identity ceiling
  / per-family memory budgets), enforced from day one.
- **No LLM self-certification as an acceptance gate.** "The model said fixed" is
  a candidate, never a promotion. Mirror the existing `LoopExitEvidencePort`
  philosophy (drivers cannot self-certify completion).
- **Behavioral (prompt/memory) changes need budgeted live verification** when
  behavior proof is required. Replay locks plumbing/regression only — it cannot
  prove a prompt change altered model behavior.
- **Executable skill changes use deterministic verification** (execution) where
  possible.
- **High-risk changes stay human/admin-gated** (code/config/security/egress/
  global-behavior/ambiguous-audience).
- **Dual-backend persistence parity** (PostgreSQL + libSQL) is required before
  any production default-on, per `CLAUDE.md`. Early phases use a typed
  `ScopedFilesystem` store behind the off-by-default gate.
- **Everything UI-initiated goes through the facade.** Admin/inspect controls
  call `RebornServicesApi` (`ironclaw_product_workflow`), never raw stores
  (`.claude/rules/tools.md`). The background coordinator uses host-owned
  privileged bindings, not the model tool surface.
- **REPL/TUI logging discipline:** background learning uses `debug!`, never
  `info!`/`warn!` (corrupts the interactive display).
- **Caller-level tests are mandatory** where a helper gates a side effect
  (`.claude/rules/testing.md`, "Test Through the Caller, Not Just the Helper").

> **Line-number caveat:** symbol locations below were verified against `main`
> and the #5163 head (`f3fac01e3`). Cite the symbol + file; treat line numbers
> as approximate (they will drift after rebase).

---

## 1. Executive Summary

### What we are building

A **general learning substrate** for Reborn: a background subsystem that turns
experience (completed runs, failures, corrections, repeated workflows) into
*scoped, sourced, time-bounded, verified, status-tracked* learned artifacts, and
makes them safely available to future runs. It answers one question for every
future run:

> *Can this specific learning be safely used in this specific run, right now?*

That question is answered by host-owned **use-time authorization** (active?
expired? source revoked? scope ⊇ run? audience ⊇ conversation? in budget?
recently harmful?), not by "the model generated it" or "a user clicked approve."

### Why it is broader than memory or skills

Self-learned memory and self-learned skills are two *artifact types* that plug
into one lifecycle; they are not the system. The substrate also owns **learned
rules** (compact prompt-level behavioral corrections), **failure signatures**
(recurrence detectors with no raw content), **verifier evidence** (the proof
that lets automation replace approval), and **human/admin-gated proposals** (for
code/config/egress/global changes that must never auto-apply). A memory-only or
skill-only design re-implements provenance, scope, TTL, risk-tiering, and
verification N times and gets the trust boundary wrong each time. Building the
lifecycle once, with memory/rules/skills/proposals as routed outputs, is the
"deeper fix": *make the agent capable of safely learning from experience across
every surface where learning matters.*

### How it slots into Reborn after #5163

#5163 gives learning the right seam: memory now flows through the provider-
neutral `MemoryService` contract, so learning binds to `ironclaw_memory` (the
contract) and never reaches into native internals. Reborn already supplies the
rest of the scaffolding the substrate needs, verified in this plan:

| Need | Existing Reborn seam (verified) |
|---|---|
| Non-blocking post-turn trigger | `LoopHostMilestoneSink` (`Completed`/`Failed`) — `crates/ironclaw_turns/src/run_profile/milestones.rs`; durable tee `DurableLoopHostMilestoneSink` — `crates/ironclaw_reborn/src/milestone_events.rs` |
| "Drivers can't self-certify" gate philosophy | `LoopExitEvidencePort` / `LoopExitApplier` — `crates/ironclaw_turns/src/loop_exit.rs` |
| Scoped, isolated storage | `ScopedFilesystem` — `crates/ironclaw_filesystem/src/scoped.rs`; 7-axis `ResourceScope` — `crates/ironclaw_host_api/src/resource.rs` |
| Trusted-owner attribution | `TurnOwner` (`Personal` / `SharedAgent`) — `crates/ironclaw_turns/src/origin.rs` vs raw `TurnActor` — `crates/ironclaw_turns/src/scope.rs` |
| Compact rule injection | before-prompt hook (`HookedLoopPromptPort`, 4 KiB) — `crates/ironclaw_hooks/src/middleware/prompt_port.rs`; identity context (~8000 tok) — `crates/ironclaw_loop_support/src/identity_context.rs` |
| Recurring background passes | `TriggerPollerWorker` — `crates/ironclaw_triggers/src/worker.rs`; `spawn_trigger_poller` — `crates/ironclaw_reborn_composition/src/trigger_poller.rs` |
| Live behavioral verification | `record_qa_phrase` — `tests/support/reborn/qa_trace.rs` (today `#[ignore]` + `ANTHROPIC_API_KEY`) |
| Regression lock (not behavioral proof) | `RebornTraceReplayModelGateway` — `tests/support/reborn/model_replay.rs`; fixtures `tests/fixtures/llm_traces/reborn_qa/` (9 today) |
| Human gating | `ApprovalResolver` / `PersistentApprovalPolicy` — `crates/ironclaw_approvals/src/lib.rs` |
| Privileged host-internal binding | `CapabilityVisibility::HostInternal` — `crates/ironclaw_extensions/src/v2.rs` (reserved "memory injection, audit"; unused) |
| Boundary enforcement | `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` (`boundary_rules()`) |

The new code is therefore mostly **a coordinator + scoped stores + use-time
authorizers + verification wiring**, not new infrastructure.

---

## 2. Research Coverage Map

Illia's research requirements (and the explicitly-required topics) mapped to
phases/PRs. "WS" = Illia workstream; "Proposal §" = the 2026-06-24 engineering
proposal.

| Research requirement | Source | Phase / PR |
|---|---|---|
| **Out-of-band learning** (never in user turn) | WS1, Proposal §Operational | Global constraint; Phase B PR 8 (signal emission), Phase C PR 9 (coordinator) |
| **Verifier-gated promotion** (no self-cert) | WS1, ASG-SI/Voyager | Phase D PR 11–13 |
| **Replay = regression lock, not behavioral proof** | §0.1, Proposal §Verification | Phase D PR 11 (replay-lock), PR 12 (live verify); stated in every verify acceptance criterion |
| **Stateful-minus-stateless Gain** | WS5, CL-Bench | Phase H PR 21 |
| **Active curation, bounded** | WS2/WS4, MemoryAgentBench | Phase G PR 19; budgets enforced from PR 6/PR 14 onward |
| **Bounded rules / bounded memory** | WS2/WS4 | Phase B PR 6 (rule budget), Phase E PR 14 (per-family memory budget), Phase G PR 19 |
| **Live canaries for high-value signatures** | WS1b | Phase H PR 22 |
| **Scoped learnings (narrow default, evidence to broaden)** | WS8/§3.5 | Phase A PR 1–2 (scope-keyed from line 1), enforced everywhere |
| **Key to TurnOwner not TurnActor** | WS1/WS8 | Phase B PR 8; tested in PR 7/PR 8 |
| **Multi-tenant isolation (Phase-1 prereq)** | WS8 | Phase A PR 2 (isolation parity test) + every store PR |
| **Failure signatures (dedupe + escalate)** | WS1, Proposal §Failure Sig | Phase B PR 8, Phase C PR 10 |
| **Learned rules (scoped, budgeted, use-time auth)** | WS2 | Phase B PR 4–7 |
| **Memory learning (provenance/TTL/revocation/supersession)** | WS4 + provenance/TTL security adds | Phase A PR 3, Phase E PR 14–15 |
| **Skill learning (deterministic verify, promote/demote, gating)** | WS3 | Phase F PR 16–17 |
| **Verifier evidence as automation-enabler** | Proposal §Verification | Phase D PR 11–12 (evidence records) |
| **Human/admin-gated proposals** | Proposal §Risk Tiers | Phase D PR 13, Phase G PR 18 |
| **Self-modification gate (in-process replay)** | WS6 | Phase D PR 13 |
| **Product/admin controls (pause/inspect/suppress/delete/approve)** | Proposal §Product UX | Phase G PR 18 |
| **Stateful eval + transfer (forward/backward)** | WS5 | Phase H PR 21 |
| **Trigger immediacy for corrections** | WS7 | Phase H PR 23 |
| **Fleet: late, opt-in, telemetry-first/skills-first, de-id, corroborated, locally re-verified** | WS9 | Phase I PR 24–25 |
| **Dual-backend persistence parity** | `CLAUDE.md` | Phase G PR 20 (gate before default-on) |
| **Anti-goals** (no closed-loop optimizer, no unbounded overlay, no unscoped state, no silent overwrite, no auto code/config, no fleet auto-trust, no turn blocking) | §5, Proposal §Non-Goals | §7 below; enforced as acceptance criteria throughout |

---

## 3. System Boundary Overview

Where learning touches the system, mapped to verified seams. Each boundary names
what learning **may** and **may not** do there.

1. **Channel / ingress boundary.** Channels keep normalizing input into turns.
   They do *not* decide what is learned. The only learning-relevant duty is
   preserving trusted identity/source context (which user/workspace/agent/
   project/conversation/integration a signal came from) so downstream
   authorization can key on the *trusted* owner, not the request body. Seam:
   `TurnScope` / `TurnActor` / `TurnOwner` resolution
   (`crates/ironclaw_turns/src/{scope,origin}.rs`).

2. **Reborn runtime / turn-loop boundary.** Owns the user-facing model/tool
   loop. At completion/failure/correction it **emits a bounded learning signal
   and returns** — no learning work in the hot path. Seam: `LoopHostMilestoneKind`
   `Completed`/`Failed` (carry `exit_id`, `run_id`, `scope`, `actor`).

3. **Durable event / evidence boundary.** The event/run history is the evidence
   source; artifacts point back to it (`SourceRef`) instead of duplicating
   sensitive content. Reconstruction happens under the *same* scope/permission
   model as future use. Seam: `DurableLoopHostMilestoneSink` →
   `RuntimeEvent` log (`crates/ironclaw_reborn/src/milestone_events.rs`),
   `ironclaw_events` / `ironclaw_event_projections`.

4. **Background learning coordinator boundary.** The offline worker: consumes
   signals, dedupes, classifies, synthesizes candidates, verifies, changes
   status, curates. Not a model tool. Owns queue/priority/budget/backpressure/
   retry. Seam: new `ironclaw_reborn_learning::LearningCoordinator`, spawned in
   `build_reborn_runtime` like `spawn_trigger_poller`.

5. **Memory service boundary.** Learned memory flows **through** `MemoryService`
   (`ironclaw_memory`), never around it. Learning *proposes* a memory item with
   provenance/TTL; retrieval still owns indexing/chunking/storage; use-time
   filters (scope/audience/TTL/revocation) gate inclusion. Seam:
   `MemoryService::{write,retrieve_context}`; today the prompt path is **dead**
   (`crates/ironclaw_loop_support/src/lib.rs` `memory_snippets: Vec::new()`) and
   must be wired before TTL/audience filters at the adapter are load-bearing.

6. **Prompt / context boundary.** Learned **rules** enter future runs as compact,
   scoped, budgeted snippets — the model sees only the rule, never the source
   evidence or provenance. The host selects applicable active rules before
   prompt construction; expired/revoked/wrong-scope/over-budget rules are
   omitted. Seams: before-prompt hook (`HookedLoopPromptPort`, 4 KiB) **and/or**
   identity context (`HostIdentityContextSource`, ~8000 tok); `LEARNED_RULES.md`
   protected path.

7. **Skill boundary.** Learned skills use the skill lifecycle; the selector only
   considers learned skills that are active+scoped+verified+allowed. Executable
   fixes verify deterministically; broad/capability-expanding skills stay
   review-gated. Seams: `ironclaw_skills` (`SkillManifest`, `selector.rs`,
   `gating.rs`), `ironclaw_skill_learning` (distillation), new `SkillAdmin` port.

8. **Capability / tool boundary.** Learning *observes* tool outcomes but never
   grants tool rights. A learned artifact may reference a capability + failure
   category; future use still goes through approval/auth/sandbox/policy
   unchanged. Seam: capability dispatch + `ApprovalResolver` remain the
   authority.

9. **Product / admin boundary.** Surfaces control + inspection (pause learning,
   pause usage, inspect active/pending, approve high-risk, suppress/delete, "why
   learned / from where"). Talks through `RebornServicesApi`
   (`ironclaw_product_workflow`) + `ironclaw_webui_v2` descriptors — never raw
   stores. The review UI is the *exception* path, not the primary safety model.

10. **Model-provider / budget boundary.** Diagnosis, synthesis, memory
    extraction, and live verification call a provider in the **background**, with
    separate per-tenant/owner budget, rate limits, observability, and egress
    disclosure. They never block the turn and never self-promote. Seam: the
    coordinator's budgeted live-verify path (lifted `record_qa_phrase`).

11. **Persistence / DB boundary.** Early stores are typed `ScopedFilesystem`
    documents (provider-neutral, off by default). Production requires dual-backend
    parity (PostgreSQL + libSQL) following the
    `ironclaw_hooks` / `ironclaw_hooks_postgres` / `ironclaw_hooks_libsql` /
    `ironclaw_hooks_parity` crate-split model — **not** the root `src/db`
    `Database` trait (the learning crate must not depend on the root `ironclaw`
    crate). No reuse of the global `tool_failures` table.

---

## 4. Phased PR Roadmap

25 PRs across 9 phases (A–I). Each PR is independently reviewable and testable.
Dependencies are explicit. Per-PR template: **Goal · After-merge behavior ·
Depends on · System areas · Storage/schema (PG/libSQL parity) · Tests
(caller-level) · Security/privacy risks · Rollout/flag · Rollback · Acceptance ·
Validation**.

---

### Phase A — Foundation: artifact model, scoped stores, gate (no behavior)

#### PR 1 — Learning artifact model + crate skeleton + boundary rule + flag

- **Goal:** Stand up `crates/ironclaw_reborn_learning` with the provider-neutral
  artifact vocabulary every later PR depends on. Pure types, zero runtime
  behavior.
- **After-merge behavior:** None observable. The crate compiles, exports types,
  and is gated off. The architecture test forbids it from depending on
  disallowed crates.
- **Depends on:** #5163 merged.
- **System areas:**
  - Create `crates/ironclaw_reborn_learning/{Cargo.toml,CLAUDE.md}` and
    `src/{lib.rs,error.rs,types.rs,provenance.rs,ttl.rs}`.
  - `types.rs`: `LearnedRuleId`, `FailureSignature`, `LearningArtifactKind`
    (`Memory`/`Rule`/`Skill`/`FailureSignature`/`Proposal`), status enums
    (`LearnedRuleStatus`, `LearnedFailureStatus`: `Candidate`/`PendingReview`/
    `Active`/`Suppressed`/`Expired`/`Revoked`/`Demoted`), `LoopSafeSummary`
    (bounded, sanitized text newtype), `LearnedRuleRecord`,
    `LearnedFailureRecord`, `RiskTier` (`Low`/`Medium`/`High`).
  - `provenance.rs`: `SourceProvenance { source_ref, extracted_at,
    content_digest, source_scope, source_audience, revocation_ref,
    extractor_version }`; `SourceRef` enum (`TurnRun`/`ConversationMessage`/
    `MemoryDocument`/`CapabilityOutput`/`ExternalDocument`); `SourceAudience`.
  - `ttl.rs`: `RetentionPolicy { extracted_at, expires_at, retention_class,
    model_suggested_ttl_seconds, rationale }`; `RetentionClass`
    (`Ephemeral`/`Session`/`Project`/`Durable`/`NeverExpires`); host clamp fn
    `clamp_retention(suggested, kind) -> RetentionPolicy`.
  - Reuse `ResourceScope` / `TurnOwner` from `ironclaw_host_api` /
    `ironclaw_turns`; do **not** invent a parallel scope type.
  - `error.rs`: sanitized `LearningError` taxonomy (`thiserror`), no raw content
    in messages.
  - Cargo: add workspace member; define feature/config gate constant for
    `IRONCLAW_REBORN_LEARNING` (off).
  - Add a `BoundaryRule` for `ironclaw_reborn_learning` in
    `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`
    (`boundary_rules()`), forbidding: root `ironclaw`, `ironclaw_memory_native`,
    `ironclaw_product_workflow`, `ironclaw_webui_v2`, `ironclaw_engine`,
    `ironclaw_gateway`, dispatch/host_runtime/native provider internals. Allow:
    `ironclaw_host_api`, `ironclaw_memory` (contract), `ironclaw_turns`,
    `ironclaw_loop_support`, `ironclaw_events`, `ironclaw_filesystem`,
    `ironclaw_safety`.
- **Storage/schema:** None.
- **Tests:** `cargo test -p ironclaw_reborn_learning` — serde round-trip for all
  records; `clamp_retention` clamps an over-long model suggestion to policy
  ceiling per kind; status transition validity table; `LoopSafeSummary` rejects
  oversized/sensitive content. `cargo test -p ironclaw_architecture` — new
  boundary rule holds.
- **Security/privacy risks:** Low. Risk = leaking a raw-content constructor;
  mitigate by validating-at-boundary constructors only (`LoopSafeSummary::new`
  rejects > N bytes; no public unchecked ctor).
- **Rollout/flag:** Crate present, `IRONCLAW_REBORN_LEARNING` off; nothing wired.
- **Rollback:** Delete crate + workspace member + boundary rule. No data.
- **Acceptance:** Crate compiles; boundary test passes; every artifact type has
  scope+source+TTL+status fields with no way to construct one missing them.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning
  cargo test -p ironclaw_architecture
  cargo clippy -p ironclaw_reborn_learning --all-features --tests
  ```

#### PR 2 — `LearningStore` trait + in-memory + `ScopedFilesystem` store + isolation parity

- **Goal:** A scoped, fail-closed store for learned records with dedupe and
  status filtering; proven isolated across tenant/user/agent/project.
- **After-merge behavior:** None observable (no caller yet). Store usable from
  tests.
- **Depends on:** PR 1.
- **System areas:**
  - `src/store.rs`: `LearningStore` trait — `upsert_failure(scope, signature)`
    (increments `occurrence_count` on `(owner axes, signature)` collision),
    `list_active_rules(run_scope)`, `put_rule`/`transition_rule_status`
    (CAS/version), `get`, plus filters that drop `Expired`/`Revoked` before
    returning prompt candidates. In-memory impl.
  - `src/filesystem_store.rs`: typed store over `ScopedFilesystem` /
    `RootFilesystem`; virtual paths derived from `ResourceScope` mirroring the
    `ironclaw_memory` path grammar (`crates/ironclaw_memory/src/path.rs`); writes
    go through the scope resolver chokepoint.
  - Key on `TurnOwner`-derived owner axes (helper to flatten `TurnOwner` →
    owner-key segment), **not** `ResourceScope.user_id` directly (which a
    `SharedAgent` run would mis-key).
- **Storage/schema:** Filesystem documents under a scoped virtual path
  (`learning/rules/...`, `learning/failures/...`). No SQL yet. (PG/libSQL parity
  deferred to PR 20.)
- **Tests:** `cargo test -p ironclaw_reborn_learning`:
  - failure upsert increments instead of duplicating on repeated signature;
  - `list_active_rules` excludes `Expired`/`Revoked`/`Suppressed`;
  - **scope-isolation parity** in the
    `tests/reborn_*_scope_isolation_parity.rs` style: a wrong-tenant /
    wrong-user / wrong-agent / wrong-project caller gets `.is_err()` (or empty,
    fail-closed) from the store — *this is the privacy bar; a leak here is an
    incident, not a bug*;
  - status transition CAS rejects stale writes.
- **Security/privacy risks:** Cross-scope leakage is the top risk → the parity
  test is the gate. Filesystem path traversal → reuse validated path
  constructors.
- **Rollout/flag:** Behind `IRONCLAW_REBORN_LEARNING` (off). No production
  default.
- **Rollback:** Remove store wiring; filesystem docs are inert and scoped.
- **Acceptance:** Parity test passes for all four axes; dedupe + status filtering
  proven; store never returns an artifact whose scope is broader than the query
  scope.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning
  cargo test -p ironclaw_reborn_learning scope_isolation
  ```

#### PR 3 — Memory contract: typed provenance + retention metadata (additive)

- **Goal:** Give memory artifacts first-class provenance + TTL on the contract,
  so memory learning (Phase E) and use-time authorization have typed fields, not
  `extra`-map strings.
- **After-merge behavior:** Memory writes may carry provenance/retention;
  retrieval excludes expired items. No new extraction yet.
- **Depends on:** #5163; coordinate with memory owners (this edits the contract
  crate — see "Module Specs" / `crates/ironclaw_memory/CLAUDE.md`).
- **System areas:**
  - `crates/ironclaw_memory/src/metadata.rs`: add typed
    `provenance: Option<MemoryProvenance>` and
    `retention: Option<RetentionPolicy>` to `DocumentMetadata` (today:
    `skip_indexing`, `skip_versioning`, `hygiene`, `schema`, `extra`). Keep
    `extra` for forward-compat. Mirror the `ttl.rs`/`provenance.rs` shapes from
    PR 1 (or factor a shared neutral type — see Open Decision; default: define in
    `ironclaw_memory` and re-export).
  - Host-clamp TTL policy applied at the `metadata_overlay` write path
    (`MemoryBackendWriteOptions::metadata_overlay`).
  - `ironclaw_memory_native` persists + inherits the new fields via existing
    metadata mechanics (no new write signature).
  - Extend `retrieve_context` filtering in `ironclaw_memory_native`
    (`src/service.rs`, the `results.retain(...)` site) to drop expired items —
    *additive to the existing scope filter*.
- **Storage/schema:** Metadata is filesystem-document JSON (native provider).
  No SQL.
- **Tests:** `cargo test -p ironclaw_memory` — provenance/retention round-trip;
  strict untrusted-metadata parse rejects malformed fields; host clamp caps an
  over-long suggested TTL. `cargo test -p ironclaw_memory_native` — expired item
  excluded from `retrieve_context`; non-expired included; scope filter still
  holds.
- **Security/privacy risks:** A provider could become the sole permission
  boundary — mitigate by keeping the **host-owned** authorizer (Phase E) as the
  final gate, not the provider's filter alone.
- **Rollout/flag:** Fields are `Option`, default `None`; behavior unchanged when
  absent. Safe to land independent of the learning gate.
- **Rollback:** Fields are additive + optional; revert is clean (no migration).
- **Acceptance:** Metadata round-trips; expired exclusion works; existing memory
  tests stay green; `ironclaw_architecture` memory boundary unaffected.
- **Validation:**
  ```bash
  cargo test -p ironclaw_memory
  cargo test -p ironclaw_memory_native
  cargo test -p ironclaw_architecture
  ```

---

### Phase B — Learned-rule injection rails + bounded signal emission

#### PR 4 — `LEARNED_RULES.md` protected identity path

- **Goal:** Register a protected, host-only identity file class for learned
  rules so they ride the existing prompt-safety machinery.
- **After-merge behavior:** `IdentityFileName::new("LEARNED_RULES.md")` succeeds;
  unknown names still fail; the registry recognizes it with distinct telemetry.
- **Depends on:** #5163.
- **System areas:**
  - `crates/ironclaw_memory/src/safety.rs`: add `LEARNED_RULES.md` to
    `DEFAULT_PROMPT_PROTECTED_PATHS`; add a `PromptProtectedPathClass` arm +
    stable `as_str()` identifier (`learned_rules_md`) so it gets per-file
    telemetry rather than falling through to `custom_protected_path`.
  - `crates/ironclaw_loop_support/src/identity_context.rs`: ensure
    `IdentityFileName::new` accepts the new name (validates against the const).
- **Storage/schema:** None.
- **Tests:** `cargo test -p ironclaw_memory safety` — new path recognized, class
  string stable. `cargo test -p ironclaw_loop_support identity_context` —
  `IdentityFileName::new("LEARNED_RULES.md")` ok; an unknown file still errors.
- **Security/privacy risks:** Low; widening the protected set is host-controlled.
- **Rollout/flag:** Independent of the learning gate (pure registry addition).
- **Rollback:** Remove the path + class arm.
- **Acceptance:** Both tests pass; no existing protected-path test regresses.
- **Validation:**
  ```bash
  cargo test -p ironclaw_memory safety
  cargo test -p ironclaw_loop_support identity_context
  ```

#### PR 5 — `CompositeHostIdentityContextSource` (generic composer)

- **Goal:** Allow multiple identity sources to compose (today only a single
  default source is wired; no combiner exists).
- **After-merge behavior:** A composite source can wrap N sources, preserve
  order, and share the identity budget. No learned source yet.
- **Depends on:** PR 4 (not strictly, but lands together).
- **System areas:** `crates/ironclaw_loop_support/src/identity_context.rs`: add
  `CompositeHostIdentityContextSource` implementing `HostIdentityContextSource`,
  delegating to an ordered `Vec<Arc<dyn HostIdentityContextSource>>`, merging
  candidates and resolving content, honoring `IdentityBudget` (default ~8000
  tok) with deterministic ordering.
- **Storage/schema:** None.
- **Tests:** `cargo test -p ironclaw_loop_support identity_context` — composite
  preserves source order + resolution; budget truncation deterministic; empty
  inner sources handled.
- **Security/privacy risks:** Low.
- **Rollout/flag:** Generic; no behavior until something composes with it.
- **Rollback:** Remove the type.
- **Acceptance:** Composite returns the union of candidates in order, within
  budget; existing `DefaultSystemPromptIdentitySource` still works standalone.
- **Validation:** `cargo test -p ironclaw_loop_support identity_context`

#### PR 6 — `LearnedRulesIdentityContextSource` + use-time authorization (fail-closed)

- **Goal:** Render active, authorized, unexpired learned rules for the current
  run scope as a compact trusted snippet — the use-time authorization core.
- **After-merge behavior:** Given a store with active rules, the source yields a
  bounded `LEARNED_RULES.md`-class snippet for matching scope; omits everything
  that fails authorization. Not yet wired into composition (PR 7).
- **Depends on:** PR 2, PR 4, PR 5.
- **System areas:**
  - `crates/ironclaw_reborn_learning/src/identity_context.rs`:
    `LearnedRulesIdentityContextSource` implementing `HostIdentityContextSource`.
  - `src/authz.rs`: `authorize_use(run_scope, run_audience, rule) -> bool` —
    fail-closed checks: status `Active`; not expired (TTL); source not revoked /
    digest matches when checkable; `run_scope ⊆ rule.scope` (equal-or-narrower);
    conversation audience ⊆ source audience when source has one. Any
    indeterminate ⇒ omit.
  - `src/rules.rs`: selection + budget ordering (specificity × confidence ×
    recency × severity) + rendering. Render **only** short rule text + opaque
    rule id; never source content, raw prompts, tool args, secrets, host paths.
    Sub-budget (e.g. ≤ 1 KiB inside the identity ceiling).
- **Storage/schema:** Reads PR 2 store. None new.
- **Tests:** `cargo test -p ironclaw_reborn_learning`:
  - TTL excludes expired; revocation/source-authorization excludes unusable;
    scope mismatch excludes wrong tenant/user/agent/project; audience mismatch
    excludes; rendering stays within budget and omits source detail; ordering
    deterministic. Each is a use-time-authorization unit test.
- **Security/privacy risks:** This is the leak chokepoint. The fail-closed
  default + the omit-on-indeterminate rule are the mitigations; tested
  explicitly.
- **Rollout/flag:** Behind `IRONCLAW_REBORN_LEARNING`.
- **Rollback:** Remove the source; nothing consumes it yet.
- **Acceptance:** Every authorization dimension has a passing exclusion test;
  rendered output provably contains no source/provenance detail.
- **Validation:** `cargo test -p ironclaw_reborn_learning`

#### PR 7 — Wire learned-rule injection in Reborn composition (seeded rules) + scope tests

- **Goal:** End-to-end injection of a **seeded** scoped rule into the model
  request, behind the flag — proving the prompt path without any auto-learning.
- **After-merge behavior:** With the flag on and a seeded active rule, a matching
  run's model request includes the rule; a non-matching scope's does not.
- **Depends on:** PR 6.
- **System areas:**
  - `crates/ironclaw_reborn_composition/src/runtime_input.rs`: add a
    `with_learning(...)` builder carrying learning settings + (test-only) seedable
    store handle.
  - `crates/ironclaw_reborn_composition/src/runtime.rs`: when the flag is on,
    build the learning store + `LearnedRulesIdentityContextSource`, compose it
    with `DefaultSystemPromptIdentitySource` via the composite from PR 5, and
    attach as the run's identity source. Keep raw store handles private
    (`factory.rs`); expose only the composed source / a facade handle.
- **Storage/schema:** Uses PR 2 store. None new.
- **Tests:** `cargo test -p ironclaw_reborn_composition learning` (caller-level,
  via `RebornBinaryE2EHarness` + `RebornTraceReplayModelGateway` request
  capture): a seeded rule for `(tenant,user[,agent,project])` appears in the
  captured model request; the **same** seeded rule does **not** appear for
  another tenant/user/agent/project; budget overflow drops rules silently
  (logged `debug!`).
- **Security/privacy risks:** Wrong-scope injection = leak → the negative scope
  test is the gate. Role attenuation: ensure the rule is injected as host/system
  trust, not user-trust (matches identity-source trust handling).
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING` on enables injection; off = no-op
  (production stays off).
- **Rollback:** Flip the flag off; remove composition wiring. No data.
- **Acceptance:** Positive + negative scope tests pass through the real caller
  path; flag-off path is a verified no-op.
- **Validation:**
  ```bash
  cargo build -p ironclaw_reborn_composition --features "webui-v2-beta libsql"
  cargo test -p ironclaw_reborn_composition learning
  ```

#### PR 8 — Milestone observer → scoped failure capture (TurnOwner-keyed, non-blocking)

- **Goal:** Capture failed runs as scoped, deduped `LearnedFailureRecord`s
  without blocking the turn and without storing raw content.
- **After-merge behavior:** With the flag on, a `Failed` milestone increments a
  scoped failure record keyed to `TurnOwner`; `Completed` optionally records
  bounded success metadata. No rule is created/injected yet.
- **Depends on:** PR 2, PR 7.
- **System areas:**
  - `crates/ironclaw_reborn_learning/src/coordinator.rs`: a milestone observer
    that wraps/tees `DurableLoopHostMilestoneSink`
    (`crates/ironclaw_reborn/src/milestone_events.rs`). On terminal milestones it
    computes a coarse `FailureSignature` = `loop_failure_kind + last capability
    id/failure kind (if available) + run profile + safe surface family`. It
    stores **only** the signature + a `SourceRef` to durable evidence — never raw
    messages/inputs.
  - Attribution: derive owner key from `TurnOwner` (`Personal` / `SharedAgent`),
    never from `TurnActor`. In a `SharedAgent` run, personal facts must filter to
    `(tenant,user)`, not project scope.
  - Wiring in `crates/ironclaw_reborn_composition/src/runtime.rs` (tee the
    existing sink). Best-effort: failures in capture are swallowed + `debug!`,
    never propagated to the turn.
- **Storage/schema:** PR 2 store (`learning/failures/...`). None new.
- **Tests:** `cargo test -p ironclaw_reborn_composition learning` (caller-level):
  a terminal `Failed` milestone through the real harness writes/increments a
  scoped failure record; a repeat increments rather than duplicates; a
  `SharedAgent` run keys to the owner, not the sender (drive both a `Personal`
  and a `SharedAgent` scope); capture failure does not fail the turn (inject a
  store error, assert the turn still completes).
- **Security/privacy risks:** Signature must not encode raw content → assert the
  stored signature contains no message/tool-arg text. Owner mis-keying = cross-
  user interference → the `SharedAgent` test is the gate.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING` on; off = sink untouched.
- **Rollback:** Remove the tee; records are inert + scoped.
- **Acceptance:** Failure capture works through the caller path, dedupes,
  owner-keys correctly, is non-blocking, and stores no raw content.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_composition learning
  cargo test -p ironclaw_reborn_learning
  ```

---

### Phase C — Background coordinator (async, budgeted, backpressure)

#### PR 9 — `LearningCoordinator` async worker + budget/backpressure/retry (no model)

- **Goal:** Move candidate-building off the milestone callback onto a bounded
  background worker with budget, backpressure, and retries.
- **After-merge behavior:** Signals enqueue to a bounded channel; a spawned
  worker drains, dedupes, and persists candidates. Diagnosis/synthesis stubbed
  (no model calls). Turn latency unaffected even at saturation.
- **Depends on:** PR 8.
- **System areas:**
  - `src/coordinator.rs`: bounded `tokio::sync::mpsc` queue fed by the observer;
    a worker loop (priority: corrections/repeats first, ordinary signals batch);
    per-tenant/owner budget + backpressure (drop/coalesce on overflow, `debug!`
    what was dropped — *no silent caps*); retry with backoff for transient store
    errors.
  - Spawn in `crates/ironclaw_reborn_composition/src/runtime.rs` next to
    `spawn_trigger_poller`, returning a handle with a cancellation token for
    clean shutdown.
- **Storage/schema:** PR 2 store. None new.
- **Tests:** `cargo test -p ironclaw_reborn_learning`: enqueue → dedupe → drain;
  backpressure coalesces/drops gracefully + logs the drop count; cancellation
  stops the worker; budget exhaustion stops new model-bound work (stub).
  Caller-level: turn latency unaffected when the queue is saturated or the worker
  is disabled.
- **Security/privacy risks:** Noisy-neighbor (one owner's failure flood starves
  others / blows budget) → per-tenant/owner budget is the mitigation; tested.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING` on; off = no worker spawned.
- **Rollback:** Cancel + drop the handle; queue drains to nothing.
- **Acceptance:** Worker is non-blocking, bounded, fair, observable; drops are
  logged, never silent.
- **Validation:** `cargo test -p ironclaw_reborn_learning coordinator`

#### PR 10 — Failure-signature dedupe/escalation + candidate emission

- **Goal:** Escalate recurring failures by frequency/severity and emit a
  `Candidate` learning (still not injected/promoted).
- **After-merge behavior:** A signature crossing a frequency/severity threshold
  produces a `Candidate` rule/learning record (status `Candidate`), prioritized
  for later verification.
- **Depends on:** PR 9.
- **System areas:** `src/failure.rs`: escalation policy (occurrence-count +
  severity thresholds), candidate creation with provenance (`SourceRef` to the
  evidence) and a conservative narrow scope + clamped TTL. `src/coordinator.rs`:
  route escalated signatures to candidate emission.
- **Storage/schema:** PR 2 store. None new.
- **Tests:** `cargo test -p ironclaw_reborn_learning`: dedupe stability (same
  signature → one row, count grows); escalation fires only past threshold;
  candidate carries narrow scope + clamped TTL + provenance; candidates are
  excluded from `list_active_rules` (not injectable).
- **Security/privacy risks:** Over-broad candidate scope → default narrowest;
  tested.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`.
- **Rollback:** Disable escalation; candidates inert.
- **Acceptance:** Escalation is frequency/severity-gated, candidates are narrow +
  sourced + non-injectable.
- **Validation:** `cargo test -p ironclaw_reborn_learning failure`

---

### Phase D — Verification (deterministic + live) + self-modification gate

#### PR 11 — Verifier traits + deterministic replay-lock gate (in-process)

- **Goal:** Define the verifier interface and the **regression-lock** path; make
  explicit that replay locks plumbing, not behavior.
- **After-merge behavior:** Before any commit of a learned change, the
  coordinator can run the deterministic reborn replay set in-process and reject
  on drift. A deterministic fake verifier exists for tests.
- **Depends on:** PR 10.
- **System areas:**
  - `src/verifier.rs`: `LearningVerifier` trait
    (`verify_candidate(candidate) -> VerifierDecision`); `VerifierEvidence`
    record (type, outcome, timestamp, budget spent); deterministic fake.
  - Replay-lock helper invoking `RebornTraceReplayModelGateway::from_trace` over
    parity + `reborn_qa` fixtures (`tests/fixtures/llm_traces/reborn_qa/`) and
    asserting no plumbing drift. Document in code + `CLAUDE.md` that this gateway
    selects by FIFO/optional-substring and performs **no structural prompt
    comparison**, so it cannot prove a prompt change altered behavior.
- **Storage/schema:** `VerifierEvidence` attached to records (PR 2 store).
- **Tests:** `cargo test -p ironclaw_reborn_learning verifier` — fake verifier
  decisions; replay-lock detects an injected plumbing regression and passes a
  clean run. `cargo test --features integration` for any composition-level
  replay wiring.
- **Security/privacy risks:** Misreading replay as behavioral proof → the
  documented distinction + the live path (PR 12) are the guard.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`.
- **Rollback:** Remove verifier wiring; candidates simply never promote.
- **Acceptance:** Replay-lock rejects drift; evidence records persist; no
  candidate auto-promotes on replay alone.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning verifier
  scripts/ci/check-reborn-qa-fixtures.sh
  ```

#### PR 12 — Budgeted live-verify runner + lock fixture (prompt/memory behavior)

- **Goal:** Prove a prompt/memory fix actually changes model behavior via a
  budgeted live re-run; lock the result as a new regression fixture.
- **After-merge behavior:** With the live sub-gate **explicitly** enabled and a
  budget, a `Candidate` prompt/memory rule is re-run live (temp 0) on the
  captured failing task; on pass it becomes `Active` and a new `reborn_qa`
  Tier-1 fixture + Tier-2 contract assertion is saved + the failure marked
  `locked`; on fail it is discarded + logged. Default: off.
- **Depends on:** PR 11.
- **System areas:**
  - Lift `record_qa_phrase` (`tests/support/reborn/qa_trace.rs`) into a
    budgeted, background-callable live-verify path callable from the coordinator
    (today only reachable from `#[ignore]` tests; needs `ANTHROPIC_API_KEY` +
    budget). Place the reusable runner where non-test code can call it (e.g. a
    `live_verify` module in the learning crate or a thin support crate), keeping
    test-only fixtures out of production builds.
  - Per-tenant/owner budget + rate-limit, prioritized by signature
    frequency/severity (only repeated/high-value mistakes earn a live verify).
  - Egress disclosure: record that learning sent data to a provider.
  - On accept: write fixture under `tests/fixtures/llm_traces/reborn_qa/` keyed
    by `failure_signature`; add a Tier-2 contract assertion in
    `tests/reborn_qa_recorded_behavior.rs`; transition rule `Candidate→Active`,
    failure `→ locked`. (Note: the `reborn_qa` tier already has 9 committed
    fixtures — this **adds**, it does not seed an empty tier.)
- **Storage/schema:** `VerifierEvidence` (live) + new fixture files. No SQL.
- **Tests:** Deterministic: a fake live-model adapter (recorded) drives the
  accept/discard branches without real spend. Attended/manual: a documented
  `#[ignore]` live recording test that exercises the real path with explicit
  credentials + budget. Assert: discard-on-fail leaves status `Candidate`;
  accept transitions + writes a scrubbed fixture (run
  `scripts/ci/check-reborn-qa-fixtures.sh`).
- **Security/privacy risks:** Egress + cost + secret leakage into fixtures →
  per-owner budget, opt-in sub-gate, and the fixture scrubber are the
  mitigations. Live verify runs in the **originating scope** with that scope's
  model selection/tools.
- **Rollout/flag:** New sub-gate `IRONCLAW_REBORN_LEARNING_LIVE_VERIFY` (off);
  per-tenant budget required to be non-zero to run.
- **Rollback:** Disable the sub-gate; candidates stay `Candidate`; no fixtures
  written.
- **Acceptance:** Behavioral acceptance requires a live pass (never replay/self-
  cert); accepted runs lock a scrubbed fixture; budget + disclosure enforced.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning live_verify
  scripts/ci/check-reborn-qa-fixtures.sh
  # attended only, with credentials + budget:
  # cargo test -p <crate> live_record -- --ignored
  ```

#### PR 13 — Self-modification gate + CODE/CONFIG propose-only via approvals

- **Goal:** No learned change commits without passing the in-process replay set;
  code/config/egress/global stays human-gated.
- **After-merge behavior:** Any rule/skill write first runs the deterministic
  replay set (parity + locked `reborn_qa`); drift ⇒ reject + retain prior state.
  CODE/CONFIG candidates become `PendingReview` proposals routed to approvals,
  never auto-applied. Failure signatures may still record recurrence while a
  proposal waits.
- **Depends on:** PR 11, PR 12.
- **System areas:**
  - `src/gate.rs`: pre-commit replay-set runner reused from PR 11; reject ⇒ keep
    prior state (the durable event log makes rollback clean).
  - Route `RiskTier::High` / kind `Proposal` to `ApprovalResolver` /
    `PersistentApprovalPolicy` (`crates/ironclaw_approvals/src/lib.rs`) — wire a
    proposal-review policy keyed to `(tenant, owner)`.
- **Storage/schema:** Approval records via the existing approvals store
  (filesystem-persistent). No new schema.
- **Tests:** `cargo test -p ironclaw_reborn_learning gate` — drift rejects a
  commit + state unchanged; a CODE/CONFIG candidate lands `PendingReview`, never
  `Active`, and surfaces to approvals. `cargo test --features integration` for
  the approvals wiring.
- **Security/privacy risks:** Auto-applying high-risk change = the core danger →
  the propose-only routing is the gate; tested that no high-risk candidate
  reaches `Active` without approval.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`; high-risk always gated regardless
  of other sub-gates.
- **Rollback:** Disable promotion; proposals queue harmlessly.
- **Acceptance:** No commit on drift; no high-risk auto-apply; proposals are
  reviewable.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning gate
  cargo test --features integration
  ```

---

### Phase E — Memory learning (provenance, TTL, revocation, supersession)

#### PR 14 — Memory candidate extraction (provenance + TTL) + use-time auth at retrieval

- **Goal:** Turn an observed durable fact/preference/convention into a scoped,
  sourced, TTL'd memory item via `MemoryService`, authorized at retrieval.
- **After-merge behavior:** The coordinator can propose a memory item
  (`MemoryService::write` with provenance + clamped TTL); future retrieval
  includes it only when use-time authorization passes.
- **Depends on:** PR 3, PR 9. **Prerequisite:** wire the dead memory-snippet
  path first — `crates/ironclaw_loop_support/src/lib.rs` currently hardcodes
  `memory_snippets: Vec::new()` and the loop never calls `retrieve_context`, so
  TTL/audience filters at the adapter are inert until this is connected.
- **System areas:**
  - `src/memory_learning.rs`: extraction → `MemoryService::write` (contract,
    never native internals) with `SourceProvenance` + `RetentionPolicy`
    (model-suggested, host-clamped). Bounded summaries only — no raw transcript.
  - Wire `retrieve_context` into the loop's `memory_snippets` (fix the dead path)
    and apply the **host-owned** use-time authorizer (PR 6 `authz`) so the
    provider's filter is not the sole boundary.
  - Per-family token budget for memory injection (bounds growth).
- **Storage/schema:** Native filesystem memory docs (via `MemoryService`). No
  SQL.
- **Tests:** `cargo test -p ironclaw_memory_native` + caller-level
  `cargo test -p ironclaw_reborn_composition` — an extracted item is retrievable
  in-scope; excluded when expired / wrong-scope / wrong-audience / source
  revoked; the live snippet path now actually injects (regression test that
  `memory_snippets` is non-empty when authorized).
- **Security/privacy risks:** Document-derived memory leaking to a broader
  audience = the headline risk → audience authorization (needs PR 15's axis for
  group cases) + host-owned gate. Until the audience axis lands, restrict
  document-derived memory to the narrowest scope.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`; extraction off by default within
  it (no default-on extraction until posture approved).
- **Rollback:** Disable extraction; existing memory behavior unchanged.
- **Acceptance:** Memory items carry provenance+TTL, are use-time authorized at
  retrieval, and the snippet path is no longer dead.
- **Validation:**
  ```bash
  cargo test -p ironclaw_memory_native
  cargo test -p ironclaw_reborn_composition memory_learning
  ```

#### PR 15 — Memory supersession/versioning + revocation + audience axis

- **Goal:** Never silently overwrite memory on contradiction; supersede with a
  version link; support source revocation and a group/audience axis.
- **After-merge behavior:** A contradicting fact supersedes the prior with a
  `superseded_by` link; revoking a source makes downstream items unusable;
  document-audience scoping is enforced.
- **Depends on:** PR 14. **Contract change — needs sign-off** (exceeds the
  #5163 "relocate, don't change the API" charter).
- **System areas:**
  - Add neutral `supersede` / `revoke` operations to the `MemoryService` trait
    (`crates/ironclaw_memory/src/service.rs`) + native impl
    (`ironclaw_memory_native`). Today the trait is CRUD+search+tree+profile_set+
    retrieve_context only.
  - Add a group/audience axis: today `MemoryDocumentScope`
    (`crates/ironclaw_memory/src/path.rs`) is 4-axis (tenant/user/agent/project)
    with no audience. Add an `AudienceSet`/`Visibility` to `DocumentMetadata`
    (host-enforced at the `retain` filter + the host authorizer, **not**
    `ScopedFilesystem`, which only sees the 7-axis `ResourceScope`).
  - Add `Revoked`/`Superseded` variants to the memory significant-event kind.
- **Storage/schema:** Metadata + event additions (filesystem JSON). No SQL.
- **Tests:** `cargo test -p ironclaw_memory` + `-p ironclaw_memory_native`:
  contradiction supersedes (old retained, linked, hidden from retrieval);
  revocation hides downstream; audience parity — a memory from a doc shared with
  group G is omitted from a conversation whose participants aren't a superset of
  G (isolation-parity style).
- **Security/privacy risks:** The audience case is a privacy bar → parity test
  is release-blocking.
- **Rollout/flag:** Additive ops behind the learning gate; supersession/
  revocation safe to enable for memory hygiene independently.
- **Rollback:** Ops are additive; revert is clean (archived docs retained).
- **Acceptance:** No silent overwrite; revocation + audience enforced with parity
  tests.
- **Validation:**
  ```bash
  cargo test -p ironclaw_memory
  cargo test -p ironclaw_memory_native
  ```

---

### Phase F — Skill learning (staging, deterministic verify, promotion/demotion, gating)

#### PR 16 — Skill status lifecycle + selector gating + `SkillAdmin` port

- **Goal:** Give skills a candidate/active lifecycle the selector respects, plus
  a privileged host-internal admin port for the background pass.
- **After-merge behavior:** A skill can be `candidate`/`active`/`suppressed`/
  `demoted`; the selector excludes non-active; the coordinator can manage skills
  via a host-internal `SkillAdmin` binding (not the model tool surface).
- **Depends on:** PR 1.
- **System areas:**
  - `crates/ironclaw_skills`: add a status lifecycle (today only
    `SkillManifest.auto_activate: bool`); selector (`selector.rs`) excludes
    non-`active` candidates; keep gating (`gating.rs`) intact.
  - `SkillAdmin` privileged binding via `CapabilityVisibility::HostInternal`
    (`crates/ironclaw_extensions/src/v2.rs`, currently reserved/unused) rather
    than a raw management handle; expose only the admin-shaped surface the
    coordinator needs.
- **Storage/schema:** Skill status persisted in the existing user-skills store
  (`management.rs`). No SQL.
- **Tests:** `cargo test -p ironclaw_skills` — selector excludes `candidate`/
  `suppressed`/`demoted`; status transitions valid; `SkillAdmin` binding is
  host-internal (not visible to the model). Caller-level: a candidate skill is
  not activated by the selector in a real selection run.
- **Security/privacy risks:** A candidate skill leaking into activation =
  unverified capability → the selector exclusion test is the gate.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING` for the admin binding; status
  field is additive.
- **Rollback:** Default all skills `active` (current behavior) + remove the port.
- **Acceptance:** Non-active skills are never selected; admin port is
  host-internal.
- **Validation:** `cargo test -p ironclaw_skills`

#### PR 17 — Verifier-gated skill induction (reuse `ironclaw_skill_learning`) + promote/demote

- **Goal:** Distill candidate skills from successful traces and promote only
  after deterministic verification; auto-demote regressions.
- **After-merge behavior:** Distilled skill → `candidate`; deterministic
  execution/contract check → `active` (low-risk, same-scope auto-promote);
  high-risk/broad/capability-expanding → `PendingReview` (approvals); live
  failure-rate over threshold → demote to `candidate` and re-enter the loop.
- **Depends on:** PR 13, PR 16.
- **System areas:**
  - Reuse `ironclaw_skill_learning` (`distill_skill`, `refine_skill`,
    `SkillInferencePort`) + the composition sink
    (`crates/ironclaw_reborn_composition/src/skill_learning.rs`,
    `SkillLearningTurnEventSink`, `SkillWriter`, Jaccard dedup, `Sanitizer`).
    Add staging + promotion/demotion around it; do **not** inherit its defaults
    (keep extraction off-by-default; do not auto-activate without verification).
  - Deterministic verification by **execution** for runnable skills (free, no
    live model); per-skill success/failure tracking → demotion.
- **Storage/schema:** Skill store + status (PR 16). No SQL.
- **Tests:** `cargo test -p ironclaw_skill_learning` + caller-level
  `cargo test -p ironclaw_reborn_composition skill_learning`: distill →
  candidate; deterministic pass → active; high-risk → pending_review; regression
  → demote; sanitizer strips injection before install.
- **Security/privacy risks:** Capability-expanding skill auto-activating →
  high-risk routing + review; injection in distilled skill → existing sanitizer.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`; extraction off by default.
- **Rollback:** Disable induction; existing skills unaffected.
- **Acceptance:** Promotion is verifier-gated; high-risk is review-gated;
  regressions demote.
- **Validation:**
  ```bash
  cargo test -p ironclaw_skill_learning
  cargo test -p ironclaw_reborn_composition skill_learning
  ```

---

### Phase G — Product/admin controls, curation, persistence parity

#### PR 18 — Product facade + admin/inspect/pause/suppress/delete/approve surface

- **Goal:** User/admin control + inspection through the product facade, not raw
  stores.
- **After-merge behavior:** Web UI can list active/pending learnings, see "why
  learned / from where" (provenance), pause learning, pause usage, suppress/
  delete, and approve high-risk candidates.
- **Depends on:** PR 13 (proposals), PR 7 (rules), PR 14 (memory), PR 17
  (skills).
- **System areas (per `reborn-feature` skill, dependency order):**
  - Port + DTOs in `ironclaw_product_workflow` (`reborn_services/learning.rs`);
    re-export; add `RebornServicesApi` methods with **default "unavailable"
    bodies** (so existing fakes compile).
  - Adapter in `ironclaw_reborn_composition` (gated), wired in
    `build_webui_services` (`webui.rs`) via a `with_learning(...)` builder.
  - HTTP in `ironclaw_webui_v2`: route consts + patterns + `*_descriptor()`
    (`read_policy` for inspect, `mutation_policy` for pause/suppress/delete/
    approve); add to `webui_v2_routes()`; **update**
    `tests/webui_v2_descriptors_contract.rs` (`expected_table()`).
  - Frontend: `apiFetch` calls in `ironclaw_webui_v2_static` pages.
  - Inspection must redact provenance to safe summaries (no raw source content).
- **Storage/schema:** Reads PR 2 store + approvals. No SQL.
- **Tests:** `cargo test -p ironclaw_product_workflow`; `cargo test -p
  ironclaw_webui_v2` (descriptor contract); caller-level: pause-usage hides
  active rules from injection; suppress/delete reflected in a subsequent run;
  approve transitions a `PendingReview` proposal. `node --check` for changed JS.
- **Security/privacy risks:** Inspection leaking source content → safe-summary
  redaction; mutation authz keyed to the trusted caller, not the request body.
- **Rollout/flag:** Behind `webui-v2-beta` + `IRONCLAW_REBORN_LEARNING`.
- **Rollback:** Remove routes (update contract table) + facade methods.
- **Acceptance:** All controls work through the facade; descriptor contract
  updated; inspection is redaction-safe.
- **Validation:**
  ```bash
  cargo build -p ironclaw_product_workflow --all-features
  cargo build -p ironclaw_webui_v2 --features webui-v2-beta
  cargo test -p ironclaw_webui_v2
  node --check crates/ironclaw_webui_v2_static/js/.../learning-api.js
  ```

#### PR 19 — Curation pass (merge/expire/demote/budgets/archive) on the trigger poller

- **Goal:** Keep learned state from becoming noise: per-scope curation under
  budgets, archive-never-delete.
- **After-merge behavior:** A scheduled pass merges duplicate rules/memories,
  expires short-lived items, demotes regressed artifacts, enforces per-scope/
  per-tier budgets (consolidate when full), and archives (never hard-deletes).
- **Depends on:** PR 9, PR 14, PR 17.
- **System areas:**
  - `src/curation.rs`: per-scope curation invoked from
    `crates/ironclaw_reborn_composition/src/trigger_poller.rs`
    (`TriggerPollerWorker`). Runs within a scope; no cross-tenant reads.
  - Rule consolidation under the 4 KiB / per-tier sub-budgets; memory dedupe via
    a neutral `dedupe`/`supersede` op (content-hash merge if the semantic/
    embedding port is unavailable — it is descoped from the lift); per-family
    token budget.
  - Per-scope fairness budget for curation compute.
- **Storage/schema:** PR 2 store + memory (via contract). No SQL.
- **Tests:** `cargo test -p ironclaw_reborn_learning curation` — duplicates
  merge; expired excluded; budget overflow consolidates (no unbounded growth);
  archive retains (no hard delete); per-scope isolation (curator never reads
  another tenant).
- **Security/privacy risks:** Cross-tenant curation read = leak → per-scope
  isolation test.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`.
- **Rollback:** Disable the pass; artifacts persist under budget.
- **Acceptance:** Bounded growth proven; archive-not-delete; per-scope isolation.
- **Validation:** `cargo test -p ironclaw_reborn_learning curation`

#### PR 20 — Dual-backend persistence parity (PostgreSQL + libSQL) — gate before default-on

- **Goal:** Production-grade durable storage with both backends at parity, per
  `CLAUDE.md`, without depending on the root `ironclaw` crate.
- **After-merge behavior:** The learning store can run on PostgreSQL or libSQL
  with identical behavior, proven by a parity suite. Required before any
  production default flip.
- **Depends on:** PR 2 (and ideally PR 19, to freeze the access patterns).
- **System areas:** Follow the **hooks model**, not root `src/db`:
  - Keep the neutral store contract in `ironclaw_reborn_learning`.
  - Add `crates/ironclaw_reborn_learning_postgres` +
    `crates/ironclaw_reborn_learning_libsql` +
    `crates/ironclaw_reborn_learning_parity` (mirroring
    `ironclaw_hooks_postgres` / `ironclaw_hooks_libsql` / `ironclaw_hooks_parity`).
  - Migrations per backend (PG SQL files; libSQL translated schema). **Scoped
    keys only** — never a global `UNIQUE` like the legacy `tool_failures` table.
- **Storage/schema:** New scoped tables (`reborn_learned_rules`,
  `reborn_learned_failures`, `reborn_verifier_evidence`) with `(tenant, owner,
  …)` keys; both backends.
- **Tests:** `cargo test -p ironclaw_reborn_learning_parity` (parity across
  backends); `cargo test --features integration` (PostgreSQL via testcontainers);
  scope-isolation parity per backend.
- **Security/privacy risks:** A backend-specific scope-key bug = leak → parity +
  isolation tests per backend.
- **Rollout/flag:** Backend selected by existing config; learning still gated
  off by default.
- **Rollback:** Fall back to the filesystem store; tables are scoped + inert.
- **Acceptance:** Both backends pass the parity + isolation suite; no global
  keys.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning_parity
  cargo test --features integration
  ```

---

### Phase H — Measurement: stateful eval, live canaries, trigger immediacy

#### PR 21 — Stateful eval (Gain + transfer) — non-blocking nightly

- **Goal:** Measure whether the loop actually helps (or collapses), per scope.
- **After-merge behavior:** A nightly job runs task sequences against a
  **persistent** reborn agent and reports `Gain = score_stateful −
  score_stateless`, forward transfer, and backward transfer (catastrophic-
  forgetting alarm).
- **Depends on:** PR 12 (locked fixtures), PR 10 (real signatures to seed
  sequences).
- **System areas:** Stateful mode on `RebornBinaryE2EHarness`
  (`tests/support/reborn/harness.rs`): carry memory + learned rules + skills
  across a sequence; compute Gain normalized by headroom; seed sequences from
  real `failure_signature`s + generalization variants. Wire as non-blocking in
  `.github/workflows/nightly-deep-ci.yml` (and/or `reborn-e2e.yml`).
- **Storage/schema:** Eval artifacts only. No production schema.
- **Tests:** A deterministic stateful-vs-stateless harness test that reproduces a
  known win (a seeded rule improves a sequence) and a known regression (a bad
  rule erodes a prior skill → backward-transfer alarm fires).
- **Security/privacy risks:** Eval may call providers → budgeted, nightly,
  non-blocking.
- **Rollout/flag:** Nightly only; never a merge gate until it reliably
  reproduces known wins/regressions.
- **Rollback:** Remove the nightly lane.
- **Acceptance:** Gain + transfer reported per scope; known win/regression
  reproduced.
- **Validation:**
  ```bash
  # stateful eval extends the existing reborn e2e harness (tests/support/reborn/)
  cargo test --features integration reborn_stateful_gain -- --ignored   # nightly/attended
  ```

#### PR 22 — Live canaries for high-value failure signatures

- **Goal:** Continuously confirm "still fixed" (frozen fixtures can't).
- **After-merge behavior:** A periodic budgeted job live-re-runs top-N high-value
  signatures and alerts on recurrence; rule retirement is evidence-based (N
  canary cycles with zero recurrence even with the rule suppressed), never on
  fixture existence.
- **Depends on:** PR 12, PR 19.
- **System areas:** Canary pass on the trigger poller / nightly; budget-capped;
  reserved for high frequency/severity. Wire retirement logic into curation
  (PR 19): do not drop a rule just because a `reborn_qa` fixture exists.
- **Storage/schema:** Canary evidence on records. No SQL.
- **Tests:** Deterministic canary-scheduling test (top-N selection, budget cap);
  retirement only after N clean cycles with suppression.
- **Security/privacy risks:** Cost/egress → budget cap + high-value-only.
- **Rollout/flag:** Sub-gate of live verify; off by default.
- **Rollback:** Disable canaries; rules persist under budget.
- **Acceptance:** Recurrence alerts; retirement is evidence-based.
- **Validation:** `cargo test -p ironclaw_reborn_learning canary`

#### PR 23 — Trigger immediacy for corrections

- **Goal:** Treat explicit corrections / "don't do that again" / repeated
  signatures as first-class, bypassing batching — still off the user turn.
- **After-merge behavior:** A correction signal jumps the queue and runs the
  verified loop promptly; it still never self-certifies inside the active turn.
- **Depends on:** PR 9, PR 12.
- **System areas:** Priority lane in `src/coordinator.rs`; map correction
  milestones/flags to immediate enqueue-at-front. Everything else batches on the
  cron pass.
- **Storage/schema:** None.
- **Tests:** Correction signal is processed before batched signals; the user turn
  still returns immediately (caller-level).
- **Security/privacy risks:** Priority abuse (flood) → still under per-owner
  budget.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING`.
- **Rollback:** Remove the priority lane; all signals batch.
- **Acceptance:** Corrections prioritized; turn unaffected; no self-cert.
- **Validation:** `cargo test -p ironclaw_reborn_learning coordinator`

---

### Phase I — Fleet / cross-deployment learning (LATE, opt-in, research-grade)

> Not part of the initial implementation. Only the **harness/capability-general**
> tier is ever eligible; personal/project/tenant learnings never leave a
> deployment.

#### PR 24 — Telemetry-only anonymized failure-signature publishing

- **Goal:** The lowest-risk fleet slice: share *which* harness bugs are
  widespread, with no fix attached.
- **After-merge behavior:** With an explicit opt-in, a deployment may publish
  **de-identified** `failure_signature` telemetry (no fix, no content).
- **Depends on:** PR 10; Phases A–G stable.
- **System areas:** New crate `ironclaw_reborn_learning_fleet` (gated, isolated
  from the active path). De-identify via `ironclaw_safety` (`redact_exact_values`,
  `LeakDetector`); reject if genericity can't be proven; signed provenance
  (deployment id, model family/version). Eligible only if
  `scope = harness-general`.
- **Storage/schema:** Outbound telemetry envelope. No inbound trust.
- **Tests:** De-identification strips paths/names/secrets; non-generic signatures
  are rejected; nothing below harness-general is publishable.
- **Security/privacy risks:** Leakage via telemetry → leak-detector gate +
  scope eligibility; consent-gated.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING_FLEET` (off) + explicit operator
  opt-in.
- **Rollback:** Disable publishing; no inbound state to unwind.
- **Acceptance:** Only de-identified harness-general signatures leave; opt-in
  enforced.
- **Validation:** `cargo test -p ironclaw_reborn_learning_fleet deidentify`

#### PR 25 — Executable-skill sharing with corroboration + mandatory local re-verify

- **Goal:** Share the durable, verifiable unit (executable skills), never auto-
  trusting incoming contributions.
- **After-merge behavior:** A deployment may pull fleet skills as **candidates**;
  activation requires local re-verification (execution), corroboration across ≥N
  independent deployments, security scan, and staged/canary rollout with easy
  rollback. Free-text rules are shared last/never (templatized, low-trust,
  review).
- **Depends on:** PR 17, PR 22, PR 24.
- **System areas:** Publish (opt-in + consent + de-id + local verify + signed
  provenance); hub trust = corroboration not authority (≥N independent
  deployments; per model family/version); pull = candidates → local re-verify
  (execution for skills) → staged rollout (reuse PR 22 canary + PR 13 gate) →
  rollback. Security-scan executables (reuse skills guard / AST audit).
- **Storage/schema:** Fleet candidate staging (scoped, isolated from active).
- **Tests:** Incoming skill is inert until local re-verify passes; corroboration
  threshold enforced; security scan rejects malicious executables; rollback
  restores prior state.
- **Security/privacy risks:** Fleet-scale poisoning + privacy = the highest risk
  → corroboration + scan + local re-verify + review for high-blast-radius.
- **Rollout/flag:** `IRONCLAW_REBORN_LEARNING_FLEET` + per-deployment trust
  policy (corroboration count, auto-pull vs review = operator policy).
- **Rollback:** Demote pulled skills to candidate; disable fleet pull.
- **Acceptance:** No auto-trust; local re-verify mandatory; corroborated +
  scanned + staged + rollback-able.
- **Validation:**
  ```bash
  cargo test -p ironclaw_reborn_learning_fleet corroboration
  cargo test -p ironclaw_reborn_learning_fleet local_reverify
  ```

---

## 5. Required Phases — Coverage Confirmation

Mapping the task's required phases to the roadmap (renamed/reordered where code
inspection suggested a better split; rationale noted):

| Required phase | Covered by |
|---|---|
| Foundation (artifact model, source/scope/TTL/status, stores, gate) | Phase A (PR 1–3) |
| Signal emission (non-blocking learning signals) | Phase B PR 8 |
| Background coordinator (queue/worker, budget/backpressure, retries) | Phase C (PR 9–10) |
| Failure signatures (dedupe, escalate by frequency/severity) | Phase B PR 8 + Phase C PR 10 |
| Learned rules (scoped, budgeted injection + use-time auth) | Phase B (PR 4–7) |
| Verification (deterministic, live, replay-lock distinction) | Phase D (PR 11–13) |
| Memory learning (extraction, provenance, TTL, revocation, supersession/versioning) | Phase A PR 3 + Phase E (PR 14–15) |
| Skill learning (candidates, deterministic verify, promote/demote, selector gating) | Phase F (PR 16–17) |
| Product/admin controls (inspect/pause/suppress/delete/approve) | Phase G PR 18 |
| Curation (merge, expire, suppress, demote, per-scope budgets) | Phase G PR 19 |
| Stateful eval (Gain + regression detection) | Phase H PR 21 |
| Live canaries (recurring checks for high-value signatures) | Phase H PR 22 |
| Fleet learning (late, opt-in) | Phase I (PR 24–25) |

**Reordering rationale:** (1) The learned-rule **injection rails** (PR 4–7) are
split out ahead of the coordinator because they're the smallest end-to-end proof
and need no model calls. (2) **Multi-tenant scoping** is not a separate phase —
it's a Phase-A invariant baked into PR 1–2 (scope-keyed from line one) and
tested in every store PR, because retrofitting isolation is leak-prone. (3)
**Persistence parity** (PR 20) is added as an explicit gate before any production
default-on, satisfying `CLAUDE.md`'s dual-backend rule without forcing SQL into
the early dev slices. (4) **Self-modification gate** (PR 13) and **trigger
immediacy** (PR 23) are placed next to the machinery they constrain.

---

## 6. First PR Recommendation

**The smallest vertical slice that proves the architecture after #5163 is the
capture → scoped store → seeded rule → injection path, behind the flag, with no
model calls and no auto-promotion** — i.e. roadmap **PR 1 + PR 2 + PR 4 + PR 5 +
PR 6 + PR 7 + PR 8**, shipped as a tight sequence (or, if a single PR is
preferred, their minimal union).

Why this is the right first slice:

- It exercises the **whole spine** — a `Failed` milestone writes a scoped,
  `TurnOwner`-keyed failure record; a *seeded* active rule is use-time authorized
  and injected into the model request for the matching scope and **omitted** for
  any other tenant/user/agent/project — through the **real caller path**
  (`RebornBinaryE2EHarness` + request capture), not helper unit tests.
- It is **conservative**: no synthesis, no live verification, no memory/skill
  extraction, no curation, default-**off**. Seeded rules only. Zero new egress.
- It proves the **load-bearing invariants** cheaply: scope isolation (the privacy
  bar), `TurnOwner` attribution, fail-closed use-time authorization, budgeted
  injection, and non-blocking capture — the exact properties everything later
  depends on.
- It produces **no risky artifacts** and is trivially reversible (flag off /
  delete crate).

**If you must land one PR first**, land **PR 1** (artifact model + scoped
in-memory/filesystem store skeleton + architecture boundary rule + flag): it
locks the contracts every later PR consumes, carries zero runtime behavior and
zero risk, and makes the boundary test enforce "learning never reaches into
`ironclaw_memory_native` or the root crate" from the very first commit. The first
*observably useful* increment is **PR 7 + PR 8** together (seeded scoped rule
injected + failure captured through the caller path).

**Explicitly deferred from the first slice:** candidate synthesis (PR 9–10),
all verification (PR 11–13), memory extraction (PR 14–15), skill induction
(PR 16–17), admin UI (PR 18), curation (PR 19), SQL parity (PR 20), measurement
(PR 21–23), fleet (PR 24–25).

**Prerequisite this slice surfaces:** none beyond #5163 — it uses only existing
seams (milestone sink, identity context, scoped filesystem). The *next* slice's
prerequisite (memory learning) is the dead `memory_snippets` path, called out in
PR 14.

---

## 7. Non-Goals (Anti-Goals)

- **No closed-loop proxy optimizer.** Acceptance is "the failing case passes
  (live for behavior, execution for skills) + no fixture regression, reviewed for
  CODE/CONFIG" — never "an LLM-judge score went up." No DSPy/GEPA-style
  evolutionary prompt search in the product loop.
- **No legacy unbounded prompt overlay.** Do not port
  `crates/ironclaw_engine/src/executor/prompt.rs` (`prompt_overlay`) or the
  `mission.rs` loop. Learned rules live in a bounded, curated, budgeted store.
- **No unscoped/global learned state.** Never write a learning without a
  `ResourceScope`; never reuse the global `tool_failures` table
  (`UNIQUE(tool_name)`). Default narrowest; broaden only on evidence.
- **No silent memory overwrite on contradiction.** Supersede with a version link
  (`superseded_by`); archive, never hard-delete.
- **No auto-apply of code/config changes.** Code/config/egress/global/security
  candidates are `PendingReview` proposals routed to `ApprovalResolver`.
- **No fleet auto-trust.** Incoming contributions require corroboration (≥N
  independent deployments) + security scan + (high-blast-radius) review; pulled
  learnings are candidates that must pass **local re-verification** before
  activation. Outgoing requires de-identification + consent. Personal/project/
  tenant learnings never leave the deployment.
- **No user-facing turn blocking.** Learning is out-of-band, best-effort; a
  disabled/backed-up/broken learning system never delays or fails a turn.
- **No LLM self-certification as a gate.** "The model said fixed" is always a
  candidate, never a promotion.
- **No attribution to the raw sender.** Key to `TurnOwner`, never `TurnActor`.
- **No default-on production behavior** until product/security posture is
  explicit; live verification and fleet stay behind their own off-by-default
  sub-gates.

---

## 8. Final Checklist (Plan-Completeness)

The implementation team can consider this phased plan complete when all of the
following hold:

**Architecture & boundaries**
- [ ] `ironclaw_reborn_learning` depends only on neutral contracts; the
  `ironclaw_architecture` boundary rule forbids root `ironclaw`,
  `ironclaw_memory_native`, product/web (PR 1).
- [ ] Learning reads/writes memory only via the `ironclaw_memory` `MemoryService`
  contract, never native internals (PR 3, 14, 15).
- [ ] Admin/inspect UI goes through `RebornServicesApi`, never raw stores (PR 18).
- [ ] The coordinator uses host-internal privileged bindings
  (`CapabilityVisibility::HostInternal`), not the model tool surface (PR 16).

**Invariants**
- [ ] Every artifact has source provenance, scope, TTL/retention, and status —
  no constructor allows omission (PR 1).
- [ ] Use-time authorization is fail-closed across status/TTL/revocation/scope/
  audience/budget, with a passing exclusion test per dimension (PR 6, 14, 15).
- [ ] Learnings key to `TurnOwner`, proven for `Personal` **and** `SharedAgent`
  (PR 8).
- [ ] Scope-isolation parity tests pass for tenant/user/agent/project on every
  store, filesystem and SQL (PR 2, 20); audience parity for memory (PR 15).
- [ ] No raw transcripts/secrets/host-paths/tool-inputs in any artifact, asserted
  on the failure signature and rendered rule (PR 6, 8).
- [ ] Budgets enforced and bounded growth proven (PR 6, 14, 19).

**Verification**
- [ ] Replay is documented + used as a regression lock only; behavioral
  acceptance requires a live re-run (PR 11, 12).
- [ ] Executable skills verify by deterministic execution (PR 17).
- [ ] No candidate auto-promotes on replay or self-certification (PR 11–13).
- [ ] High-risk (code/config/egress/global/ambiguous-audience) is human/admin-
  gated; no auto-apply path exists (PR 13, 18).
- [ ] The self-modification gate runs the in-process replay set before any commit
  and rejects on drift (PR 13).

**Out-of-band & safety posture**
- [ ] Signal emission is non-blocking; a forced store error does not fail the
  turn (PR 8).
- [ ] The coordinator is bounded (queue/budget/backpressure/retry), fair per
  tenant/owner, and logs (never silently drops) at `debug!` (PR 9).
- [ ] Master gate + live-verify sub-gate + fleet sub-gate all default off;
  egress is disclosed (PR 7, 12, 24).

**Lifecycle completeness**
- [ ] All artifact types (memory, rule, skill, failure signature, proposal) plug
  into one lifecycle (candidate → pending/active/suppressed/expired/revoked/
  demoted) (PR 1, 13, 14, 17).
- [ ] Curation enforces per-scope budgets, supersedes/expires/demotes, and
  archives-never-deletes (PR 19).
- [ ] Rule retirement is evidence-based (canary cycles), not fixture-existence-
  based (PR 22).

**Measurement & fleet**
- [ ] Stateful Gain + forward/backward transfer reported per scope, nightly,
  non-blocking (PR 21).
- [ ] Fleet is last, opt-in, telemetry-first then skills-first, de-identified,
  corroborated, and locally re-verified before activation (PR 24–25).

**Process & docs**
- [ ] Dual-backend (PostgreSQL + libSQL) parity lands before any production
  default-on (PR 20).
- [ ] Caller-level tests exist wherever a helper gates a side effect
  (`.claude/rules/testing.md`).
- [ ] Docs updated where behavior/setup/feature-parity/security posture changes:
  `crates/ironclaw_memory/CLAUDE.md` (PR 3, 15), `crates/ironclaw_skills`
  docs (PR 16), `crates/ironclaw_reborn_learning/CLAUDE.md` (PR 1+), `.env.example`
  for new flags (PR 7, 12, 24), and a `docs/reborn/` posture note for the
  default-on decision (PR 20).
- [ ] Contract-crate edits (`MemoryService` curation ops, `DocumentMetadata`
  fields) are sign-off-gated with memory owners (PR 3, 15).

---

### Appendix — Verified anchor index (symbol · file)

- Milestone trigger — `LoopHostMilestoneSink` / `LoopHostMilestoneKind` ·
  `crates/ironclaw_turns/src/run_profile/milestones.rs`; durable tee
  `DurableLoopHostMilestoneSink` · `crates/ironclaw_reborn/src/milestone_events.rs`
- Verifier-gate philosophy — `LoopExitEvidencePort` / `LoopExitApplier` ·
  `crates/ironclaw_turns/src/loop_exit.rs`; reborn impl
  `ThreadCheckpointLoopExitEvidencePort` ·
  `crates/ironclaw_reborn/src/loop_exit_applier.rs`
- Scope — `ResourceScope` · `crates/ironclaw_host_api/src/resource.rs`;
  `TurnActor` · `crates/ironclaw_turns/src/scope.rs`; `TurnOwner`
  (`Personal`/`SharedAgent`) · `crates/ironclaw_turns/src/origin.rs`
- Scoped storage — `ScopedFilesystem` · `crates/ironclaw_filesystem/src/scoped.rs`;
  `RootFilesystem` · `crates/ironclaw_filesystem/src/root.rs`; isolation parity
  pattern · `tests/reborn_*_scope_isolation_parity.rs`
- Privileged binding — `CapabilityVisibility::HostInternal` (reserved/unused) ·
  `crates/ironclaw_extensions/src/v2.rs`
- Before-prompt injection — `HookedLoopPromptPort` (4 KiB) ·
  `crates/ironclaw_hooks/src/middleware/prompt_port.rs`; dispatch ·
  `crates/ironclaw_hooks/src/dispatch/mod.rs`
- Identity context — `HostIdentityContextSource` / `IdentityFileName` (~8000 tok) ·
  `crates/ironclaw_loop_support/src/identity_context.rs`;
  `DefaultSystemPromptIdentitySource` ·
  `crates/ironclaw_reborn_composition/src/default_system_prompt.rs`
- Protected paths — `PromptProtectedPathRegistry` / `PromptProtectedPathClass` /
  `DEFAULT_PROMPT_PROTECTED_PATHS` · `crates/ironclaw_memory/src/safety.rs`
- Memory contract — `MemoryService` · `crates/ironclaw_memory/src/service.rs`;
  `DocumentMetadata` · `crates/ironclaw_memory/src/metadata.rs`;
  `MemoryDocumentScope` (4-axis) · `crates/ironclaw_memory/src/path.rs`;
  dead snippet path `memory_snippets: Vec::new()` ·
  `crates/ironclaw_loop_support/src/lib.rs`; native `retrieve_context` retain
  filter · `crates/ironclaw_memory_native/src/service.rs`
- Verification substrate — `record_qa_phrase` · `tests/support/reborn/qa_trace.rs`;
  `RebornTraceReplayModelGateway` (`from_trace`/`take_step`/`matches_request`) ·
  `tests/support/reborn/model_replay.rs`; `RebornBinaryE2EHarness` ·
  `tests/support/reborn/harness.rs`; fixtures ·
  `tests/fixtures/llm_traces/reborn_qa/`; Tier-2 contracts ·
  `tests/reborn_qa_recorded_behavior.rs`; scrubber ·
  `scripts/ci/check-reborn-qa-fixtures.sh`; workflows ·
  `.github/workflows/{replay-gate,reborn-e2e,nightly-deep-ci}.yml`
- Composition — `factory.rs` (`build_reborn_services`), `runtime.rs`
  (`build_reborn_runtime`, `spawn_trigger_poller`), `runtime_input.rs`
  (`RebornRuntimeInput` `with_*`), `webui.rs` (`build_webui_services`),
  `trigger_poller.rs`, `milestone_events.rs`, `skill_learning.rs` ·
  `crates/ironclaw_reborn_composition/src/`
- Triggers — `TriggerPollerWorker` · `crates/ironclaw_triggers/src/worker.rs`
- Approvals — `ApprovalResolver` / `PersistentApprovalPolicy` ·
  `crates/ironclaw_approvals/src/lib.rs`
- Product facade — `RebornServicesApi` / `RebornServices` ·
  `crates/ironclaw_product_workflow/src/reborn_services.rs`
- WebUI v2 — `webui_v2_routes` / `*_descriptor` ·
  `crates/ironclaw_webui_v2/src/descriptors.rs`; contract ·
  `crates/ironclaw_webui_v2/tests/webui_v2_descriptors_contract.rs`
- Architecture boundaries — `boundary_rules` ·
  `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`
- Skill learning — `ironclaw_skill_learning` (`distill_skill`/`refine_skill`/
  `SkillInferencePort`); composition sink · `…/skill_learning.rs`; skills
  selector/gating/management · `crates/ironclaw_skills/src/{selector,gating,
  management}.rs`
- Legacy anti-patterns — `crates/ironclaw_engine/src/runtime/mission.rs`,
  `crates/ironclaw_engine/src/executor/prompt.rs`,
  `migrations/V3__tool_failures.sql` (global `UNIQUE(tool_name)`)
- Dual-backend model — `ironclaw_hooks` + `ironclaw_hooks_postgres` +
  `ironclaw_hooks_libsql` + `ironclaw_hooks_parity`
- Safety/redaction — `redact_exact_values` / `LeakDetector` ·
  `crates/ironclaw_safety/src/{redaction,leak_detector}.rs`
