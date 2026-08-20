# Subagent Background Delivery (Slices 1–2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a subagent child run in the background — the parent keeps working instead of parking — and durably deliver the child's result back to the parent whether it is mid-run, idle, or already finished.

**Architecture:** Blocking-mode subagents are already built end-to-end (spawn port → durable await-edge → resolver → parent resume) and switched off in production by a deny-filter. This plan does not fork any of it. It adds the one missing half: a provenance-tagged `activate()` re-activation primitive on the existing `TurnCoordinator`, and background delivery wired into the existing `AwaitEdgeResolver` drain path plus the existing (currently stubbed) `PostCapabilityStage::drain_settled` seam. Production stays deny-filtered; the e2e suite exercises the new path through the harness's own capability enablement.

**Tech Stack:** Rust (async/tokio, `async_trait`), serde, the IronClaw Reborn workspace under `crates/`. No new crate, no cargo feature, no new dependency.

**Spec:** `docs/internal/reborn/subagent-spawn/thread-harness-design.md` (canonical; §2, §5, §6, §8, §8.1, §8.2, §8.3 are the sections this plan implements). Shape decisions and slice order: `docs/internal/reborn/subagent-spawn/pr2-pr6-shape.md`. Recon map: `docs/internal/reborn/subagent-spawn/research-background-enable.md`.

## Global Constraints

Copied verbatim from the spec and the repo contract. Every task's requirements implicitly include this section.

- **No feature flag, no new crate, no stored counters, no new tables.** `disabled_capability_ids` is the sole on/off gate for the capability (design "Standing ruling"; §12 non-goals; `.claude/rules/cargo-features.md`).
- **Production stays deny-filtered in this plan.** `default_disabled_capability_ids()` in `crates/loop/ironclaw_turn_runner/src/runtime.rs:277-282` is NOT touched by slices 1–2. Do not remove `builtin.spawn_subagent` from it. Do not edit the tests that assert the capability is off (`tests/integration/tool_call.rs:756,797`, `tests/integration/tool_disclosure.rs:168`, `crates/app/ironclaw_composition/tests/service_factory.rs:386,413`).
- **Spawn creates and wires child runs only.** Planning, execution, capability calls, checkpointing, gates, retries, and completion continue through the existing runner/driver/executor path (root `AGENTS.md:47`).
- **Never mint `TrustedInboundTurnRequest`** and never call a trusted trigger submitter factory from any code this plan touches (root `AGENTS.md:49`; pinned by `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs:1400,1657`). A background wake is an ordinary provenance-tagged submission, not trusted ingress.
- **Three independently named budget constants**, never merged by a refactor: `DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS = 16` (existing), the SUBAGENT family `iteration_limit` 16 (existing), and `SYSTEM_WAKE_STREAK_CAP = 16` (new in Task 4). They coincide numerically by accident (design §8.3).
- **No `.unwrap()` / `.expect()` in production code** (tests are fine). Propagate with cause: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`. `.map_err(|_| …)` is banned outright and is not `silent-ok`-exemptible (`.claude/rules/error-handling.md`).
- **Typed identities only.** Never re-derive a run/thread id from a display string. New fixed-set values are enums with explicit serde (`.claude/rules/types.md`).
- **Additive serde on every persisted struct:** `#[serde(default, skip_serializing_if = "Option::is_none")]` so existing durable rows keep deserializing.
- **`ironclaw_loop_host` may not depend on `ironclaw_turn_runner`.** Traits go in `loop_host`/`agent_loop`, impls in `turn_runner` (design §4.1; `.claude/rules/type-placement.md` dependency-inversion category).
- **Never append to `crates/loop/ironclaw_turn_runner/src/subagent/completion_observer.rs`** (4,758 lines, over budget — design §4.6). New files aim under 800 lines (`.claude/rules/architecture.md`).
- **`crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs` is frozen at exactly 3 `test-support` methods** by `crates/app/ironclaw_architecture_tests/tests/reborn_struct_test_support_ratchet.rs:378`. Adding a 4th fails the ratchet.
- **Prompt text lives in `prompts/*.md`**, loaded with `include_str!()`, never inline in Rust (root `AGENTS.md`; pinned by `reborn_composition_boundaries`).
- **No `info!`/`warn!` from background tasks** — they corrupt the REPL TUI. Use `debug!` plus counters (repo `CLAUDE.md`; design §5.4).
- **Test-first, always.** Write the failing test, run it, watch it fail for the right reason, then implement. Never weaken an assertion to go green.
- **Run after any task that moves dependency edges, layer keys, or pinned guidance:** `cargo test -p ironclaw_architecture_tests`.

## Status (updated 2026-08-19)

**Slice 1 is complete and committed** — Tasks 1–5, each test-first, each with
its verification command run and green. Commits:

| Task | Commit | Evidence |
|---|---|---|
| 1 | `2ee01e5a2` | wire-string test red → green |
| 2 | `aea2c81e3` | metadata round-trip + legacy-default tests; workspace check clean across 26 changed files |
| 3 | `4a953a5d9` | 4 tests, incl. the fail-closed default |
| 4 | `8ae9832f7` | 2 tests, **mutation-verified** (flipping the sort and dropping the kind filter each kill the test) |
| 5 | `fc0a2f8f8` | 6 predicate tests + 3 caller tests, **mutation-verified** (disabling the cap kills 2) |

Gates run at slice close: `cargo test -p ironclaw_turns -p ironclaw_processes`
(12 suites green), `cargo test -p ironclaw_architecture_tests` (all green after
re-pinning the `host_api` contracts size ceiling, which `ActivationProvenance`
tipped over), `cargo check --workspace --all-targets` clean, `cargo fmt`.
Both standing invariants re-verified: the production deny-filter is untouched
and the diff adds no trusted-ingress minting.

**Two gates could not be run in this environment, and are outstanding:**
- `cargo clippy` — the only Rust toolchain on this box has no `clippy`
  component and no `rustup` to add one.
- Anything requiring the WebUI build — its build script shells out to
  `pnpm` via a corepack that is broken here (`ERR_VM_DYNAMIC_IMPORT_CALLBACK_MISSING`),
  unrelated to these changes. Everything else was verified with
  `SKIP_FRONTEND_BUILD=1`, the build script's own sanctioned opt-out.

**Slice 2 (Tasks 6–10) has not been started.** Task 6 is the next action.

---

### Task 1 ✅ DONE: `ActivationProvenance` vocabulary

Adds the enum that tags *why* a run was created. Nothing reads it yet — Task 2 persists it, Tasks 3/4 act on it.

**Files:**
- Modify: `crates/contracts/ironclaw_host_api/src/turn.rs` (add beside `TurnStatus`, which begins at line 866)
- Test: same file, in its existing `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces: `ironclaw_host_api::turn::ActivationProvenance` with variants `Human`, `ParentAgent`, `System`; wire strings `"human"`, `"parent_agent"`, `"system"`. Re-exported through `ironclaw_turns`' prelude (which already re-exports `TurnStatus` and friends), so downstream crates write `use ironclaw_turns::ActivationProvenance;`.

- [x] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/contracts/ironclaw_host_api/src/turn.rs`:

```rust
#[test]
fn activation_provenance_wire_strings_are_snake_case() {
    use super::ActivationProvenance;

    assert_eq!(
        serde_json::to_value(ActivationProvenance::Human).expect("serialize"),
        serde_json::json!("human")
    );
    assert_eq!(
        serde_json::to_value(ActivationProvenance::ParentAgent).expect("serialize"),
        serde_json::json!("parent_agent")
    );
    assert_eq!(
        serde_json::to_value(ActivationProvenance::System).expect("serialize"),
        serde_json::json!("system")
    );

    let round_tripped: ActivationProvenance =
        serde_json::from_value(serde_json::json!("parent_agent")).expect("deserialize");
    assert_eq!(round_tripped, ActivationProvenance::ParentAgent);
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_host_api activation_provenance_wire_strings_are_snake_case`
Expected: FAIL to compile — `cannot find type ActivationProvenance in this scope`.

- [x] **Step 3: Write minimal implementation**

Add to `crates/contracts/ironclaw_host_api/src/turn.rs`, immediately before the `TurnStatus` definition:

```rust
/// Why a run was created on its thread. Set once at run creation and
/// immutable thereafter — the derived streak caps (design §6, §8.3) read
/// windows of this field instead of maintaining a stored counter.
///
/// `Human` is the ordinary case and resets both streaks. `ParentAgent` tags a
/// parent re-activating one of its own children (`subagent_extend`).
/// `System` tags a background subagent completion waking its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationProvenance {
    Human,
    ParentAgent,
    System,
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_host_api activation_provenance_wire_strings_are_snake_case`
Expected: PASS.

- [x] **Step 5: Re-export from the turns prelude**

Find the prelude re-export list in `crates/kernel/ironclaw_turns/src/lib.rs` (the one that already names `TurnStatus`) and add `ActivationProvenance` to it, alphabetically within its group. Confirm with:

Run: `cargo build -p ironclaw_turns`
Expected: builds clean.

- [x] **Step 6: Commit**

```bash
git add crates/contracts/ironclaw_host_api/src/turn.rs crates/kernel/ironclaw_turns/src/lib.rs
git commit -m "feat(turns): add ActivationProvenance vocabulary for subagent activation tagging"
```

---

### Task 2 ✅ DONE: Persist provenance on the run record

Threads the provenance from a submission into durable process metadata and back out onto `TurnRunRecord`. This is the field Tasks 3 and 4 read.

**Files:**
- Modify: `crates/kernel/ironclaw_turns/src/request.rs` (`SubmitTurnRequest`, struct begins line 50; lineage fields at 73-77 are the pattern to copy)
- Modify: `crates/kernel/ironclaw_turns/src/agent_turn_runtime.rs` (`TurnRunRecord`, lineage fields around lines 181-185)
- Modify: `crates/kernel/ironclaw_turns/src/process_projection/metadata.rs` (`AgentTurnProcessStateMetadata`, `subagent_depth` at line 101 is the sibling to copy)
- Modify: `crates/kernel/ironclaw_turns/src/process_projection/runtime.rs` (write side: wherever `subagent_depth` is written into the metadata at submit; read side: `turn_run_record_from_process_snapshot`, line 1083, which reads `metadata.subagent_depth` at line 1130)
- Modify: all `SubmitTurnRequest { … }` literal sites (46 across `crates/` and `tests/` — see Step 5)
- Test: `crates/kernel/ironclaw_turns/tests/` (add to the existing process-projection round-trip suite; if none exists, create `crates/kernel/ironclaw_turns/tests/activation_provenance.rs`)

**Interfaces:**
- Consumes: `ActivationProvenance` from Task 1.
- Produces:
  - `SubmitTurnRequest.subagent_activation_provenance: Option<ActivationProvenance>` (serde-defaulted; `None` means an ordinary human-initiated submission).
  - `TurnRunRecord.subagent_activation_provenance: Option<ActivationProvenance>` — set once at run creation, never mutated.
  - `AgentTurnProcessStateMetadata.subagent_activation_provenance: Option<ActivationProvenance>` — the durable carrier.

- [x] **Step 1: Write the failing test**

Create `crates/kernel/ironclaw_turns/tests/activation_provenance.rs`. Follow the construction helpers used by the crate's existing projection tests — open `crates/kernel/ironclaw_turns/src/process_projection/runtime.rs`'s own `#[cfg(test)] mod tests` and reuse its snapshot fixture builder rather than inventing one:

```rust
//! Provenance must survive the process-metadata round trip: a submission
//! tagged `System` must project back onto `TurnRunRecord` as `System`, and an
//! untagged legacy row must project as `None` (not a default variant).

use ironclaw_turns::ActivationProvenance;

#[test]
fn provenance_round_trips_through_agent_turn_process_metadata() {
    let metadata_json = serde_json::json!({
        "turn_id": "turn-1",
        "accepted_message_ref": "msg-1",
        "resolved_run_profile_id": "default",
        "resolved_run_profile_version": 1,
        "subagent_activation_provenance": "system",
    });

    let metadata: ironclaw_turns::process_projection::AgentTurnProcessStateMetadata =
        serde_json::from_value(metadata_json).expect("metadata deserializes");

    assert_eq!(
        metadata.subagent_activation_provenance,
        Some(ActivationProvenance::System),
        "a System-tagged submission must round-trip through durable metadata"
    );
}

#[test]
fn legacy_metadata_without_provenance_projects_as_none() {
    let metadata_json = serde_json::json!({
        "turn_id": "turn-1",
        "accepted_message_ref": "msg-1",
        "resolved_run_profile_id": "default",
        "resolved_run_profile_version": 1,
    });

    let metadata: ironclaw_turns::process_projection::AgentTurnProcessStateMetadata =
        serde_json::from_value(metadata_json).expect("legacy metadata deserializes");

    assert_eq!(
        metadata.subagent_activation_provenance, None,
        "rows written before this field existed must stay readable as None"
    );
}
```

If `AgentTurnProcessStateMetadata` is not public from the crate root, place these two tests inside `crates/kernel/ironclaw_turns/src/process_projection/metadata.rs`'s own `#[cfg(test)] mod tests` instead and drop the module path prefix. Do not add a `pub` export just to make an external test compile.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_turns provenance_round_trips_through_agent_turn_process_metadata`
Expected: FAIL — `no field subagent_activation_provenance on type AgentTurnProcessStateMetadata`.

- [x] **Step 3: Add the field to the durable metadata carrier**

In `crates/kernel/ironclaw_turns/src/process_projection/metadata.rs`, immediately after the `subagent_depth` field (line 101):

```rust
    /// Why this run was activated on its thread (design §6/§8.3). Set once at
    /// run creation, never mutated. Absent on rows written before the field
    /// existed, and on every ordinary human-initiated submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_activation_provenance: Option<ActivationProvenance>,
```

Add `ActivationProvenance` to that file's `use` list.

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p ironclaw_turns provenance_round_trips_through_agent_turn_process_metadata legacy_metadata_without_provenance_projects_as_none`
Expected: PASS (both).

- [x] **Step 5: Add the field to the request and record, and fix every construction site**

In `crates/kernel/ironclaw_turns/src/request.rs`, at the end of `SubmitTurnRequest` (after `product_context`):

```rust
    /// Why this submission is activating the thread (design §6/§8.3).
    /// `None` — the default for every ordinary caller — is an untagged,
    /// human-initiated submission. Only the coordinator's `activate()` entry
    /// point sets this to `ParentAgent` or `System`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_activation_provenance: Option<ActivationProvenance>,
```

In `crates/kernel/ironclaw_turns/src/agent_turn_runtime.rs`, at the end of `TurnRunRecord` (after `resume_disposition`), the identical field with this doc comment:

```rust
    /// Why this run was activated (design §6/§8.3). Immutable after creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_activation_provenance: Option<ActivationProvenance>,
```

Then wire the projection in `crates/kernel/ironclaw_turns/src/process_projection/runtime.rs`:
- Read side, in `turn_run_record_from_process_snapshot` (line 1083), beside `subagent_depth: metadata.subagent_depth,` at line 1130, add:
  ```rust
        subagent_activation_provenance: metadata.subagent_activation_provenance,
  ```
- Write side: find where the submission builds `AgentTurnProcessStateMetadata` (grep `subagent_depth:` in this file — the write site sets it from the request) and copy the provenance across the same way:
  ```rust
        subagent_activation_provenance: request.subagent_activation_provenance,
  ```

Now fix the construction sites. Both structs are built with explicit field lists (no `..Default::default()` spread anywhere), so the compiler will name every one:

```bash
cargo build --workspace --all-targets 2>&1 | grep -c "missing field .subagent_activation_provenance"
```

Add `subagent_activation_provenance: None,` to each reported site. There are 46 `SubmitTurnRequest { … }` literals plus the `TurnRunRecord { … }` literals. Every one of them is an ordinary submission or a test fixture — `None` is correct for all of them. Do not set a non-`None` value anywhere in this task; Task 3 owns the only caller that does.

- [x] **Step 6: Run the full crate suite plus a workspace build**

Run: `cargo test -p ironclaw_turns`
Expected: PASS.

Run: `cargo build --workspace --all-targets`
Expected: builds clean, zero `missing field` errors.

- [x] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(turns): persist subagent activation provenance on the run record"
```

---

### Task 3 ✅ DONE: The `activate()` re-activation primitive

The single primitive for re-activating an existing thread with a provenance tag. Background delivery (Task 7) calls it; `subagent_extend` (a later slice) will too.

**Files:**
- Modify: `crates/kernel/ironclaw_turns/src/request.rs` (add `ActivateThreadRequest`)
- Modify: `crates/kernel/ironclaw_turns/src/coordinator.rs` (`TurnCoordinator` trait at line 125; `DefaultTurnCoordinator` impl at line 484; the `Arc<C>` blanket impl at line 771)
- Test: `crates/kernel/ironclaw_turns/src/coordinator.rs`'s own `#[cfg(test)] mod tests` (it already has one — the declared-limits test lives at line 829)

**Interfaces:**
- Consumes: `ActivationProvenance` (Task 1); `SubmitTurnRequest.subagent_activation_provenance` (Task 2).
- Produces:
  ```rust
  pub struct ActivateThreadRequest {
      pub scope: TurnScope,
      pub actor: TurnActor,
      pub accepted_message_ref: AcceptedMessageRef,
      pub provenance: ActivationProvenance,
      pub idempotency_key: IdempotencyKey,
      pub received_at: TurnTimestamp,
      pub requested_run_profile: Option<RunProfileRequest>,
  }
  ```
  and `TurnCoordinator::activate(&self, request: ActivateThreadRequest) -> Result<SubmitTurnResponse, TurnError>`, with a fail-closed default impl. `TurnError::ThreadBusy`-shaped rejection when a run is already live on the thread comes from the existing admission path unchanged — `activate` adds no new busy handling.

- [x] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/kernel/ironclaw_turns/src/coordinator.rs`. Reuse the module's existing fixture helpers for building a coordinator — read the `declared_limits_narrow_profile_ceilings_and_never_widen_them` test at line 829 first and copy its setup shape rather than inventing new fixtures:

```rust
/// `activate()` is `submit_turn` plus a provenance tag — it must reach the
/// same admission path and stamp the request so the run record carries the
/// tag. Anything else (a second submission path, a bypass of admission) would
/// violate the design's "spawn creates and wires child runs only" rule.
#[tokio::test]
async fn activate_submits_through_admission_and_stamps_provenance() {
    let (coordinator, recorder) = recording_coordinator_fixture();

    let response = coordinator
        .activate(ActivateThreadRequest {
            scope: test_scope(),
            actor: test_actor(),
            accepted_message_ref: test_message_ref(),
            provenance: ActivationProvenance::System,
            idempotency_key: IdempotencyKey::new("activate-test-1").expect("valid key"),
            received_at: chrono::Utc::now(),
            requested_run_profile: None,
        })
        .await
        .expect("activate succeeds on an idle thread");

    assert!(matches!(response, SubmitTurnResponse::Accepted { .. }));

    let submitted = recorder.submitted_requests();
    assert_eq!(submitted.len(), 1, "activate must submit exactly one turn");
    assert_eq!(
        submitted[0].subagent_activation_provenance,
        Some(ActivationProvenance::System),
        "activate must stamp the provenance onto the submission it builds"
    );
}

/// A coordinator that has not opted into activation must refuse, not silently
/// fall through to an untagged submission (fail-closed default).
#[tokio::test]
async fn default_activate_impl_refuses_rather_than_submitting_untagged() {
    struct NonActivatingCoordinator;

    #[async_trait]
    impl TurnCoordinator for NonActivatingCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            panic!("default activate() must not reach submit_turn");
        }
        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn retry_turn(
            &self,
            _request: RetryTurnRequest,
        ) -> Result<RetryTurnResponse, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unreachable!("not exercised by this test")
        }
        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unreachable!("not exercised by this test")
        }
    }

    let error = NonActivatingCoordinator
        .activate(ActivateThreadRequest {
            scope: test_scope(),
            actor: test_actor(),
            accepted_message_ref: test_message_ref(),
            provenance: ActivationProvenance::System,
            idempotency_key: IdempotencyKey::new("activate-test-2").expect("valid key"),
            received_at: chrono::Utc::now(),
            requested_run_profile: None,
        })
        .await
        .expect_err("default impl must refuse");

    assert!(
        matches!(error, TurnError::InvalidRequest { .. }),
        "default activate() must fail closed, got {error:?}"
    );
}
```

`recording_coordinator_fixture()`, `test_scope()`, `test_actor()`, and `test_message_ref()` are helpers you add alongside these tests if the module does not already provide equivalents. The recorder must capture the full `SubmitTurnRequest` the coordinator built (not just a count) — per `.claude/rules/testing.md`, doubles capture every argument the production caller passes.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_turns activate_submits_through_admission_and_stamps_provenance default_activate_impl_refuses_rather_than_submitting_untagged`
Expected: FAIL to compile — `no method named activate`, `cannot find struct ActivateThreadRequest`.

- [x] **Step 3: Add the request type**

In `crates/kernel/ironclaw_turns/src/request.rs`, after `SubmitChildRunRequest`:

```rust
/// Re-activate an existing thread with an explicit provenance tag — the single
/// re-activation primitive (design §1). This is deliberately *not* a second
/// admission path: `activate` builds an ordinary [`SubmitTurnRequest`], so
/// one-active-run exclusivity, idempotency replay, and busy rejection all
/// behave exactly as they do for any other submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateThreadRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub accepted_message_ref: AcceptedMessageRef,
    pub provenance: ActivationProvenance,
    pub idempotency_key: IdempotencyKey,
    pub received_at: TurnTimestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_run_profile: Option<RunProfileRequest>,
}
```

- [x] **Step 4: Add the trait method with a fail-closed default**

In `crates/kernel/ironclaw_turns/src/coordinator.rs`, inside `pub trait TurnCoordinator` (line 125), after `submit_turn`:

```rust
    /// Re-activate an existing thread with a provenance tag (design §1, §8).
    ///
    /// Defaults to a refusal rather than an untagged `submit_turn` fallthrough:
    /// a coordinator that has not opted into activation semantics must not
    /// silently create runs whose provenance the streak caps then cannot see.
    /// `DefaultTurnCoordinator` provides the real implementation; this default
    /// exists so the many test doubles of this trait need not each restate it
    /// (same reason `abort_prepared_turn` above carries one).
    async fn activate(
        &self,
        _request: ActivateThreadRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Err(TurnError::InvalidRequest {
            reason: "this coordinator does not support thread activation".to_string(),
        })
    }
```

- [x] **Step 5: Implement it on `DefaultTurnCoordinator`**

In the `impl<S> TurnCoordinator for DefaultTurnCoordinator<S>` block (line 484), after `submit_turn`:

```rust
    async fn activate(
        &self,
        request: ActivateThreadRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        // Deliberately routed through this coordinator's own `submit_turn`:
        // admission, idempotency replay, profile resolution, and the wake
        // notification are shared with every other submission. The only
        // difference an activation makes is the provenance stamp.
        self.submit_turn(SubmitTurnRequest {
            scope: request.scope,
            actor: request.actor,
            accepted_message_ref: request.accepted_message_ref,
            requested_run_profile: request.requested_run_profile,
            output_contract: None,
            requested_model: None,
            idempotency_key: request.idempotency_key,
            received_at: request.received_at,
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            subagent_activation_provenance: Some(request.provenance),
        })
        .await
    }
```

If the compiler reports fields of `SubmitTurnRequest` this literal is missing, add them with the same neutral values the crate's other non-child submissions use — do not invent new semantics here.

- [x] **Step 6: Forward it on the `Arc<C>` blanket impl**

In `impl<C> TurnCoordinator for Arc<C>` (line 771), add the forwarding method beside the others:

```rust
    async fn activate(
        &self,
        request: ActivateThreadRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        (**self).activate(request).await
    }
```

Without this, `Arc<dyn TurnCoordinator>` callers would silently get the trait's refusing default instead of the real implementation.

- [x] **Step 7: Run tests to verify they pass**

Run: `cargo test -p ironclaw_turns activate_submits_through_admission_and_stamps_provenance default_activate_impl_refuses_rather_than_submitting_untagged`
Expected: PASS (both).

Run: `cargo test -p ironclaw_turns`
Expected: PASS.

- [x] **Step 8: Commit**

```bash
git add crates/kernel/ironclaw_turns/src/request.rs crates/kernel/ironclaw_turns/src/coordinator.rs
git commit -m "feat(turns): add provenance-tagged activate() re-activation primitive"
```

---

### Task 4 ✅ DONE: Bounded newest-first run query for a thread

The streak caps (Task 5, and `subagent_extend` in a later slice) are *derived* — no stored counter — so they need to read a bounded window of a thread's most recent runs. Nothing today returns newest-first bounded run records for a thread; `children_of` is parent-keyed and unbounded.

**Files:**
- Modify: `crates/kernel/ironclaw_processes/src/journal_store/rows.rs` (add `recent_processes_for_scope` beside `processes_for_scope` at line 691; `ordered_process_query` at line 1104 hardcodes `SortDirection::Ascending` at line 1131)
- Modify: `crates/kernel/ironclaw_processes/src/journal.rs` (`ProcessSnapshotSource`, line 946-954)
- Modify: `crates/kernel/ironclaw_processes/src/journal_store.rs` (the `impl ProcessSnapshotSource for ProcessJournalStore<F>` block at line 715-736)
- Modify: `crates/kernel/ironclaw_turns/src/process_projection/store_adapter.rs:35-47`
- Modify: `crates/kernel/ironclaw_turns/src/agent_turn_runtime.rs` (`AgentTurnSpawnTreeRuntimePort`, line 73)
- Modify: `crates/kernel/ironclaw_turns/src/process_projection/runtime.rs` (impl block at line 668; mirror `children_of` at 684-698)
- Modify (stubs): `crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs:947`, `crates/loop/ironclaw_turn_runner/src/structured_finalization/tests.rs:189`
- Test: `crates/kernel/ironclaw_processes/tests/process_journal_store_contract.rs` (existing `process_snapshots` cases at lines 823 and 2536 are the pattern)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `AgentTurnSpawnTreeRuntimePort::recent_runs_for_thread(&self, scope: &TurnScope, limit: u32) -> Result<Vec<TurnRunRecord>, TurnError>` — newest-first, at most `limit` `AgentTurn` records for that exact thread scope.

**Why this shape (do not redesign it):** the index this needs already exists — `ordered_index("process_scope_v3", &["scope_key", "created_at", "process_id"])` at `rows.rs:214-217`, where `scope_key` (`rows/keys.rs:59-69`) already includes `thread_id`. `SortDirection::Descending` is already implemented and tested in all three backends (`postgres.rs:689-699`, `libsql.rs:970-980`, `in_memory.rs:460,496`). So there is **no new index, no migration, and no per-backend SQL work** — index declaration never backfills (`ironclaw_filesystem/CONTRACT.md:168-169`), which is exactly why reusing `process_scope_v3` matters.

**Two traps, both load-bearing:**
1. `process_scope_v3` is **not** keyed on `process_kind`, and a thread's scope also holds `CapabilityInvocation`/`CapabilityInvocationState` processes (`crates/kernel/ironclaw_processes/src/invocation_state.rs:695`). A raw `LIMIT 16` can therefore come back entirely non-`AgentTurn` and yield zero runs. **Over-fetch**: page the descending keyset until `limit` `AgentTurn` records are collected or a hard page budget is spent. Do not declare a new kind-keyed index — that would need the offline `migrate_row_native_indexes` step (`journal_store.rs:404-444`).
2. The architecture gate `crates/app/ironclaw_architecture_tests/tests/reborn_process_storage_scan_gate.rs:30-41` forbids `.query(` and `.tail_bounded(` inside `journal_store.rs` outside two named migration/startup functions. It scans **only** `journal_store.rs`, not `rows.rs`, and its own self-test at lines 210-223 confirms `.query_ordered(` is deliberately not scanned. So: put the enumeration in `rows.rs` and reach storage through `.query_ordered(`. Do not add a `.query(` call to `journal_store.rs`.

- [x] **Step 1: Write the failing test**

In `crates/kernel/ironclaw_processes/tests/process_journal_store_contract.rs`, beside the existing `process_snapshots` coverage. Use the file's existing backend-parametrized harness so the case runs on every backend — read the test at line 823 first and copy its setup:

```rust
/// The streak caps read a bounded newest-first window of a thread's runs.
/// Two properties matter and both are load-bearing: ordering must be
/// newest-first (an ascending read returns the wrong end of history), and the
/// limit must be honoured against AgentTurn rows specifically — a thread's
/// scope also holds capability-invocation processes, so a naive LIMIT can be
/// filled entirely by non-AgentTurn rows.
#[tokio::test]
async fn recent_process_snapshots_returns_newest_first_bounded_by_limit() {
    let harness = contract_harness().await;
    let scope = harness.thread_scope();

    // Seed 5 AgentTurn processes, oldest first, interleaved with a
    // capability-invocation process that must not consume the limit.
    let mut agent_turn_ids = Vec::new();
    for index in 0..5 {
        agent_turn_ids.push(harness.seed_agent_turn_process(&scope, index).await);
        harness.seed_capability_invocation_process(&scope, index).await;
    }

    let recent = harness
        .store()
        .recent_process_snapshots(&scope, 3)
        .await
        .expect("recent snapshots");

    assert_eq!(recent.len(), 3, "limit must bound the AgentTurn result count");
    let returned: Vec<_> = recent.iter().map(|s| s.process_id).collect();
    let expected: Vec<_> = agent_turn_ids.iter().rev().take(3).copied().collect();
    assert_eq!(
        returned, expected,
        "must return the newest 3 AgentTurn processes, newest first"
    );
}
```

`seed_capability_invocation_process` may not exist in the harness — if not, add it alongside the existing agent-turn seeder rather than dropping the interleaving, because the interleaving is the whole point of trap 1 above.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_processes recent_process_snapshots_returns_newest_first_bounded_by_limit`
Expected: FAIL to compile — `no method named recent_process_snapshots`.

- [x] **Step 3: Add the descending bounded row query**

In `crates/kernel/ironclaw_processes/src/journal_store/rows.rs`, give `ordered_process_query` (line 1104) a direction parameter, replacing the hardcoded `SortDirection::Ascending` at line 1131, and pass `SortDirection::Ascending` explicitly at its four existing call sites (`query_claim_candidates` line 930, `query_active_conflict` line 972, `query_running_quota_rows` line 995, `query_expired_processes` line 1046) so their behavior is unchanged.

Then add, beside `processes_for_scope` (line 691):

```rust
/// Newest-first, bounded read of one scope's `AgentTurn` processes.
///
/// `process_scope_v3` is keyed on `(scope_key, created_at, process_id)` and
/// deliberately not on `process_kind`, while a thread's scope also holds
/// capability-invocation processes. A flat `LIMIT` would therefore be
/// satisfiable entirely by non-`AgentTurn` rows, so this pages the descending
/// keyset until it has `limit` agent-turn rows or spends its page budget.
pub(super) async fn recent_agent_turn_processes_for_scope<F>(
    filesystem: &F,
    scope: &ResourceScope,
    limit: u32,
) -> Result<Vec<JournaledProcessSnapshot>, ProcessJournalStoreError>
where
    F: RootFilesystem + ?Sized,
{
    const MAX_PAGES: u32 = 8;

    let mut collected: Vec<JournaledProcessSnapshot> = Vec::new();
    let mut page = 0;
    while collected.len() < limit as usize && page < MAX_PAGES {
        let batch = ordered_process_query(
            filesystem,
            "process_scope_v3",
            scope_key_filters(scope)?,
            /* sort_key */ "created_at",
            SortDirection::Descending,
            /* limit */ limit * 2,
            /* offset_page */ page,
        )
        .await?;
        let exhausted = batch.is_empty();
        collected.extend(
            batch
                .into_iter()
                .filter(|snapshot| snapshot.process_kind == ProcessKind::AgentTurn),
        );
        if exhausted {
            break;
        }
        page += 1;
    }
    collected.truncate(limit as usize);
    Ok(collected)
}
```

Match `ordered_process_query`'s real signature — read it at line 1104 and adapt the call above to whatever its filter/paging parameters actually are. Do not change its paging contract; only add the direction.

- [x] **Step 4: Add the port method and thread it through**

`crates/kernel/ironclaw_processes/src/journal.rs`, on `ProcessSnapshotSource` (line 946):

```rust
    /// Newest-first, bounded read of one scope's agent-turn processes. Unlike
    /// [`Self::process_snapshots`] this is explicitly bounded — callers that
    /// need a fixed recent window must not enumerate the whole scope.
    async fn recent_process_snapshots(
        &self,
        scope: &ResourceScope,
        limit: u32,
    ) -> Result<Vec<JournaledProcessSnapshot>, Self::Error>;
```

Implement it in `journal_store.rs`'s `impl ProcessSnapshotSource for ProcessJournalStore<F>` (line 715), mirroring `process_snapshots` at 721-735: call `ensure_materialized()`, keep the existing `ResourceScope::system()` rejection at 726-731, then delegate to `rows::recent_agent_turn_processes_for_scope`. **Introduce no `.query(` or `.tail_bounded(` call in this file.**

Forward it through `crates/kernel/ironclaw_turns/src/process_projection/store_adapter.rs:35-47`.

- [x] **Step 5: Run the storage test and the architecture gate**

Run: `cargo test -p ironclaw_processes recent_process_snapshots_returns_newest_first_bounded_by_limit`
Expected: PASS on every backend the harness parametrizes.

Run: `cargo test -p ironclaw_architecture_tests --test reborn_process_storage_scan_gate`
Expected: PASS.

- [x] **Step 6: Expose it as `recent_runs_for_thread` on the turn port**

In `crates/kernel/ironclaw_turns/src/agent_turn_runtime.rs`, on `AgentTurnSpawnTreeRuntimePort` (line 73) — **not** on `AgentTurnRuntimePort`, which has six test doubles this would break:

```rust
    /// The newest `limit` runs on this exact thread scope, newest first.
    /// Bounded by construction: the derived activation-streak caps
    /// (design §6, §8.3) read a fixed window instead of keeping a counter.
    async fn recent_runs_for_thread(
        &self,
        scope: &TurnScope,
        limit: u32,
    ) -> Result<Vec<TurnRunRecord>, TurnError>;
```

Implement it in `crates/kernel/ironclaw_turns/src/process_projection/runtime.rs`'s impl block (line 668), mirroring `children_of` (684-698): call `self.snapshots.recent_process_snapshots(&scope.to_resource_scope(), limit)`, then map through `turn_run_record_from_process_snapshot` (line 1083).

Add stub implementations to the two test doubles — `crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs:947` and `crates/loop/ironclaw_turn_runner/src/structured_finalization/tests.rs:189` — returning `Ok(Vec::new())`.

- [x] **Step 7: Verify the whole workspace still builds and the projection test passes**

Run: `cargo test -p ironclaw_turns`
Expected: PASS.

Run: `cargo build --workspace --all-targets`
Expected: clean.

- [x] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(processes): add bounded newest-first agent-turn query for a thread scope"
```

---

### Task 5 ✅ DONE: System-wake streak cap in `activate()`

Bounds the autonomous spawn → settle → wake → spawn cycle. Without it a parent that spawns a fresh child on every background completion loops forever with no human in it, under every existing cap.

**Files:**
- Create: `crates/kernel/ironclaw_turns/src/activation_streak.rs`
- Modify: `crates/kernel/ironclaw_turns/src/lib.rs` (declare the module)
- Modify: `crates/kernel/ironclaw_turns/src/coordinator.rs` (`DefaultTurnCoordinator::activate` from Task 3)
- Test: `crates/kernel/ironclaw_turns/src/activation_streak.rs`'s own `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `ActivationProvenance` (Task 1); `TurnRunRecord.subagent_activation_provenance` (Task 2); `TurnCoordinator::activate` (Task 3); `recent_runs_for_thread` (Task 4).
- Produces: `pub const SYSTEM_WAKE_STREAK_CAP: u32 = 16;` and `pub fn system_wake_admitted(recent: &[TurnRunRecord]) -> bool` — pure windowing logic over an already-fetched window, so it is unit-testable without a store.

**Spec rules that must hold exactly (design §8.3):**
- Fetch at most `K = SYSTEM_WAKE_STREAK_CAP = 16` records of `Human`/`System` provenance. `ParentAgent` runs are **excluded from the fetch**, not filtered after — they neither reset nor count.
- All `K` are `System` with no `Human` → refuse the pending `System` activation.
- A `Human` anywhere in the window, **or** fewer than `K` records in history → admit.
- A refusal loses nothing durable: the settled edge stays `Settled` and is drained by the run-start sweep or the boot pass. The cap gates the *reactive wake* only, never delivery.
- `SYSTEM_WAKE_STREAK_CAP` stays its own named constant. It must not be merged with `DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS` or the SUBAGENT family `iteration_limit`, which also happen to be 16.

- [x] **Step 1: Write the failing test**

Create `crates/kernel/ironclaw_turns/src/activation_streak.rs` with the tests first (implementation stub returning `unimplemented!()` so the file compiles):

```rust
#[cfg(test)]
mod tests {
    use super::{SYSTEM_WAKE_STREAK_CAP, system_wake_admitted};
    use crate::ActivationProvenance;

    /// Build a newest-first window of run records carrying only the field the
    /// cap reads. `provenances[0]` is the newest run.
    fn window(provenances: &[ActivationProvenance]) -> Vec<crate::TurnRunRecord> {
        provenances
            .iter()
            .map(|provenance| test_run_record_with_provenance(Some(*provenance)))
            .collect()
    }

    #[test]
    fn under_cap_consecutive_system_wakes_are_admitted() {
        let recent = window(&[ActivationProvenance::System; 15]);
        assert!(
            system_wake_admitted(&recent),
            "15 consecutive System wakes is under the cap of {SYSTEM_WAKE_STREAK_CAP}"
        );
    }

    #[test]
    fn cap_plus_one_consecutive_system_wakes_is_refused() {
        let recent = window(&[ActivationProvenance::System; 16]);
        assert!(
            !system_wake_admitted(&recent),
            "a full window of System runs means the pending wake is the 17th consecutive one"
        );
    }

    #[test]
    fn a_human_activation_anywhere_in_the_window_resets_the_streak() {
        let mut provenances = [ActivationProvenance::System; 16];
        provenances[15] = ActivationProvenance::Human;
        assert!(
            system_wake_admitted(&window(&provenances)),
            "a Human run anywhere in the window resets the streak"
        );
    }

    #[test]
    fn a_short_history_is_admitted() {
        let recent = window(&[ActivationProvenance::System; 3]);
        assert!(
            system_wake_admitted(&recent),
            "a young thread with fewer than {SYSTEM_WAKE_STREAK_CAP} records must be admitted"
        );
    }

    #[test]
    fn untagged_legacy_runs_count_as_human_and_reset_the_streak() {
        let mut recent = window(&[ActivationProvenance::System; 15]);
        recent.push(test_run_record_with_provenance(None));
        assert!(
            system_wake_admitted(&recent),
            "an untagged run is an ordinary human-initiated run and must reset the streak"
        );
    }
}
```

`test_run_record_with_provenance(Option<ActivationProvenance>) -> TurnRunRecord` is a fixture you add in the same `mod tests` — build it from whatever minimal `TurnRunRecord` constructor the crate's existing tests use (`crates/kernel/ironclaw_turns/src/agent_turn_runtime.rs` has a `#[cfg(test)] mod tests` with record fixtures — reuse it).

Note the `ParentAgent` exclusion is **not** tested here: it is enforced by the *fetch*, not this function, and is covered in Step 4.

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw_turns activation_streak`
Expected: FAIL — `not yet implemented` panic from the stub (or a compile error if the stub is absent).

- [x] **Step 3: Write minimal implementation**

In the same file, above the test module:

```rust
//! Derived activation-streak caps (design §6, §8.3).
//!
//! Deliberately no stored counter and no new component: each cap is a
//! predicate over a bounded, newest-first window of the thread's own run
//! records, fetched with the complementary provenance excluded.

use crate::{ActivationProvenance, TurnRunRecord};

/// Consecutive `System`-provenance activations allowed on one thread before
/// the reactive wake is refused (design §8.3).
///
/// Independently named on purpose. It coincides numerically with the
/// 16-descendant spawn-tree cap and the SUBAGENT family's 16-iteration limit,
/// and those three budgets must never be merged by a refactor.
pub const SYSTEM_WAKE_STREAK_CAP: u32 = 16;

/// Whether a pending `System` activation may be admitted, given the thread's
/// newest-first window of `Human`/`System` runs (`ParentAgent` runs are
/// excluded by the caller's fetch, so they neither count nor reset).
///
/// Refusing costs nothing durable: the settled await-edge stays `Settled` and
/// drains via the run-start sweep or the boot pass. This gates the reactive
/// wake only, never delivery itself.
pub fn system_wake_admitted(recent: &[TurnRunRecord]) -> bool {
    if recent.len() < SYSTEM_WAKE_STREAK_CAP as usize {
        return true;
    }
    recent
        .iter()
        .take(SYSTEM_WAKE_STREAK_CAP as usize)
        .any(|record| {
            record.subagent_activation_provenance != Some(ActivationProvenance::System)
        })
}
```

Declare `pub mod activation_streak;` in `crates/kernel/ironclaw_turns/src/lib.rs`.

- [x] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_turns activation_streak`
Expected: PASS (all five).

- [x] **Step 5: Enforce the cap inside `activate()`**

In `DefaultTurnCoordinator::activate` (Task 3), before building the `SubmitTurnRequest`, add the guard. It applies to `System` provenance only:

```rust
        if request.provenance == ActivationProvenance::System {
            // ParentAgent runs are excluded from the window entirely, and must
            // be excluded from the FETCH, not filtered after it: filtering a
            // K-sized fetch returns fewer than K records whenever ParentAgent
            // runs are interleaved, and a short window reads as "streak not
            // established" and admits.
            let fetch_limit = SYSTEM_WAKE_STREAK_CAP.saturating_mul(SYSTEM_WAKE_WINDOW_OVERFETCH);
            let raw = self
                .store
                .recent_runs_for_thread(&request.scope, fetch_limit)
                .await?;
            // A retry of an already-accepted activation must reach submit_turn's
            // durable idempotency replay, not be refused by the run it created.
            let recent = raw
                .iter()
                .filter(|record| record.accepted_message_ref != request.accepted_message_ref)
                .filter(|record| {
                    record.subagent_activation_provenance != Some(ActivationProvenance::ParentAgent)
                })
                .take(SYSTEM_WAKE_STREAK_CAP as usize)
                .cloned()
                .collect::<Vec<_>>();
            // Fail closed when the window could not be established: a FULL fetch
            // that still cannot yield a cap-sized non-ParentAgent window means
            // the streak is unknown, not absent. A short fetch is a young
            // thread and still admits.
            let window_crowded_out = u32::try_from(raw.len()).unwrap_or(u32::MAX) >= fetch_limit
                && (recent.len() as u32) < SYSTEM_WAKE_STREAK_CAP;
            if window_crowded_out || !system_wake_admitted(&recent) {
                return Err(TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::SystemWakeStreak,
                )));
            }
        }
```

`spawn_tree_runtime()` is the accessor for the `AgentTurnSpawnTreeRuntimePort`. `DefaultTurnCoordinator` holds `process_runtime`/`store` as `AgentTurnRuntimePort` — if neither is typed as the spawn-tree port, thread the spawn-tree port into `DefaultTurnCoordinator` as a required (**not** `Option<Arc<…>>` — see `.claude/rules/architecture.md` smell 2) constructor dependency, and update its construction sites. If that turns out to widen the blast radius beyond this task, **stop and report** rather than reaching for an `Option`.

Note the filter above is a belt-and-braces second pass: the *fetch* is what the spec requires to exclude `ParentAgent`, and `recent_runs_for_thread` returns all kinds. Keeping the filter here (rather than pushing a provenance predicate into the storage query) keeps Task 4's query general and the exclusion visible at the policy site.

- [x] **Step 6: Add the caller-level test**

Add to `crates/kernel/ironclaw_turns/src/coordinator.rs`'s test module, beside Task 3's tests:

```rust
/// The cap must be enforced at the caller, not just in the predicate — and a
/// refusal must not submit a turn.
#[tokio::test]
async fn activate_refuses_system_wake_past_the_streak_cap_without_submitting() {
    let (coordinator, recorder) =
        recording_coordinator_with_recent_runs(vec![
            ActivationProvenance::System;
            SYSTEM_WAKE_STREAK_CAP as usize
        ]);

    let error = coordinator
        .activate(ActivateThreadRequest {
            scope: test_scope(),
            actor: test_actor(),
            accepted_message_ref: test_message_ref(),
            provenance: ActivationProvenance::System,
            idempotency_key: IdempotencyKey::new("streak-cap-1").expect("valid key"),
            received_at: chrono::Utc::now(),
            requested_run_profile: None,
        })
        .await
        .expect_err("a saturated System streak must refuse");

    assert!(matches!(error, TurnError::InvalidRequest { .. }));
    assert!(
        recorder.submitted_requests().is_empty(),
        "a refused wake must not submit a turn"
    );
}

/// ParentAgent runs are excluded from the System window: a thread whose recent
/// history is all ParentAgent must still admit a System wake.
#[tokio::test]
async fn parent_agent_runs_do_not_saturate_the_system_streak() {
    let (coordinator, _recorder) =
        recording_coordinator_with_recent_runs(vec![
            ActivationProvenance::ParentAgent;
            SYSTEM_WAKE_STREAK_CAP as usize
        ]);

    coordinator
        .activate(ActivateThreadRequest {
            scope: test_scope(),
            actor: test_actor(),
            accepted_message_ref: test_message_ref(),
            provenance: ActivationProvenance::System,
            idempotency_key: IdempotencyKey::new("streak-cap-2").expect("valid key"),
            received_at: chrono::Utc::now(),
            requested_run_profile: None,
        })
        .await
        .expect("ParentAgent history must not block a System wake");
}
```

- [x] **Step 7: Run tests to verify they pass**

Run: `cargo test -p ironclaw_turns`
Expected: PASS.

Run: `cargo clippy -p ironclaw_turns --all-targets --all-features -- -D warnings`
Expected: zero warnings.

- [x] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(turns): bound autonomous System activations with a derived streak cap"
```

**Slice 1 is complete here.** Before starting Task 6, run the slice gate:

```bash
cargo test -p ironclaw_turns -p ironclaw_processes
cargo test -p ironclaw_architecture_tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

---

### Task 6: Accept `mode: "background"` at the spawn codec

Deletes the rejection. This is pure unblocking — `SpawnSubagentMode::{Blocking, Background}` already exists (`subagent_spawn_port.rs:181-186`) and already rides durably on `AwaitedChildSetRecord.mode` (line 273) and `SubagentThreadMetadata.mode` (line 294). Nothing downstream constructs `Background` yet; Task 7 does that.

**Files:**
- Modify: `crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs` (`SpawnSubagentArgs` line 188; `TryFrom<SpawnSubagentWireArgs>` lines 218-242; `background_subagents_disabled()` lines 1500-1503; `build_spawn_subagent_parameters_schema` lines 62-107)
- Modify: `crates/loop/ironclaw_loop_host/prompts/spawn_subagent_description.md`
- Test: `crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs`

**Interfaces:**
- Consumes: nothing from slice 1.
- Produces: `SpawnSubagentArgs.mode: SpawnSubagentMode` (defaulting to `Blocking`), populated from either the `mode` wire field or the legacy `run_in_background` boolean.

- [ ] **Step 1: Write the failing test**

In `crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs`. Several existing tests in this file assert the string `"background subagents are disabled"` — find them with `rg -n "background subagents are disabled" crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs` and **replace** those assertions with the ones below rather than adding duplicates beside them.

```rust
/// Background mode is accepted at the codec now that delivery exists.
/// Both spellings must land on the same typed mode: the explicit `mode` field
/// and the legacy `run_in_background` boolean.
#[test]
fn background_mode_is_accepted_from_both_wire_spellings() {
    let explicit: SpawnSubagentArgs = serde_json::from_value::<SpawnSubagentWireArgs>(
        serde_json::json!({
            "subagent_type": "general",
            "task": "research the competitor set",
            "mode": "background",
        }),
    )
    .expect("wire args parse")
    .try_into()
    .expect("background mode is accepted");
    assert_eq!(explicit.mode, SpawnSubagentMode::Background);

    let legacy: SpawnSubagentArgs = serde_json::from_value::<SpawnSubagentWireArgs>(
        serde_json::json!({
            "subagent_type": "general",
            "task": "research the competitor set",
            "run_in_background": true,
        }),
    )
    .expect("wire args parse")
    .try_into()
    .expect("legacy run_in_background is accepted");
    assert_eq!(legacy.mode, SpawnSubagentMode::Background);
}

/// Omitting the mode must stay blocking — the historical default, and the one
/// every existing caller and stored payload relies on.
#[test]
fn omitted_mode_defaults_to_blocking() {
    let args: SpawnSubagentArgs = serde_json::from_value::<SpawnSubagentWireArgs>(
        serde_json::json!({
            "subagent_type": "general",
            "task": "summarize this file",
        }),
    )
    .expect("wire args parse")
    .try_into()
    .expect("args convert");
    assert_eq!(args.mode, SpawnSubagentMode::Blocking);
}

/// The model cannot ask for background mode it cannot see: the advertised
/// schema must carry the mode enum, and must keep rejecting unknown fields.
#[test]
fn advertised_schema_exposes_the_mode_enum() {
    let schema = build_spawn_subagent_parameters_schema(&[]);
    let mode = &schema["properties"]["mode"];
    assert_eq!(mode["enum"], serde_json::json!(["blocking", "background"]));
    assert_eq!(
        schema["additionalProperties"],
        serde_json::json!(false),
        "the schema must keep rejecting unknown fields"
    );
    assert_eq!(
        schema["required"],
        serde_json::json!(["subagent_type", "task"]),
        "mode stays optional — omitting it means blocking"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw_loop_host background_mode_is_accepted_from_both_wire_spellings omitted_mode_defaults_to_blocking advertised_schema_exposes_the_mode_enum`
Expected: FAIL — `no field mode on SpawnSubagentArgs`, and the schema assertion fails because `properties.mode` is absent.

- [ ] **Step 3: Add `mode` to the typed args and delete the rejection**

In `crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs`, add to `SpawnSubagentArgs` (line 188):

```rust
    #[serde(default = "blocking_mode_default")]
    pub mode: SpawnSubagentMode,
```

with

```rust
fn blocking_mode_default() -> SpawnSubagentMode {
    SpawnSubagentMode::Blocking
}
```

Replace the body of `TryFrom<SpawnSubagentWireArgs>` (lines 221-241) — delete both rejection branches at 222-227 and map the two spellings onto the typed mode instead:

```rust
    fn try_from(value: SpawnSubagentWireArgs) -> Result<Self, Self::Error> {
        // Two accepted spellings, one typed mode. `run_in_background: true` is
        // the legacy boolean; `mode` is the current field. When both are
        // present they must agree — silently preferring one would let a
        // caller believe it asked for the other.
        let mode = match (value.mode, value.run_in_background) {
            (Some(SpawnSubagentWireMode::Background), _) | (None, true) => {
                SpawnSubagentMode::Background
            }
            (Some(SpawnSubagentWireMode::Blocking), false) | (None, false) => {
                SpawnSubagentMode::Blocking
            }
            (Some(SpawnSubagentWireMode::Blocking), true) => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "conflicting spawn mode: mode is \"blocking\" but run_in_background is true",
                ));
            }
        };
        if value.task.len() > DEFAULT_SUBAGENT_GOAL_MAX_BYTES {
            return Err(spawn_goal_field_too_large("task", value.task.len()));
        }
        if let Some(handoff) = value.handoff.as_deref()
            && handoff.len() > DEFAULT_SUBAGENT_GOAL_MAX_BYTES
        {
            return Err(spawn_goal_field_too_large("handoff", handoff.len()));
        }
        Ok(Self {
            subagent_kind: value.subagent_kind,
            task: value.task,
            handoff: value.handoff,
            mode,
        })
    }
```

Delete `background_subagents_disabled()` (lines 1500-1503) entirely. The compiler will flag any remaining caller.

- [ ] **Step 4: Advertise `mode` in the schema**

In `build_spawn_subagent_parameters_schema` (line 62), add to the `properties` object beside `handoff`:

```rust
            "mode": {
                "type": "string",
                "enum": ["blocking", "background"],
                "description": "How to wait for the child. \"blocking\" (the default) pauses this turn until the child finishes and returns its result inline. \"background\" returns immediately and delivers the child's result later, letting you keep working meanwhile."
            }
```

Leave `required` as `["subagent_type", "task"]` and `additionalProperties` as `false`.

- [ ] **Step 5: Update the model-facing description**

`crates/loop/ironclaw_loop_host/prompts/spawn_subagent_description.md` currently states the child "runs to completion and returns its final result" — blocking-only wording with no background affordance. Rewrite it to describe both modes and when to pick each: blocking when the answer is needed to continue this turn, background when the work is independent and you have other work to do meanwhile. Keep the file's existing tone and length; the prompt text stays in this file and is never inlined into Rust.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p ironclaw_loop_host`
Expected: PASS, including the rewritten assertions that previously expected a rejection.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(subagent): accept background spawn mode at the codec and advertise it in the schema"
```

---

### Task 7: Background spawn returns immediately instead of parking

`finish_spawn` hard-codes `SpawnSubagentMode::Blocking` at line 908 regardless of what the caller asked for, and always returns `resolution::await_dependent_run` (line 1101), which parks the parent on a gate. Background mode must thread the requested mode through and return the non-suspending channel instead.

**Files:**
- Modify: `crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs` (`finish_spawn` lines 889-1109)
- Test: `crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs`

**Interfaces:**
- Consumes: `SpawnSubagentArgs.mode` (Task 6).
- Produces: a background spawn resolves to `resolution::spawned_child_run(...)` (`crates/contracts/ironclaw_loop_contracts/src/resolution.rs:213`) — the existing `Done`/`ChildSpawned` channel, documented there as "a NON-suspending child run whose result the executor appends before continuing". Blocking spawns keep returning `resolution::await_dependent_run` unchanged.

- [ ] **Step 1: Write the failing test**

In `crates/loop/ironclaw_loop_host/src/subagent_spawn_port/tests.rs`, beside the existing spawn tests (reuse their harness — the file already builds a port with `StaticAgentTurnRuntime` and `StaticCoordinator`):

```rust
/// A background spawn must not park the parent. The durable records must also
/// carry Background, because recovery reads the mode off them to decide
/// whether a settled edge resumes a gate or activates a thread.
#[tokio::test]
async fn background_spawn_returns_without_parking_and_records_background_mode() {
    let harness = spawn_port_harness().await;

    let resolution = harness
        .invoke_spawn(serde_json::json!({
            "subagent_type": "general",
            "task": "research the competitor set",
            "mode": "background",
        }))
        .await
        .expect("background spawn succeeds");

    assert!(
        matches!(resolution, Resolution::Done(_)),
        "background spawn must resolve on the non-suspending channel, got {resolution:?}"
    );

    let metadata = harness.recorded_child_thread_metadata();
    assert_eq!(
        metadata.mode,
        SpawnSubagentMode::Background,
        "child thread metadata must record Background so recovery routes delivery correctly"
    );
    let awaited = harness.recorded_awaited_child_set_record();
    assert_eq!(awaited.mode, SpawnSubagentMode::Background);
}

/// Blocking stays exactly as it was — this task must not change the shipped
/// blocking behavior in any way.
#[tokio::test]
async fn blocking_spawn_still_parks_the_parent_on_the_dependent_run_gate() {
    let harness = spawn_port_harness().await;

    let resolution = harness
        .invoke_spawn(serde_json::json!({
            "subagent_type": "general",
            "task": "summarize this file",
        }))
        .await
        .expect("blocking spawn succeeds");

    assert!(
        matches!(resolution, Resolution::Suspended(_)),
        "blocking spawn must still park on the dependent-run gate, got {resolution:?}"
    );
    assert_eq!(
        harness.recorded_child_thread_metadata().mode,
        SpawnSubagentMode::Blocking
    );
}
```

`spawn_port_harness()`, `invoke_spawn`, `recorded_child_thread_metadata`, and `recorded_awaited_child_set_record` are helpers — the file already has equivalents for the existing spawn tests. Reuse them; do not add a fourth `test-support` method to `SubagentSpawnCapabilityPort` itself, which is frozen at exactly 3 by `reborn_struct_test_support_ratchet.rs:378`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw_loop_host background_spawn_returns_without_parking_and_records_background_mode blocking_spawn_still_parks_the_parent_on_the_dependent_run_gate`
Expected: the background test FAILS (resolution is `Suspended`, metadata mode is `Blocking`); the blocking test PASSES already.

- [ ] **Step 3: Thread the mode through `finish_spawn`**

In `finish_spawn`, replace line 908:

```rust
        let mode = SpawnSubagentMode::Blocking;
```

with

```rust
        let mode = args.mode;
```

Everything downstream already takes `mode` as a parameter — the `spawn_result_payload` call at line 917 and the `SubagentThreadMetadata` at line 947 both pass it through, so no other write site changes.

- [ ] **Step 4: Return the non-suspending resolution for background**

Replace the tail of `finish_spawn` (the `Ok(resolution::await_dependent_run(...))` at lines 1101-1109) with a branch on mode:

```rust
        match mode {
            // Blocking: park this turn on the dependent-run gate. The
            // resolver resumes it once every sibling in the gate group has
            // settled (design §1).
            SpawnSubagentMode::Blocking => {
                let loop_gate_ref =
                    LoopGateRef::new(gate_ref.as_str()).map_err(invalid_static_ref)?;
                Ok(resolution::await_dependent_run(
                    loop_gate_ref,
                    result_ref,
                    safe_summary("subagent spawned; waiting for completion"),
                    write_result.byte_len,
                    write_result.model_observation,
                )
                .resolution)
            }
            // Background: do not park. The placeholder result is already in
            // the transcript; the resolver overwrites it in place and wakes
            // this thread when the child settles (design §8). The edge stays
            // `open` across this parent run's own terminal transition — for
            // background that is the normal delivery case, never abandonment
            // (design §2).
            SpawnSubagentMode::Background => Ok(resolution::spawned_child_run(
                child_run_id,
                result_ref,
                safe_summary("subagent spawned in the background; result will arrive later"),
                write_result.byte_len,
                write_result.model_observation,
            )),
        }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ironclaw_loop_host`
Expected: PASS (both new tests and the whole existing suite — the blocking path must be untouched).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(subagent): background spawns resolve without parking the parent turn"
```

---

### Task 8: Wake a background parent when its child settles

The delivery half. In blocking mode the parent sits on a gate and `resume_parent` (`resolver.rs:489`) wakes it. A background parent is not parked — it may be mid-run, idle, or already finished — so it is woken by a `System`-provenance `activate()` instead.

**Files:**
- Modify: `crates/loop/ironclaw_turn_runner/src/subagent/await_edge/resolver.rs` (`drain_settled_group` lines 673-746; the `resume_parent` call is at line 737)
- Test: `crates/loop/ironclaw_turn_runner/src/subagent/await_edge/resolver.rs`'s own `#[cfg(test)] mod tests` (the mixed-status group test at line 1431 is the pattern; the module already has a `StaticCoordinator` double at line 1375)

**Interfaces:**
- Consumes: `TurnCoordinator::activate` + `ActivateThreadRequest` (Task 3); `AwaitEdge.mode` (already durable).
- Produces: no new public API. `drain_settled_group` gains a private mode branch.

**Spec rules (design §8, §8.2 trigger 1):**
- `ThreadBusy` from the activation is a **benign no-op** — leave the edge `Settled` and let the run-start sweep or boot pass drain it. Do not retry in place, do not fail the drain, do not abandon the edge.
- Exactly **one** activation attempt per settled child; the edge's `Settled` state is the dedupe.
- The transcript write happens *before* the wake, exactly as it already does for blocking — a woken parent must find its result already in place.

- [ ] **Step 1: Write the failing test**

Add to the resolver's test module:

```rust
/// A background child's completion must wake its parent thread with a
/// System-provenance activation rather than resuming a gate the parent is not
/// sitting on.
#[tokio::test]
async fn background_edge_activates_the_parent_instead_of_resuming_a_gate() {
    let harness = resolver_harness_with_mode(SpawnSubagentMode::Background).await;

    harness.settle_child_as_completed().await;

    assert!(
        harness.coordinator().resumes().is_empty(),
        "a background parent is not parked, so nothing may call resume_turn"
    );
    let activations = harness.coordinator().activations();
    assert_eq!(activations.len(), 1, "exactly one wake per settled child");
    assert_eq!(activations[0].provenance, ActivationProvenance::System);
    assert_eq!(activations[0].scope.thread_id, harness.parent_thread_id());

    assert_eq!(
        harness.transcript_updates().len(),
        1,
        "the child's framed result must be written into the parent transcript"
    );
}

/// ThreadBusy is benign: the parent is mid-run, the edge stays settled, and
/// the run-start sweep or boot pass drains it later. Losing the result here
/// would be silent data loss.
#[tokio::test]
async fn background_wake_losing_to_thread_busy_leaves_the_edge_settled() {
    let harness = resolver_harness_with_mode(SpawnSubagentMode::Background).await;
    harness.coordinator().fail_next_activation_with(TurnError::ThreadBusy);

    let outcome = harness
        .settle_child_as_completed()
        .await
        .expect("a busy parent must not fail the drain");

    assert_eq!(outcome, ResolveOutcome::Drained);
    assert_eq!(
        harness.edge_state().await,
        Some(AwaitEdgeState::Settled),
        "the edge must stay settled so a later trigger can drain it"
    );
}

/// Blocking is untouched by this task.
#[tokio::test]
async fn blocking_edge_still_resumes_the_gate_and_never_activates() {
    let harness = resolver_harness_with_mode(SpawnSubagentMode::Blocking).await;

    harness.settle_child_as_completed().await;

    assert_eq!(harness.coordinator().resumes().len(), 1);
    assert!(
        harness.coordinator().activations().is_empty(),
        "a blocking parent is resumed through its gate, never activated"
    );
}
```

Extend the module's existing `StaticCoordinator` double (line 1375) with an `activate` override that records `ActivateThreadRequest`s and can be primed to fail — per `.claude/rules/testing.md`, capture the whole request, not just a count.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw_turn_runner background_edge_activates_the_parent_instead_of_resuming_a_gate background_wake_losing_to_thread_busy_leaves_the_edge_settled`
Expected: FAIL — the background case currently calls `resume_turn`, so `activations()` is empty and `resumes()` is not.

- [ ] **Step 3: Branch the drain on edge mode**

In `drain_settled_group` (line 673), replace the unconditional `self.resume_parent(...)` at line 737:

```rust
        match edge.mode {
            // Blocking: the parent is parked on the dependent-run gate.
            SpawnSubagentMode::Blocking => {
                self.resume_parent(&edge, parent_run_id, driving_child_run_id)
                    .await?;
            }
            // Background: the parent is not parked — it may be mid-run, idle,
            // or already terminal. Wake the thread with a System activation
            // (design §8). The framed results are already written above, so a
            // woken parent finds them in place.
            SpawnSubagentMode::Background => {
                match self.activate_parent(&edge, parent_run_id).await {
                    Ok(()) => {}
                    // Benign: the parent is running right now. Leave the edge
                    // settled — §8.2's trigger 2 (run-start sweep) or trigger
                    // 3 (boot pass) drains it. Retrying here would just race
                    // the same live run.
                    Err(TurnError::ThreadBusy) => {
                        debug!(
                            parent_run_id = %parent_run_id,
                            "background subagent wake lost to a live parent run; \
                             edge stays settled for the next drain trigger"
                        );
                        return Ok(ResolveOutcome::Drained);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
```

The `ThreadBusy` early return must come **before** the `close_edge` loop at lines 740-743 — closing the edges would delete the very records the later triggers need.

Add the private helper beside `resume_parent`:

```rust
    /// Wake a background parent's thread with a `System`-provenance
    /// activation. Mirrors `resume_parent`'s use of the actor and scope cached
    /// on `edge.parent_run_context` at open/reconstruct time — never a live
    /// lookup, which deadlocks from inside the child's own commit-observer
    /// callback (see `parent_run_context`'s doc comment).
    async fn activate_parent(
        &self,
        edge: &AwaitEdge,
        parent_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let actor = edge
            .parent_run_context
            .actor
            .clone()
            .ok_or_else(|| TurnError::InvalidRequest {
                reason: "subagent parent run context missing actor for activation".to_string(),
            })?;
        let coordinator = self
            .coordinator
            .get()
            .ok_or_else(|| TurnError::Unavailable {
                reason: "await-edge resolver coordinator is not bound".to_string(),
            })?;
        coordinator
            .activate(ActivateThreadRequest {
                scope: edge.parent_run_context.scope.clone(),
                actor,
                accepted_message_ref: background_wake_message_ref(edge)?,
                provenance: ActivationProvenance::System,
                idempotency_key: IdempotencyKey::new(format!(
                    "subagent-wake:{parent_run_id}:{}",
                    edge.child_thread_id
                ))
                .map_err(|reason| TurnError::InvalidRequest { reason })?,
                received_at: chrono::Utc::now(),
                requested_run_profile: None,
            })
            .await
            .map(|_| ())
    }
```

`background_wake_message_ref(edge)` builds the `AcceptedMessageRef` for the wake. **Verify its correct construction against the live type before writing it** — the framed result is already in the transcript under `edge.result_ref`, so the wake references that existing message rather than accepting new inbound content. If `AcceptedMessageRef` cannot be derived from an existing result ref, stop and report rather than inventing a synthetic inbound message: fabricating inbound content would put un-ingressed text on the parent's thread.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_turn_runner`
Expected: PASS, including the untouched blocking tests (`mixed_status_group_updates_each_result_resumes_once_and_consumes_every_edge` at line 1431 must stay green).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(subagent): wake background parents with a System activation on child settle"
```

---

### Task 9: Batch the multi-edge drain

The shipped per-member loop (`drain_settled_group` lines 718-735) calls `update_parent_result_reference` once per settled edge, and each call rescans the parent transcript — the in-code comment at lines 714-717 marks this as deliberately adequate for blocking's tiny groups and explicitly reserves the batched form for background (P2.4). Background sweeps and boot passes can find many settled edges at once, where the per-edge loop degrades to O(E×M) over E edges and M messages.

**Files:**
- Modify: `crates/domains/ironclaw_threads/src/filesystem_service.rs` (`update_tool_result_reference`, lines 1965-2013 — add a batch entry point beside it)
- Modify: `crates/loop/ironclaw_turn_runner/src/subagent/await_edge/resolver.rs` (`update_parent_result_reference` line 449; the drain loop lines 718-735)
- Test: `tests/integration/subagent_await_edge.rs` (the file already owns this seam — extend it, do not add a new file)

**Interfaces:**
- Consumes: the drain path from Task 8.
- Produces: a batch update that takes the full set of `(result_ref, provider_call_id, safe_summary)` triples for one parent thread and applies them in **one** snapshot read plus **one** CAS write.

- [ ] **Step 1: Write the failing test**

In `tests/integration/subagent_await_edge.rs`:

```rust
/// A multi-edge drain must not rescan the parent transcript per edge.
/// The design's bound is O(E+M), not O(E×M): one snapshot read and one CAS
/// write for the whole batch, however many children settled together.
#[tokio::test]
async fn draining_multiple_settled_edges_performs_one_snapshot_read_and_one_cas_write() {
    let harness = await_edge_harness().await;
    let counting = harness.counting_thread_service();

    harness.spawn_background_children(3).await;
    harness.settle_all_children().await;
    harness.run_drain_pass().await;

    assert_eq!(
        counting.snapshot_reads(),
        1,
        "one snapshot read for the whole batch, not one per edge"
    );
    assert_eq!(
        counting.cas_writes(),
        1,
        "one CAS write for the whole batch, not one per edge"
    );
    assert_eq!(
        harness.parent_transcript_results().len(),
        3,
        "every child's framed result must still land in the parent transcript"
    );
}
```

The counting wrapper around the thread service's snapshot-read and CAS-write calls is the write-count seam the design names. Build it as a decorator in the integration support tree (`tests/integration/support/doubles/`), following `recording_test_capability_port.rs` as the shape.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test reborn_integration_subagent_await_edge draining_multiple_settled_edges_performs_one_snapshot_read_and_one_cas_write`
(Confirm the exact target name from `tests/integration/CLAUDE.md` before running.)
Expected: FAIL — 3 snapshot reads and 3 CAS writes, one per edge.

- [ ] **Step 3: Add the batch primitive to the thread service**

In `crates/domains/ironclaw_threads/src/filesystem_service.rs`, beside `update_tool_result_reference` (line 1965), add a batch form taking a `Vec` of the per-result triples. It performs the existing `matches_tool_result_reference` rescan **once**, rewrites every matched message's content in the same pass, and commits through a single `apply_message_update` CAS-retry closure. Keep the single-result method — it stays the right primitive for one edge — and implement it in terms of the batch form with a one-element vector, so there is one code path, not two.

The batch write is idempotent for the same reason the single write already is (design §8.1): it is a CAS-guarded in-place field update on an already-existing message, not an append, so replaying it reproduces identical content.

- [ ] **Step 4: Pin drain-replay idempotency (design §8.1's required test)**

The claim above is "verified, not asserted" in the design — but the *batch* form is new code, so it needs its own regression test. The dangerous window is a crash after the transcript write but before the edge's CAS to `drained`: recovery replays the write, and an append-shaped bug would duplicate the result instead of overwriting it.

Add to `tests/integration/subagent_await_edge.rs`:

```rust
/// Design §8.1: the drain's transcript write is an in-place CAS'd field
/// update, not an append, so a crash between the write and the edge's CAS to
/// `drained` is safe to replay. An append-shaped regression here would show up
/// as duplicated child results in the parent's transcript.
#[tokio::test]
async fn replayed_drain_write_leaves_exactly_one_result_message_unchanged() {
    let harness = await_edge_harness().await;
    harness.spawn_background_child().await;

    // Crash the drain after the transcript write, before the edge CAS.
    harness.settle_child_with_crash_after_transcript_write().await;

    let after_crash = harness.parent_transcript_results();
    assert_eq!(after_crash.len(), 1, "the first write landed");
    assert_eq!(
        harness.edge_state().await,
        Some(AwaitEdgeState::Settled),
        "the edge never reached drained, so recovery must replay"
    );

    harness.run_boot_recovery_pass().await;

    let after_replay = harness.parent_transcript_results();
    assert_eq!(
        after_replay.len(),
        1,
        "replay must overwrite in place, never append a second result"
    );
    assert_eq!(
        after_replay[0], after_crash[0],
        "replayed content must be byte-identical"
    );
}
```

Run: `cargo test --test reborn_integration_subagent_await_edge replayed_drain_write_leaves_exactly_one_result_message_unchanged`
Expected: PASS once the batch form is in place. If it fails with two messages, the batch implementation appended instead of rewriting — fix the implementation, never the assertion.

- [ ] **Step 5: Use the batch form from the drain**

In `drain_settled_group`, replace the per-member loop at lines 718-735: accumulate every settled member's `(result_ref, provider_call_id, safe_summary)` into one vector first — still deriving each member's status and reason from **its own** edge, never the driving member's (the external-review fix the current comment at lines 709-713 records) — then issue a single batch call.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test reborn_integration_subagent_await_edge`
Expected: PASS.

Run: `cargo test -p ironclaw_threads -p ironclaw_turn_runner`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "perf(subagent): batch multi-edge drain into one transcript read and write"
```

---

### Task 10: Run-start sweep and the three-trigger healing test

Trigger 2 of the design's retry set. `PostCapabilityStage::drain_settled` (`crates/loop/ironclaw_agent_loop/src/executor/post_capability.rs:34-39`) returns `Vec::new()` unconditionally and names `LoopBackgroundChildPort`, a type that was never built and that the design supersedes. This task implements it and pins the invariant that no settled edge can go permanently undrained.

**Files:**
- Modify: `crates/contracts/ironclaw_loop_contracts/src/` (the port that carries the drain seam — see the boundary note below)
- Modify: `crates/loop/ironclaw_agent_loop/src/executor/post_capability.rs`
- Modify: `crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs` (the implementing decorator) and `crates/loop/ironclaw_loop_host/src/await_edge_port.rs` (the seam it delegates through)
- Modify: `crates/loop/ironclaw_turn_runner/src/subagent/await_edge/resolver.rs` (implement the drain-for-parent entry point)
- Test: `tests/integration/subagent_await_edge.rs`

**Hard boundary constraint — read before designing this:** `ironclaw_agent_loop` is **contracts-tier only**. Its `BoundaryRule` in `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs` permits `ironclaw_common`, `ironclaw_host_api`, and `ironclaw_loop_contracts` and nothing else — it may not depend on `ironclaw_turns` or `ironclaw_turn_runner`. So the stage cannot call the resolver directly; the seam must be a port defined in `ironclaw_loop_contracts`, implemented up-stack. Note also that `AgentLoopDriverHost` is a single blanket impl over a bundle of `Loop*Port`s (`crates/contracts/ironclaw_loop_contracts/src/host/progress.rs:329`), so adding a *required* method to that bundle is not a drop-in.

**Recommended shape (confirm against live types before writing code):** add the drain method to the existing `LoopCapabilityPort` with a **default implementation returning `Ok(0)`**, and implement it on `SubagentSpawnCapabilityPort` — the decorator that already owns spawn and already holds an await-edge seam (`deps.await_edge_writer`). That keeps the drain in the one component that already knows about await edges, adds no new port to the declared decorator chain, and leaves every other `LoopCapabilityPort` implementation untouched. **If this turns out to require a fourth `test-support` method on `SubagentSpawnCapabilityPort`, stop** — that struct is frozen at exactly 3 by `reborn_struct_test_support_ratchet.rs:378`.

**Interfaces:**
- Consumes: Task 8's activation path and Task 9's batch drain.
- Produces: `drain_settled` returns the count of edges drained this iteration (replacing `Vec<()>`), and the resolver gains a `drain_settled_for_parent(parent_scope, parent_run_id)` entry point that the port calls.

- [ ] **Step 1: Write the failing test**

The design names this test explicitly. In `tests/integration/subagent_await_edge.rs`:

```rust
/// Design §8.2's invariant, both halves in one test: a settle-time wake that
/// loses to ThreadBusy is always healed — by the run-start sweep if the thread
/// runs again, or by the boot pass if it does not. A settled edge can never go
/// permanently undrained.
///
/// The parent-completed precondition is asserted too (design §2): a background
/// edge stays `open` across the parent run's own terminal transition — it is
/// never abandoned — and the child's later settle still delivers.
#[tokio::test]
async fn settled_edge_threadbusy_is_healed_by_run_start_and_boot_pass() {
    // (a) run-start sweep heals it.
    {
        let harness = await_edge_harness().await;
        harness.spawn_background_child().await;
        harness.settle_child_while_parent_run_is_live().await;

        assert_eq!(
            harness.edge_state().await,
            Some(AwaitEdgeState::Settled),
            "the settle-time wake lost to ThreadBusy, so the edge stays settled"
        );
        assert_eq!(
            harness.system_activation_attempts(),
            1,
            "exactly one wake attempt per settled child — the settled state is the dedupe"
        );

        harness.advance_parent_loop_one_iteration().await;

        assert_eq!(
            harness.edge_state().await,
            None,
            "the run-start sweep must drain and close the edge"
        );
        assert_eq!(harness.parent_transcript_results().len(), 1);
    }

    // (b) the boot pass heals it when the thread never runs again.
    {
        let harness = await_edge_harness().await;
        harness.spawn_background_child().await;
        harness.settle_child_while_parent_run_is_live().await;

        harness.run_boot_recovery_pass().await;

        assert_eq!(
            harness.edge_state().await,
            None,
            "the boot pass must drain the edge at the resolver layer, with no activation"
        );
        assert_eq!(harness.parent_transcript_results().len(), 1);
    }
}

/// A background edge is not abandoned when the parent run that opened it goes
/// terminal — for background that is the normal delivery case (design §2).
#[tokio::test]
async fn background_edge_survives_its_parent_runs_terminal_transition() {
    let harness = await_edge_harness().await;
    harness.spawn_background_child().await;
    harness.complete_parent_run().await;

    assert_eq!(
        harness.edge_state().await,
        Some(AwaitEdgeState::Open),
        "a background edge stays open across the parent run's terminal transition"
    );

    harness.settle_child_as_completed().await;

    assert_eq!(harness.parent_transcript_results().len(), 1);
    assert_eq!(harness.system_activation_attempts(), 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test reborn_integration_subagent_await_edge settled_edge_threadbusy_is_healed_by_run_start_and_boot_pass background_edge_survives_its_parent_runs_terminal_transition`
Expected: FAIL — case (a) leaves the edge `Settled` after the loop iteration because `drain_settled` is a stub. Case (b) may already pass via the existing boot recovery; if so, keep it — it pins behavior this task must not break.

- [ ] **Step 3: Define the drain seam and implement `drain_settled`**

Follow the boundary constraint above. Then replace the stub in `post_capability.rs`:

```rust
    /// R2 — drain settled background-mode subagent results (design §8.2,
    /// trigger 2). `process` runs on every `TurnCompletedStep::Continue`,
    /// including a freshly-activated run's first iteration, so any settled
    /// edge whose settle-time wake lost to `ThreadBusy` is picked up the next
    /// time this thread runs for any reason.
    async fn drain_settled(&self, ctx: StageContext<'_>) -> u64 {
```

and call it from `process` where `let _drained = self.drain_settled();` sits today (line 71), replacing that discard. A drain failure must not fail the turn — log at `debug!` and continue; the boot pass is the backstop. Do **not** use `warn!`/`info!` here: this runs on a background path and would corrupt the REPL TUI.

Update the stage's doc comment (lines 19-24) — it still describes R2 as a no-op awaiting `LoopBackgroundChildPort`, which this task retires.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test reborn_integration_subagent_await_edge`
Expected: PASS.

Run: `cargo test -p ironclaw_agent_loop -p ironclaw_loop_host -p ironclaw_turn_runner`
Expected: PASS.

- [ ] **Step 5: Run the boundary gate**

Run: `cargo test -p ironclaw_architecture_tests`
Expected: PASS — in particular `reborn_dependency_boundaries` (the agent-loop contracts-only rule), `reborn_loop_port_location_scan`, and `reborn_struct_test_support_ratchet`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(subagent): drain settled background children on every loop iteration"
```

---

## Slice 2 completion gate

Run all of these before calling slices 1–2 done. Do not pipe test output through `head`/`tail` — a partial view under-counts failures.

```bash
cargo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p ironclaw_turns -p ironclaw_processes -p ironclaw_threads --no-fail-fast
cargo test -p ironclaw_agent_loop -p ironclaw_loop_host -p ironclaw_turn_runner --no-fail-fast
cargo test --test reborn_integration_subagent_await_edge --no-fail-fast
cargo test -p ironclaw_architecture_tests
bash scripts/reborn-e2e-rust.sh
scripts/pre-commit-safety.sh
```

Then confirm the two standing invariants this plan must not have broken:

```bash
# Production is still deny-filtered — this must still return the capability id.
rg -n "spawn_subagent" crates/loop/ironclaw_turn_runner/src/runtime.rs

# No trusted-ingress minting crept in.
rg -n "TrustedInboundTurnRequest|TrustedTriggerSubmitRequest" \
  crates/loop/ironclaw_turn_runner/src/subagent crates/loop/ironclaw_loop_host/src/subagent_spawn_port.rs
```

## What is deliberately NOT in this plan

Named so a reader does not mistake their absence for an oversight — each is a later slice in `pr2-pr6-shape.md`:

- **Clearing the deny-filter.** Production enablement is the last slice of the whole effort, after PR6, per the shape doc's deviation from the design's own staging.
- **The drain safety scan** (shape slice 3) and the **gate-propagation escalation walk** (shape slice 4). Both are hard prod-enable gates; neither is needed for the background mechanism to be correct, and both land before enablement.
- **`ResolveReport` counters, the `ironclaw subagent edges` operator command, and boot-recovery fairness** (shape slice 5).
- **`subagent_inspect` / `subagent_extend` / `subagent_cancel` and the WebUI child tree** (shape slices 6–9).
- **The §6 `ParentAgent` extend budget of 8.** Task 1 lands the `ParentAgent` variant and Task 2 persists it, but the 8-activation window is `subagent_extend`'s, and ships with it.
- **Un-ignoring `tests/reborn_subagent_spawn_e2e.rs`.** Its five cases — including the one that currently asserts background is *rejected* — flip in shape slice 5, alongside the counters and operator command.
