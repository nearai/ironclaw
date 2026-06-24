# Subagent thread harness — PR 0 + PR 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the prerequisites (PR 0) and the durable await-edge walking skeleton (PR 1) for the subagent thread harness: a filesystem-backed await-edge store + resolver that survives process restart, proven end-to-end for blocking-mode subagents.

**Architecture:** Per `docs/superpowers/specs/2026-06-23-subagent-thread-harness-design.md`. One `ScopedFilesystem` file per "parent run P awaits child run C" (the *await-edge*). The child's own run record (`get_run_state`) is the source of truth for terminal status. Idempotent delivery is the edge CAS itself (single-winner per transition); no ledger. A blocking parent reuses `TurnStatus::BlockedDependentRun` with a real await-edge `GateRef` — no new status variant. Everything is gated behind `subagent.v2_enabled`; flag-off preserves today's in-memory gate path unchanged.

**Tech Stack:** Rust, async/tokio, `ironclaw_filesystem::ScopedFilesystem` (CAS via `CasExpectation`), `ironclaw_turns` (`TurnScope`, `TurnRunId`, `get_run_state`), `serde`/`serde_json`. Tests use `InMemoryBackend` + `CompositeRootFilesystem` + `ScopedFilesystem::with_fixed_view`.

## Global Constraints

- **Reborn only.** All new code lives under `crates/ironclaw_reborn/`, `crates/ironclaw_turns/`, `crates/ironclaw_filesystem/`, `crates/ironclaw_reborn_composition/`, `crates/ironclaw_loop_support/`. No `src/agent/`, `src/db/`, engine v1/v2.
- **No `.unwrap()`/`.expect()` in production code** (tests are fine). Map errors with context via `thiserror` types.
- **Reuse, don't extend, the blocked-status model:** blocking parents use `TurnStatus::BlockedDependentRun` + `BlockedReason::AwaitDependentRun { gate_ref }` with a real await-edge gate ref `gate:subagent-await-<child_run_id>` (hyphen — LoopGateRef-compatible). No new `TurnStatus` variant.
- **LLM data is never deleted:** the await-edge moves through states (`open`→`settled`→`drained`/`abandoned`); rows are never hard-deleted (closed states are markers, not deletions).
- **Flag-gated:** flag-off (`subagent.v2_enabled = false`, the default) MUST preserve current behavior exactly — the new path is never consulted.
- **Quality gate before each commit:** `cargo fmt`, `cargo clippy --all --benches --tests --examples --all-features` (zero warnings), relevant `cargo test`.

---

## File Structure

- `crates/ironclaw_reborn/src/subagent/completion_observer.rs` — **Modify:** lift `wrap_untrusted_subagent_text` to `pub(crate)`; flag-gated settle+drain on child terminal (Task 1.6b).
- `crates/ironclaw_reborn_composition/src/runtime_input.rs` — **Modify:** add `subagent_v2_enabled` flag + builder.
- `crates/ironclaw_reborn/src/subagent/await_edge.rs` — **Create:** edge types (`AwaitEdge`, `AwaitEdgeState`, `EdgeTerminalKind`), `FilesystemAwaitEdgeStore` (CRUD + CAS), `await_edge_gate_ref` helper.
- `crates/ironclaw_reborn/src/subagent/await_edge_resolver.rs` — **Create:** `AwaitEdgeResolver` + object-safe `AwaitEdgeSettler` trait (settle/drain, boot resolve).
- `crates/ironclaw_reborn/src/subagent/mod.rs` — **Modify:** `mod await_edge; mod await_edge_resolver;` + `pub(crate) mod test_support` (shared `TurnScope`/fixture helpers).
- `crates/ironclaw_loop_support/src/subagent_spawn_port.rs` — **Modify:** flag-gated await-edge write in `finish_spawn` + verify the existing depth guard.
- `crates/ironclaw_reborn_composition/src/runtime.rs` — **Modify:** construct + inject the store/resolver behind the flag (Task 1.6c).

---

# PR 0 — Prerequisites

Gating, behavior-neutral. Two independent tasks. (No `TurnStatus` change — blocking reuses `BlockedDependentRun`, see Global Constraints.)

### Task 0.1: Make `wrap_untrusted_subagent_text` reusable

The resolver/observer frames child output as untrusted; reuse the existing helper instead of duplicating it.

**Files:**
- Modify: `crates/ironclaw_reborn/src/subagent/completion_observer.rs:789`
- Test: `crates/ironclaw_reborn/src/subagent/completion_observer.rs` (tests module)

**Interfaces:**
- Produces: `pub(crate) fn wrap_untrusted_subagent_text(value: String) -> String` reachable as `crate::subagent::completion_observer::wrap_untrusted_subagent_text`.

- [ ] **Step 1: Change visibility** — at `completion_observer.rs:789` change `fn wrap_untrusted_subagent_text(` to `pub(crate) fn wrap_untrusted_subagent_text(`.

- [ ] **Step 2: Add a reachability test** in the tests module:

```rust
#[test]
fn wrap_untrusted_text_wraps_in_delimiters() {
    assert_eq!(super::wrap_untrusted_subagent_text("x".to_string()), "|||x|||");
}
```

- [ ] **Step 3: Run + commit**

Run: `cargo test -p ironclaw_reborn completion_observer::tests::wrap_untrusted_text` → PASS
```bash
cargo fmt && git add -A
git commit -m "refactor(reborn): expose wrap_untrusted_subagent_text as pub(crate)"
```

---

### Task 0.2: Add `subagent_v2_enabled` flag

Mirror the existing `regex_skill_activation_enabled` pattern.

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/runtime_input.rs` (field ~396, builder ~588)
- Modify: `crates/ironclaw_reborn_config/src/config_file.rs` (add `SubagentSection { v2_enabled: Option<bool> }` + `subagent: Option<SubagentSection>` on `RebornConfigFile`, mirroring `SkillsSection` ~line 170). **Required:** `RebornConfigFile` has `#[serde(deny_unknown_fields)]`, so a TOML `[subagent]` block is rejected at parse time without this struct field. Keep the section primitive-only so it does not violate the `ironclaw_reborn_config` dependency-boundary test (`crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`).
- Modify: `crates/ironclaw_reborn_cli/src/runtime/mod.rs` (config read)
- Test: `crates/ironclaw_reborn_composition/src/runtime_input.rs` tests module

**Interfaces:**
- Produces: `RebornRuntimeInput.subagent_v2_enabled: bool` (default `false`) + `RebornRuntimeInput::with_subagent_v2_enabled(bool) -> Self`.

- [ ] **Step 1: Write the failing test** (use the same `RebornRuntimeInput` constructor the neighboring tests in this file use — read that tests module first)

```rust
#[test]
fn subagent_v2_enabled_defaults_false_and_builder_sets_it() {
    let input = /* construct via the file's existing test constructor */;
    assert!(!input.subagent_v2_enabled);
    let input = input.with_subagent_v2_enabled(true);
    assert!(input.subagent_v2_enabled);
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p ironclaw_reborn_composition runtime_input::tests::subagent_v2_enabled` → FAIL (no field).

- [ ] **Step 3: Add field + builder**

Struct field (near `regex_skill_activation_enabled`):
```rust
    /// Enables the v2 subagent thread harness (durable await-edge delivery).
    /// Default false: the existing in-memory gate path is used.
    pub subagent_v2_enabled: bool,
```
Builder (near `with_regex_skill_activation_enabled`):
```rust
    pub fn with_subagent_v2_enabled(mut self, enabled: bool) -> Self {
        self.subagent_v2_enabled = enabled;
        self
    }
```
Initialize `subagent_v2_enabled: false` in every constructor / `Default` impl the compiler flags. **Note the default is `false`** (unlike `regex_skill_activation_enabled` which defaults `true`) — do not copy `.unwrap_or(true)`.

- [ ] **Step 4: Add the config section + wire the CLI read (default false)** — first add `SubagentSection`/`subagent` to `RebornConfigFile` in `crates/ironclaw_reborn_config/src/config_file.rs` (mirror `SkillsSection`; see Files note re `deny_unknown_fields`). Then in `crates/ironclaw_reborn_cli/src/runtime/mod.rs`, mirror the `regex_skill_activation_enabled` helper: read `config_file.subagent.as_ref().and_then(|s| s.v2_enabled).unwrap_or(false)` and pass via `.with_subagent_v2_enabled(...)`.

- [ ] **Step 5: Run + commit**

Run: `cargo test -p ironclaw_reborn_composition runtime_input::tests::subagent_v2_enabled` → PASS; `cargo build --workspace` → builds
```bash
cargo fmt && git add -A
git commit -m "feat(reborn): add subagent.v2_enabled flag (default off)"
```

---

# PR 1 — Walking skeleton (blocking only)

### Task 1.1: Await-edge types + gate-ref helper

**Files:**
- Create: `crates/ironclaw_reborn/src/subagent/await_edge.rs`
- Create: `crates/ironclaw_reborn/src/subagent/test_support.rs` with the `turn_scope` fixture below — it MUST use `new_with_owner` (plain `TurnScope::new` sets `thread_owner: ActorFallback`, which makes `to_resource_scope()` fall back to the system user and breaks per-user path isolation in tests). Task 1.2 extends this file with the store fixtures.

```rust
use ironclaw_host_api::{TenantId, ThreadId, UserId};
use ironclaw_turns::TurnScope;

pub(crate) fn turn_scope(tenant: &str, user: &str, thread: &str) -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::from_trusted(tenant.to_string()),
        None, // agent_id
        None, // project_id
        ThreadId::from_trusted(thread.to_string()),
        Some(UserId::from_trusted(user.to_string())),
    )
}
```
(Confirm the `new_with_owner` parameter order against `crates/ironclaw_turns/src/scope.rs` before use.)
- Modify: `crates/ironclaw_reborn/src/subagent/mod.rs` (`mod await_edge;` + `#[cfg(test)] pub(crate) mod test_support;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `enum AwaitEdgeState { Open, Settled, Drained, Abandoned }` (serde, snake_case)
  - `enum EdgeTerminalKind { Completed, Failed, Cancelled }` (serde, snake_case)
  - `struct AwaitEdge { child_scope: TurnScope, child_thread_id: ThreadId, mode: SpawnSubagentMode, state: AwaitEdgeState, terminal_kind: Option<EdgeTerminalKind>, created_at: String, settled_at: Option<String>, closed_at: Option<String> }` (serde). **`child_scope` is required** — the child run lives under its own scope (different `thread_id` from the parent), so the resolver must use it to look up the child's status; looking the child up under the parent's scope returns `ScopeNotFound` and the parent would hang.
  - `fn edge_terminal_kind_from(status: TurnStatus) -> Option<EdgeTerminalKind>`
  - `fn await_edge_gate_ref(child_run_id: &TurnRunId) -> Result<GateRef, String>` → `gate:subagent-await-<child_run_id>` (the real await-edge gate ref the blocking parent waits on). **Note: hyphen, not colon, after `subagent-await`** — the runner converts this to a `LoopGateRef` at the spawn checkpoint (`subagent_spawn_port.rs:927`, `LoopGateRef::new(gate_ref.as_str())`), and `LoopGateRef` validation rejects `:` in the suffix (only alphanumeric/`_`/`-`/`.`). The existing v1 ref uses the same hyphen form `gate:subagent-<uuid>`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn await_edge_serde_round_trips() {
        let edge = AwaitEdge {
            child_scope: crate::subagent::test_support::turn_scope("t", "u", "ct"),
            child_thread_id: ThreadId::from_trusted("t-1".to_string()),
            mode: SpawnSubagentMode::Blocking,
            state: AwaitEdgeState::Open,
            terminal_kind: None,
            created_at: "2026-06-23T00:00:00Z".to_string(),
            settled_at: None,
            closed_at: None,
        };
        let back: AwaitEdge = serde_json::from_slice(&serde_json::to_vec(&edge).unwrap()).unwrap();
        assert_eq!(back.state, AwaitEdgeState::Open);
        assert_eq!(back.terminal_kind, None);
        assert_eq!(back.child_scope, edge.child_scope); // scope identity must survive round-trip
    }

    #[test]
    fn terminal_kind_maps_only_terminal_statuses() {
        assert_eq!(edge_terminal_kind_from(TurnStatus::Completed), Some(EdgeTerminalKind::Completed));
        assert_eq!(edge_terminal_kind_from(TurnStatus::Failed), Some(EdgeTerminalKind::Failed));
        assert_eq!(edge_terminal_kind_from(TurnStatus::Cancelled), Some(EdgeTerminalKind::Cancelled));
        assert_eq!(edge_terminal_kind_from(TurnStatus::Running), None);
        assert_eq!(edge_terminal_kind_from(TurnStatus::BlockedDependentRun), None);
    }

    #[test]
    fn gate_ref_encodes_child_run_id_and_is_loop_gate_ref_compatible() {
        let child = TurnRunId::new();
        let gr = await_edge_gate_ref(&child).unwrap();
        assert!(gr.as_str().starts_with("gate:subagent-await-")); // hyphen, not colon
        assert!(gr.as_str().contains(&child.to_string()));
        // Must also satisfy LoopGateRef validation (runner converts it at checkpoint):
        ironclaw_turns::LoopGateRef::new(gr.as_str()).expect("await gate ref must be LoopGateRef-valid");
    }
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p ironclaw_reborn subagent::await_edge::tests` → FAIL (module not found).

- [ ] **Step 3: Implement the types**

```rust
use ironclaw_host_api::ThreadId;
use ironclaw_loop_support::subagent_spawn_port::SpawnSubagentMode;
use ironclaw_turns::{GateRef, TurnRunId, TurnScope, TurnStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitEdgeState { Open, Settled, Drained, Abandoned }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeTerminalKind { Completed, Failed, Cancelled }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwaitEdge {
    pub child_scope: TurnScope,
    pub child_thread_id: ThreadId,
    pub mode: SpawnSubagentMode,
    pub state: AwaitEdgeState,
    pub terminal_kind: Option<EdgeTerminalKind>,
    pub created_at: String,
    pub settled_at: Option<String>,
    pub closed_at: Option<String>,
}

pub fn edge_terminal_kind_from(status: TurnStatus) -> Option<EdgeTerminalKind> {
    match status {
        TurnStatus::Completed => Some(EdgeTerminalKind::Completed),
        TurnStatus::Failed => Some(EdgeTerminalKind::Failed),
        TurnStatus::Cancelled => Some(EdgeTerminalKind::Cancelled),
        _ => None,
    }
}

pub fn await_edge_gate_ref(child_run_id: &TurnRunId) -> Result<GateRef, String> {
    GateRef::new(format!("gate:subagent-await-{child_run_id}"))
}
```

Add `mod await_edge;` to `subagent/mod.rs`. (`GateRef::new` returns `Result<_, String>`; `SpawnSubagentMode` at `subagent_spawn_port.rs:179` already derives `Serialize, Deserialize` + `#[serde(rename_all = "snake_case")]` and is `pub` — both confirmed, no extra work.)

- [ ] **Step 4: Run to verify it passes** — `cargo test -p ironclaw_reborn subagent::await_edge::tests` → PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy -p ironclaw_reborn --tests && git add -A
git commit -m "feat(reborn): await-edge types + gate-ref helper"
```

---

### Task 1.2: FilesystemAwaitEdgeStore — write, get, list, CAS transitions

**Files:**
- Modify: `crates/ironclaw_reborn/src/subagent/await_edge.rs`
- Modify: `crates/ironclaw_reborn/src/subagent/test_support.rs` (extend with the `await_edge_store*` fixtures; `turn_scope` already added in Task 1.1)
- Test: inline `#[cfg(test)]` (extend the module)

**Interfaces:**
- Consumes: `AwaitEdge`, `AwaitEdgeState`, `EdgeTerminalKind` (Task 1.1); `ScopedFilesystem`, `CasExpectation`, `Entry`, `ContentType`, `FilesystemError`, `RecordVersion`, `RootFilesystem` (`ironclaw_filesystem`); `ScopedPath` (`ironclaw_host_api`); `TurnScope`, `TurnRunId` (`ironclaw_turns`).
- Produces:
  - `struct FilesystemAwaitEdgeStore<F: RootFilesystem> { pub fs: Arc<ScopedFilesystem<F>> }`
  - `async fn write_open(&self, scope, parent: &TurnRunId, child: &TurnRunId, edge: &AwaitEdge) -> Result<(), AwaitEdgeError>` (CAS `Absent`; `Ok(())` if already exists — idempotent spawn)
  - `async fn get(&self, scope, parent, child) -> Result<Option<(AwaitEdge, RecordVersion)>, AwaitEdgeError>`
  - `async fn transition(&self, scope, parent, child, expect: AwaitEdgeState, to: AwaitEdgeState, terminal_kind: Option<EdgeTerminalKind>, now: &str) -> Result<bool, AwaitEdgeError>` (version-CAS; `Ok(true)` if performed, `Ok(false)` if already at/past `to` or version race — idempotent)
  - `async fn list_unclosed_for_parent(&self, scope, parent) -> Result<Vec<(TurnRunId, AwaitEdgeState)>, AwaitEdgeError>` — returns children whose edge is `Open` OR `Settled` (the unresolved set the resolver must act on; a `Settled`-but-not-`Drained` edge means the child finished but the parent was not yet resumed — crash between settle and resume). Excludes `Drained`/`Abandoned`.
  - `async fn list_parents_with_unclosed_edges(&self, scope) -> Result<Vec<TurnRunId>, AwaitEdgeError>` — `list_dir` on the await-edge root; returns each `parent_run_id` directory name. Drives boot recovery directly from the edge store (no `TurnStateStore` active-run query — none exists).
  - `enum AwaitEdgeError { Backend{reason}, Path{reason}, Serde{reason} }` (thiserror)

- [ ] **Step 1: Extend the shared test fixture** `test_support.rs` (created in Task 1.1 with `turn_scope`; add the store fixtures here)

Copy the fixture pattern verbatim from `crates/ironclaw_turns/tests/filesystem_turn_state_contract.rs` (the canonical in-memory `ScopedFilesystem` fixture: `InMemoryBackend::new()` + `CompositeRootFilesystem` + `MountDescriptor { virtual_root /engine, backend_kind: BackendKind::MemoryDocuments, storage_class: StorageClass::StructuredRecords, content_kind: ContentKind::StructuredRecord, index_policy: IndexPolicy::NotIndexed, capabilities: backend.capabilities() }` + `MountView` with a `/turns` grant at `/engine/tenants/<t>/users/<u>/turns` + `ScopedFilesystem::with_fixed_view`). Expose:

```rust
pub(crate) fn turn_scope(tenant: &str, user: &str, thread: &str) -> TurnScope; // ExplicitUser owner
pub(crate) fn await_edge_store_on(backend: Arc<InMemoryBackend>, tenant: &str, user: &str)
    -> FilesystemAwaitEdgeStore<CompositeRootFilesystem>;
pub(crate) fn await_edge_store() -> (Arc<InMemoryBackend>, FilesystemAwaitEdgeStore<CompositeRootFilesystem>); // default tenant/user, returns backend for restart tests
```

Read `crates/ironclaw_turns/src/scope.rs` for `TurnThreadOwner::ExplicitUser { owner_user_id }` to build `turn_scope`.

- [ ] **Step 2: Write the failing tests** (in `await_edge.rs` tests, using `crate::subagent::test_support::*`)

```rust
    fn open_edge() -> AwaitEdge { /* AwaitEdge { state: Open, mode: Blocking, child_thread_id: ThreadId::from_trusted("ct".into()), created_at:"t0".into(), terminal_kind:None, settled_at:None, closed_at:None } */ }

    #[tokio::test]
    async fn write_then_get_round_trips() {
        let (_b, s) = await_edge_store();
        let (scope, p, c) = (turn_scope("t","u","th"), TurnRunId::new(), TurnRunId::new());
        s.write_open(&scope, &p, &c, &open_edge()).await.unwrap();
        let (got, _v) = s.get(&scope, &p, &c).await.unwrap().expect("present");
        assert_eq!(got.state, AwaitEdgeState::Open);
    }

    #[tokio::test]
    async fn settle_then_double_settle_is_noop() {
        let (_b, s) = await_edge_store();
        let (scope, p, c) = (turn_scope("t","u","th"), TurnRunId::new(), TurnRunId::new());
        s.write_open(&scope, &p, &c, &open_edge()).await.unwrap();
        assert!(s.transition(&scope,&p,&c, AwaitEdgeState::Open, AwaitEdgeState::Settled, Some(EdgeTerminalKind::Completed), "t1").await.unwrap());
        assert!(!s.transition(&scope,&p,&c, AwaitEdgeState::Open, AwaitEdgeState::Settled, Some(EdgeTerminalKind::Completed), "t1").await.unwrap());
        let (got,_) = s.get(&scope,&p,&c).await.unwrap().unwrap();
        assert_eq!(got.state, AwaitEdgeState::Settled);
        assert_eq!(got.terminal_kind, Some(EdgeTerminalKind::Completed));
    }

    #[tokio::test]
    async fn list_unclosed_includes_open_and_settled_excludes_closed() {
        let (_b, s) = await_edge_store();
        let (scope, p) = (turn_scope("t","u","th"), TurnRunId::new());
        let (c1, c2, c3) = (TurnRunId::new(), TurnRunId::new(), TurnRunId::new());
        s.write_open(&scope,&p,&c1,&open_edge()).await.unwrap(); // Open
        s.write_open(&scope,&p,&c2,&open_edge()).await.unwrap();
        s.transition(&scope,&p,&c2, AwaitEdgeState::Open, AwaitEdgeState::Settled, Some(EdgeTerminalKind::Completed), "t1").await.unwrap(); // Settled
        s.write_open(&scope,&p,&c3,&open_edge()).await.unwrap();
        s.transition(&scope,&p,&c3, AwaitEdgeState::Open, AwaitEdgeState::Settled, Some(EdgeTerminalKind::Completed), "t1").await.unwrap();
        s.transition(&scope,&p,&c3, AwaitEdgeState::Settled, AwaitEdgeState::Drained, None, "t2").await.unwrap(); // Drained
        let mut got = s.list_unclosed_for_parent(&scope,&p).await.unwrap();
        got.sort_by_key(|(id, _)| id.to_string());
        // c1 Open + c2 Settled present; c3 Drained absent.
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|(id, st)| *id == c1 && *st == AwaitEdgeState::Open));
        assert!(got.iter().any(|(id, st)| *id == c2 && *st == AwaitEdgeState::Settled));
        assert!(got.iter().all(|(id, _)| *id != c3));
    }

    #[tokio::test]
    async fn list_parents_with_unclosed_edges_finds_the_parent() {
        let (_b, s) = await_edge_store();
        let (scope, p, c) = (turn_scope("t","u","th"), TurnRunId::new(), TurnRunId::new());
        s.write_open(&scope,&p,&c,&open_edge()).await.unwrap();
        assert_eq!(s.list_parents_with_unclosed_edges(&scope).await.unwrap(), vec![p]);
    }
```

- [ ] **Step 3: Run to verify it fails** — `cargo test -p ironclaw_reborn subagent::await_edge::tests::write_then_get` → FAIL.

- [ ] **Step 4: Implement the store** (append to `await_edge.rs`)

```rust
use std::sync::Arc;
use ironclaw_filesystem::{CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ScopedPath;
use ironclaw_turns::{TurnRunId, TurnScope};

#[derive(Debug, thiserror::Error)]
pub enum AwaitEdgeError {
    #[error("await-edge backend error: {reason}")] Backend { reason: String },
    #[error("await-edge path error: {reason}")] Path { reason: String },
    #[error("await-edge serde error: {reason}")] Serde { reason: String },
}

const AWAIT_EDGE_ROOT: &str = "/turns/subagent-await-edges";

fn edge_path(parent: &TurnRunId, child: &TurnRunId) -> Result<ScopedPath, AwaitEdgeError> {
    ScopedPath::new(format!("{AWAIT_EDGE_ROOT}/{parent}/{child}.json")).map_err(|e| AwaitEdgeError::Path { reason: e.to_string() })
}
fn parent_dir(parent: &TurnRunId) -> Result<ScopedPath, AwaitEdgeError> {
    ScopedPath::new(format!("{AWAIT_EDGE_ROOT}/{parent}")).map_err(|e| AwaitEdgeError::Path { reason: e.to_string() })
}

pub struct FilesystemAwaitEdgeStore<F: RootFilesystem> { pub fs: Arc<ScopedFilesystem<F>> }

impl<F: RootFilesystem> FilesystemAwaitEdgeStore<F> {
    fn entry_for(edge: &AwaitEdge) -> Result<Entry, AwaitEdgeError> {
        let body = serde_json::to_vec(edge).map_err(|e| AwaitEdgeError::Serde { reason: e.to_string() })?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }

    pub async fn write_open(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId, edge: &AwaitEdge) -> Result<(), AwaitEdgeError> {
        let path = edge_path(parent, child)?;
        match self.fs.put(&scope.to_resource_scope(), &path, Self::entry_for(edge)?, CasExpectation::Absent).await {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(()), // already exists; idempotent
            Err(e) => Err(AwaitEdgeError::Backend { reason: e.to_string() }),
        }
    }

    pub async fn get(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId) -> Result<Option<(AwaitEdge, RecordVersion)>, AwaitEdgeError> {
        let path = edge_path(parent, child)?;
        match self.fs.get(&scope.to_resource_scope(), &path).await {
            Ok(Some(v)) => {
                let edge = serde_json::from_slice(&v.entry.body).map_err(|e| AwaitEdgeError::Serde { reason: e.to_string() })?;
                Ok(Some((edge, v.version)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(AwaitEdgeError::Backend { reason: e.to_string() }),
        }
    }

    pub async fn transition(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId,
        expect: AwaitEdgeState, to: AwaitEdgeState, terminal_kind: Option<EdgeTerminalKind>, now: &str) -> Result<bool, AwaitEdgeError> {
        let Some((mut edge, version)) = self.get(scope, parent, child).await? else { return Ok(false) };
        if edge.state == to { return Ok(false) }        // already transitioned (idempotent)
        if edge.state != expect { return Ok(false) }    // not in expected predecessor
        edge.state = to;
        match to {
            AwaitEdgeState::Settled => { edge.terminal_kind = terminal_kind; edge.settled_at = Some(now.to_string()); }
            AwaitEdgeState::Drained | AwaitEdgeState::Abandoned => { edge.closed_at = Some(now.to_string()); }
            AwaitEdgeState::Open => {}
        }
        let path = edge_path(parent, child)?;
        match self.fs.put(&scope.to_resource_scope(), &path, Self::entry_for(&edge)?, CasExpectation::Version(version)).await {
            Ok(_) => Ok(true),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(false), // lost race; re-read would show advanced state
            Err(e) => Err(AwaitEdgeError::Backend { reason: e.to_string() }),
        }
    }

    /// O(K) — lists then reads each. K is bounded (blocking: ≤1/parent at depth cap 1).
    /// Returns children whose edge is Open or Settled (the unresolved set). PR 2
    /// may add a scope-wide open-count for background fan-out capacity.
    pub async fn list_unclosed_for_parent(&self, scope: &TurnScope, parent: &TurnRunId) -> Result<Vec<(TurnRunId, AwaitEdgeState)>, AwaitEdgeError> {
        let dir = parent_dir(parent)?;
        let entries = match self.fs.list_dir(&scope.to_resource_scope(), &dir).await {
            Ok(e) => e,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(e) => return Err(AwaitEdgeError::Backend { reason: e.to_string() }),
        };
        let mut out = Vec::new();
        for entry in entries {
            let Some(stem) = entry.name.strip_suffix(".json") else { continue };
            let Ok(child) = TurnRunId::parse(stem) else { continue };
            if let Some((edge, _)) = self.get(scope, parent, &child).await? {
                if matches!(edge.state, AwaitEdgeState::Open | AwaitEdgeState::Settled) {
                    out.push((child, edge.state));
                }
            }
        }
        Ok(out)
    }

    /// Boot recovery driver: every parent_run_id directory under the await-edge
    /// root in this scope. (list_dir returns the directory entries; filter to
    /// Directory file_type. If the backend does not report FileType::Directory
    /// for path-segment nodes — check crates/ironclaw_filesystem/src/in_memory.rs's
    /// list_dir — fall back to filtering entries whose name does NOT end in
    /// ".json"; the `list_parents_with_unclosed_edges_finds_the_parent` test
    /// catches a wrong choice.)
    pub async fn list_parents_with_unclosed_edges(&self, scope: &TurnScope) -> Result<Vec<TurnRunId>, AwaitEdgeError> {
        let root = ScopedPath::new(AWAIT_EDGE_ROOT).map_err(|e| AwaitEdgeError::Path { reason: e.to_string() })?;
        let entries = match self.fs.list_dir(&scope.to_resource_scope(), &root).await {
            Ok(e) => e,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(e) => return Err(AwaitEdgeError::Backend { reason: e.to_string() }),
        };
        Ok(entries
            .into_iter()
            .filter(|e| e.file_type == ironclaw_filesystem::FileType::Directory)
            .filter_map(|e| TurnRunId::parse(&e.name).ok())
            .collect())
    }
}
```

- [ ] **Step 5: Run + commit**

Run: `cargo test -p ironclaw_reborn subagent::await_edge::tests` → PASS; `cargo clippy -p ironclaw_reborn --tests` → clean
```bash
cargo fmt && git add -A
git commit -m "feat(reborn): FilesystemAwaitEdgeStore with CAS transitions"
```

---

### Task 1.3: Crash-recovery + idempotency + scope-isolation proof (the central bet)

**Files:**
- Create: `crates/ironclaw_reborn/tests/await_edge_durability.rs`

**Interfaces:**
- Consumes: `FilesystemAwaitEdgeStore`, `AwaitEdge`, `AwaitEdgeState`, and the `test_support` fixtures (Task 1.2). To reach `pub(crate) test_support` from an integration test, the fixtures must be `pub` OR duplicate the fixture in the test file. Choose: make `test_support` items `pub` under a `#[doc(hidden)] pub mod test_support` gated by `#[cfg(any(test, feature = "test-support"))]`, or copy the fixture into this file. Pick copy for PR 1 to avoid widening crate API; note the duplication.

- [ ] **Step 1: Write the restart + idempotency test** (shares one `Arc<InMemoryBackend>` across two store instances)

```rust
#[tokio::test]
async fn edge_survives_store_rebuild_and_resettle_is_idempotent() {
    let backend = Arc::new(InMemoryBackend::new());
    let store_a = await_edge_store_on(backend.clone(), "t", "u");
    let (scope, p, c) = (turn_scope("t","u","th"), TurnRunId::new(), TurnRunId::new());
    store_a.write_open(&scope,&p,&c,&open_edge()).await.unwrap();
    drop(store_a); // simulate restart
    let store_b = await_edge_store_on(backend.clone(), "t", "u");
    let (edge,_) = store_b.get(&scope,&p,&c).await.unwrap().expect("survived restart");
    assert_eq!(edge.state, AwaitEdgeState::Open);
    assert!(store_b.transition(&scope,&p,&c,AwaitEdgeState::Open,AwaitEdgeState::Settled,Some(EdgeTerminalKind::Completed),"t1").await.unwrap());
    assert!(!store_b.transition(&scope,&p,&c,AwaitEdgeState::Open,AwaitEdgeState::Settled,Some(EdgeTerminalKind::Completed),"t1").await.unwrap());
}
```

- [ ] **Step 2: Write the scope-isolation test** (design §11 — promoted from old §7.3). Two stores over the SAME backend with different `(tenant,user)` fixed-view prefixes; an edge under `t1` must be invisible under `t2`.

```rust
#[tokio::test]
async fn scoped_query_excludes_other_scope_edges() {
    let backend = Arc::new(InMemoryBackend::new());
    let store_t1 = await_edge_store_on(backend.clone(), "t1", "u1");
    let store_t2 = await_edge_store_on(backend.clone(), "t2", "u2");
    let p = TurnRunId::new();
    let c = TurnRunId::new();
    store_t1.write_open(&turn_scope("t1","u1","th"), &p, &c, &open_edge()).await.unwrap();
    // Same parent run id, different scope/prefix → t2 sees nothing.
    assert!(store_t2.list_unclosed_for_parent(&turn_scope("t2","u2","th"), &p).await.unwrap().is_empty());
    assert!(store_t2.get(&turn_scope("t2","u2","th"), &p, &c).await.unwrap().is_none());
}
```

(`await_edge_store_on` with distinct tenant/user builds a `MountView` rooted at `/engine/tenants/<t>/users/<u>/turns` — so different scopes are different physical subtrees over the shared backend; this proves path-level isolation, which is what the await-edge store relies on.)

- [ ] **Step 3: Run + commit**

Run: `cargo test -p ironclaw_reborn --test await_edge_durability` → PASS
```bash
cargo fmt && git add -A
git commit -m "test(reborn): await-edge restart recovery + idempotency + scope isolation"
```

---

### Task 1.4: AwaitEdgeResolver + object-safe AwaitEdgeSettler trait

**Files:**
- Create: `crates/ironclaw_reborn/src/subagent/await_edge_resolver.rs`
- Modify: `crates/ironclaw_reborn/src/subagent/mod.rs` (`mod await_edge_resolver;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes:
  - `AwaitEdgeResolver` (this task) consumes: `FilesystemAwaitEdgeStore`, `AwaitEdgeState`, `edge_terminal_kind_from` (Tasks 1.1/1.2); `TurnStateStore::get_run_state` + `GetRunStateRequest` + `TurnStatus::is_terminal()` (`ironclaw_turns`). It does **not** use the coordinator.
  - `resume_parent_for_await_edge` (a free fn in this file, written in Task 1.6b) additionally consumes: `await_edge_gate_ref`, `TurnCoordinator::resume_turn` + `ResumeTurnRequest` + `ResumeTurnPrecondition::BlockedDependentRunGate` + `ironclaw_turns::IdempotencyKey`.
- Produces:
  - `#[async_trait] pub trait AwaitEdgeSettler: Send + Sync { async fn settle_if_terminal(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId, now: &str) -> Result<bool, AwaitEdgeError>; }` — object-safe; the observer holds `Arc<dyn AwaitEdgeSettler>` (no generic leak). This **only settles the edge `Open→Settled`** if the child is terminal — it does NOT drain and does NOT resume. **Drain happens only after the parent is resumed** (the design's `drained` = "parent consumed"; draining at settle time would lose the parent on a crash-before-resume — the F4 hole). The caller's sequence is always: settle → resume parent → drain.
  - `struct AwaitEdgeResolver<F: RootFilesystem> { store: Arc<FilesystemAwaitEdgeStore<F>>, runs: Arc<dyn TurnStateStore> }` — **pure** (no coordinator); fully testable without a live coordinator.
  - `impl<F: RootFilesystem + 'static> AwaitEdgeSettler for AwaitEdgeResolver<F>` — settles `open→settled` if the child is terminal; returns whether it settled. No drain, no resume.
  - `impl<F> AwaitEdgeResolver<F> { async fn resolve_parent(&self, scope, parent, parent_status: TurnStatus, now) -> Result<ResolveReport, AwaitEdgeError> }` — **boot path:** over `list_unclosed_for_parent`: an `Open` edge whose child is terminal → settle it; **every edge now in `Settled` (just-settled OR settled-before-the-crash) → collect into `settled_children`** for the caller to resume+drain (this closes F4 — a `Settled`-but-undrained edge from a lost resume is recovered); an `Open` edge whose child is not terminal and whose parent is terminal → abandon. Resume + drain are the caller's job (1.6c), so the resolver stays coordinator-free.
  - `struct ResolveReport { settled_children: Vec<TurnRunId>, abandoned: u32, left_open: u32 }` — `settled_children` is what the boot routine resumes then drains.

- [ ] **Step 1: Write the failing tests** (with a minimal fake `TurnStateStore`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // FakeRunStore { statuses: HashMap<TurnRunId, TurnStatus> } implementing
    // TurnStateStore: get_run_state builds a TurnRunState with the mapped status
    // (read status.rs:301 for required fields, or return Err(TurnError::ScopeNotFound)
    // for unknown runs). All other trait methods: unimplemented!() (never called here).

    #[tokio::test]
    async fn settles_when_child_terminal() { /* child=Completed -> settle_if_terminal returns true; edge -> Settled (NOT drained) */ }
    #[tokio::test]
    async fn leaves_open_when_child_running() { /* child=Running -> returns false; edge stays Open */ }
    #[tokio::test]
    async fn resolve_parent_collects_settled_children() { /* child Completed -> resolve_parent: settled_children == [child]; edge stays Settled (caller drains) */ }
    #[tokio::test]
    async fn resolve_parent_recovers_presettled_edge() { /* edge pre-Settled (crash before resume) -> resolve_parent still collects it in settled_children (F4) */ }
    #[tokio::test]
    async fn resolve_parent_abandons_open_when_parent_terminal() { /* child Running, parent Failed -> report.abandoned==1 */ }
}
```

Fill bodies using the Task 1.2 store fixture + `FakeRunStore`.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p ironclaw_reborn subagent::await_edge_resolver::tests` → FAIL.

- [ ] **Step 3: Implement**

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_turns::{GetRunStateRequest, TurnRunId, TurnScope, TurnStateStore, TurnStatus};
use super::await_edge::{edge_terminal_kind_from, AwaitEdgeError, AwaitEdgeState, FilesystemAwaitEdgeStore};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolveReport { pub settled_children: Vec<TurnRunId>, pub abandoned: u32, pub left_open: u32 }

#[async_trait]
pub trait AwaitEdgeSettler: Send + Sync {
    async fn settle_if_terminal(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId, now: &str) -> Result<bool, AwaitEdgeError>;
}

pub struct AwaitEdgeResolver<F: RootFilesystem> {
    pub store: Arc<FilesystemAwaitEdgeStore<F>>,
    pub runs: Arc<dyn TurnStateStore>,
}

impl<F: RootFilesystem> AwaitEdgeResolver<F> {
    /// Look up the child run under its OWN scope (read from the edge record).
    /// The child lives under a different thread_id than the parent; querying
    /// with the parent scope returns ScopeNotFound and the parent would hang.
    async fn child_status(&self, child_scope: &TurnScope, child: &TurnRunId) -> Option<TurnStatus> {
        match self.runs.get_run_state(GetRunStateRequest { scope: child_scope.clone(), run_id: child.clone() }).await {
            Ok(state) => Some(state.status),
            Err(_) => None,
        }
    }

    /// Boot recovery. For each unclosed edge: settle terminal Open children;
    /// collect every now-Settled child (just-settled OR settled-before-crash —
    /// F4 recovery) for the caller to resume+drain; abandon Open edges whose
    /// child is not terminal when the parent itself is terminal. Coordinator-free.
    pub async fn resolve_parent(&self, scope: &TurnScope, parent: &TurnRunId, parent_status: TurnStatus, now: &str) -> Result<ResolveReport, AwaitEdgeError> {
        let mut report = ResolveReport::default();
        for (child, state) in self.store.list_unclosed_for_parent(scope, parent).await? {
            let settled_now = state == AwaitEdgeState::Open
                && self.settle_if_terminal(scope, parent, &child, now).await?;
            if state == AwaitEdgeState::Settled || settled_now {
                report.settled_children.push(child); // caller resumes then drains
            } else if parent_status.is_terminal() {
                if self.store.transition(scope, parent, &child, AwaitEdgeState::Open, AwaitEdgeState::Abandoned, None, now).await? {
                    report.abandoned += 1;
                }
            } else {
                report.left_open += 1;
            }
        }
        Ok(report)
    }
}

#[async_trait]
impl<F: RootFilesystem + 'static> AwaitEdgeSettler for AwaitEdgeResolver<F> {
    async fn settle_if_terminal(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId, now: &str) -> Result<bool, AwaitEdgeError> {
        // `scope` is the PARENT scope (the edge path's scope). Read the edge to
        // get the CHILD scope for the status lookup.
        let Some((edge, _)) = self.store.get(scope, parent, child).await? else { return Ok(false) };
        let Some(status) = self.child_status(&edge.child_scope, child).await else { return Ok(false) };
        let Some(kind) = edge_terminal_kind_from(status) else { return Ok(false) };
        self.store.transition(scope, parent, child, AwaitEdgeState::Open, AwaitEdgeState::Settled, Some(kind), now).await
    }
}
```

(`TurnStateStore` is already used as `Arc<dyn TurnStateStore>` elsewhere in the codebase, so it is object-safe and compiles as a trait object. `TurnScope`/`TurnRunId` are `Clone`.)

- [ ] **Step 4: Run + clippy + commit**

Run: `cargo test -p ironclaw_reborn subagent::await_edge_resolver::tests` → PASS; `cargo clippy -p ironclaw_reborn --tests` → clean
```bash
cargo fmt && git add -A
git commit -m "feat(reborn): AwaitEdgeResolver + AwaitEdgeSettler trait"
```

---

### Task 1.5: Verify the existing spawn depth floor

The guard already exists (`subagent_spawn_port.rs:653`, `DEFAULT_SUBAGENT_MAX_DEPTH = 1`); lock it with a regression test.

**Files:**
- Test: `crates/ironclaw_loop_support/src/subagent_spawn_port/tests.rs`

- [ ] **Step 1: Check existing coverage** — `rg -n "max_depth|subagent_depth" crates/ironclaw_loop_support/src/subagent_spawn_port/tests.rs`. If a spawn-refused-at-max-depth test exists, mark done; else continue.
- [ ] **Step 2: Write the regression test** — mirror an existing spawn test's harness; set the parent record's `subagent_depth = limits.max_depth`; assert the spawn is refused (the host error the code returns at line 653).
- [ ] **Step 3: Run + commit** — `cargo test -p ironclaw_loop_support subagent_spawn_port` → PASS
```bash
cargo fmt && git add -A && git commit -m "test(loop_support): lock spawn depth floor at max_depth"
```

---

### Task 1.6a: Flag-gated await-edge write at spawn

**Files:**
- Modify: `crates/ironclaw_loop_support/src/subagent_spawn_port.rs` (`finish_spawn`, ~847)
- Test: extend `crates/ironclaw_loop_support/src/subagent_spawn_port/tests.rs`

**Interfaces:**
- Consumes: `FilesystemAwaitEdgeStore::write_open` + `AwaitEdge` (Tasks 1.1/1.2), `await_edge_gate_ref` (Task 1.1), `RebornRuntimeInput.subagent_v2_enabled` threaded into `SubagentSpawnDeps` (add a `subagent_v2_enabled: bool` field + optional `await_edge_store: Option<Arc<dyn AwaitEdgeWriter>>` where `AwaitEdgeWriter` is a tiny object-safe trait wrapping `write_open` — keeps `SubagentSpawnDeps` non-generic).
- Produces: when `subagent_v2_enabled`, `finish_spawn` writes an `open` await-edge (in addition to the existing `record_awaited_child`) and parks the parent in `BlockedDependentRun` with `BlockedReason::AwaitDependentRun { gate_ref: await_edge_gate_ref(child_run_id) }`.

- [ ] **Step 1: Read** `finish_spawn` (~847) and how the parent is currently blocked (`completion_observer.rs:3050` sets `BlockedDependentRun`). Note the exact block-construction call.
- [ ] **Step 2: Define `AwaitEdgeWriter`** (object-safe) in `await_edge.rs`: `#[async_trait] pub trait AwaitEdgeWriter: Send + Sync { async fn write_open(&self, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId, edge: &AwaitEdge) -> Result<(), AwaitEdgeError>; }` and impl it for `FilesystemAwaitEdgeStore<F>`. (This thin erasure trait exists solely to keep `SubagentSpawnDeps` non-generic — add a one-line comment saying so.) Add `subagent_v2_enabled: bool` and `await_edge_store: Option<Arc<dyn AwaitEdgeWriter>>` to `SubagentSpawnDeps`.

  **Architecture-rule compliance (B2):** `Option<Arc<dyn AwaitEdgeWriter>>` is a flag-gated optional dependency, which `.claude/rules/architecture.md` rule 2 flags. Annotate the field: `// arch-exempt: optional_arc — flag-gated v2 path, set iff subagent.v2_enabled; removed when v2 becomes default (plan 2026-06-23 cleanup PR)`. **Ripple:** `SubagentSpawnDeps` is a struct-literal with ~15 test construction sites + 1 production site (`ironclaw_reborn/src/runtime.rs:556`). Every site must add `subagent_v2_enabled: false, await_edge_store: None,`. Enumerate them first: `rg -ln "SubagentSpawnDeps" crates/` then grep each file for the struct-literal construction (rustfmt may break `SubagentSpawnDeps {` across lines, so don't rely on a brace-adjacent pattern); update each in this task (mechanical two-field additions: `subagent_v2_enabled: false, await_edge_store: None,`). The production site (`ironclaw_reborn/src/runtime.rs:556`) is wired for real in Task 1.6c.
- [ ] **Step 3: Write the failing test** — spawn with `subagent_v2_enabled=true` + a captured in-memory `AwaitEdgeWriter`; assert an `open` edge is written for the child. With `false`, assert none.
- [ ] **Step 4: Implement** — in `finish_spawn`, after `record_awaited_child`, `if deps.subagent_v2_enabled { if let Some(store) = &deps.await_edge_store { store.write_open(&self.run_context.scope, &parent_run_id, &child_run_id, &edge).await? } }` where:
  - the **scope arg is `self.run_context.scope`** (the PARENT scope) — boot recovery enumerates the await-edge root under this scope;
  - `edge.child_scope` MUST be the **exact same `TurnScope` value** `finish_spawn` passes to `submit_child_run` for this child (the `child_turn_scope` it builds, `ActorFallback` owner and all — do NOT rebuild it with a different owner). The correctness invariant is *capture-verbatim*: the child run is persisted under that scope, so the resolver's `get_run_state(edge.child_scope, child)` matches how it was stored, regardless of `thread_owner` / `to_resource_scope()` semantics. (This sidesteps F1: we never need to reason about which fields `get_run_state` keys on — same scope in, same scope to look up.) `edge.child_thread_id` = the child thread id; `edge.mode` = the spawn mode; `state: Open`; timestamps from the spawn clock.
  - Use `await_edge_gate_ref(&child_run_id)?` (hyphen form) as the block `gate_ref` in the parent's `BlockedReason::AwaitDependentRun`. Flag-off branch byte-for-byte unchanged.
- [ ] **Step 5: Run + commit** — `cargo test -p ironclaw_loop_support subagent_spawn_port` → PASS
```bash
cargo fmt && git add -A && git commit -m "feat(loop_support): flag-gated await-edge write at spawn"
```

---

### Task 1.6b: Settle + drain on child terminal (completion observer)

**Files:**
- Modify: `crates/ironclaw_reborn/src/subagent/completion_observer.rs`
- Test: extend the observer's tests module

**Interfaces:**
- Consumes: `Arc<dyn AwaitEdgeSettler>` (Task 1.4) + `subagent_v2_enabled` injected into the observer.
- Produces: when `subagent_v2_enabled`, on a child terminal event the observer runs the sequence **settle → resume → drain**: (1) `settler.settle_if_terminal(...)` (Open→Settled); (2) resume the parent via the shared `resume_parent_for_await_edge(...)` helper (Task 1.6c) — `gate_resolution_ref: await_edge_gate_ref(child)?`, `precondition: BlockedDependentRunGate`; (3) drain `store.transition(Settled→Drained)` only after the resume returns Ok. Drain-after-resume is the F4 fix: a crash between settle and resume leaves the edge `Settled`, and boot recovery (1.6c) re-resumes it. **B1 fix:** block (1.6a) and resume both use `await_edge_gate_ref(child)`; never `record.gate_ref` (v1 format → `BlockedDependentRunGate` precondition rejects it → parent hangs).

- [ ] **Step 1: Read** the observer's terminal handler + the existing v1 resume assembly at `completion_observer.rs:467` (`coordinator.resume_turn(ResumeTurnRequest { scope, actor, run_id, gate_resolution_ref, source_binding_ref, reply_target_binding_ref, idempotency_key, precondition: BlockedDependentRunGate })`). Note exactly how each field is sourced — these become the shared helper's inputs (Task 1.6c).
- [ ] **Step 2: Write the failing test** — observer with `subagent_v2_enabled=true` + a fake `AwaitEdgeSettler` + a fake coordinator capturing `ResumeTurnRequest`s + an in-memory await-edge store; pre-write an Open edge; fire child-terminal(Completed); assert (a) settler invoked, (b) resume fired with `gate_resolution_ref == await_edge_gate_ref(child)` + `precondition == BlockedDependentRunGate`, (c) edge ends `Drained`. With `false`: v1 path unchanged, settler not called, no edge touched.
- [ ] **Step 3: Implement** — add `subagent_v2_enabled: bool`, `await_edge_settler: Option<Arc<dyn AwaitEdgeSettler>>`, and an await-edge store handle (or fold drain into the helper) to the observer; in the terminal handler, when enabled, run settle → `resume_parent_for_await_edge` → drain. **Ordering note:** `resume_parent_for_await_edge` is the shared helper canonically defined in Task 1.6c; since this task lands first, write it inline in `await_edge_resolver.rs` now (a real impl, not a stub) and Task 1.6c will reuse it as-is (no extraction needed — 1.6c just wires the boot caller to the same fn). Do not ship 1.6b without the resume call — without it the parent hangs after the child completes. Flag-off branch byte-for-byte unchanged.
- [ ] **Step 4: Run + commit** — `cargo test -p ironclaw_reborn completion_observer` → PASS
```bash
cargo fmt && git add -A && git commit -m "feat(reborn): settle await-edge on child terminal (flag-gated)"
```

---

### Task 1.6c: Composition wiring + end-to-end integration test

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Test: `crates/ironclaw_reborn_composition/tests/subagent_await_edge_e2e.rs` (mirror `subagent_runtime_wiring.rs`)

**Interfaces:**
- Consumes: everything above + `subagent_v2_enabled` from `RebornRuntimeInput`; `AwaitEdgeResolver::resolve_parent` → `ResolveReport.settled_children` (Task 1.4); the shared resume helper.
- Produces:
  - when `subagent_v2_enabled`, `runtime.rs` constructs `FilesystemAwaitEdgeStore` over the runtime `ScopedFilesystem`, wraps it as `Arc<dyn AwaitEdgeWriter>` (into `SubagentSpawnDeps`, with `subagent_v2_enabled: true`) and an `AwaitEdgeResolver` as `Arc<dyn AwaitEdgeSettler>` (into the observer, with `subagent_v2_enabled: true`). Behind the flag only.
  - a **shared resume helper** `async fn resume_parent_for_await_edge(coordinator: &Arc<dyn TurnCoordinator>, runs: &Arc<dyn TurnStateStore>, scope: &TurnScope, parent: &TurnRunId, child: &TurnRunId) -> Result<(), AwaitEdgeError>` in `await_edge_resolver.rs` — the single grounded v2 resume, used by BOTH the observer (live, 1.6b) and the boot routine. It builds `ironclaw_turns::ResumeTurnRequest` (F2): `scope`/`run_id` = parent; `gate_resolution_ref = await_edge_gate_ref(child)?`; `precondition: ResumeTurnPrecondition::BlockedDependentRunGate`; **`source_binding_ref`/`reply_target_binding_ref` derived deterministically from `(parent, child)`** via the spawn-port helpers (Step 0 below makes them callable); **`actor`** = the parent's `get_run_state(...).actor` (`Option<TurnActor>`); **if `None`, do NOT invent a sentinel** (none exists; `TurnActor` has no system variant, and approval recovery *rejects* `None`) — return `AwaitEdgeError::Backend { reason: "parent run has no actor; cannot resume await-edge" }`, log a warning, and let boot recovery skip this parent (BLOCKER-1 fix); **`idempotency_key`** = `ironclaw_turns::IdempotencyKey::new(format!("subagent-resume-await:{parent}:{child}"))` — **specifically `ironclaw_turns::IdempotencyKey`** (the `bounded_ref!` type on `ResumeTurnRequest`), NOT `ironclaw_host_runtime::IdempotencyKey`; colons are valid here (BLOCKER-3). (This fn is written in Task 1.6b — 1.6c reuses the same impl for the boot caller; one impl, not two.)
  - a **boot-recovery routine** run once at composition startup when `subagent_v2_enabled`: `store.list_parents_with_unclosed_edges(scope)` → for each parent `resolver.resolve_parent(scope, parent, parent_status, now)` (read `parent_status` via `get_run_state`); for every `child` in `settled_children`: `resume_parent_for_await_edge(...)` **then** `store.transition(scope, parent, child, Settled, Drained, None, now)` (drain only after resume — F4). This delivers across a restart — the durable edge resumes the parent the lost in-memory gate store cannot. **Enumerates the await-edge directory directly (F3)** — there is no `TurnStateStore` active-run query. Dispatch detached (do not block startup), per design §6.

- [ ] **Step 0 (BLOCKER-2): make the binding-ref helpers callable.** `source_binding_ref(parent, child)` / `reply_target_binding_ref(parent, child)` in `subagent_spawn_port.rs` are private `fn`. Change both to `pub(crate)` and `pub use` re-export them from `crates/ironclaw_loop_support/src/lib.rs` (alongside the existing `pub use subagent_spawn_port::{...}`), so `ironclaw_reborn`'s `await_edge_resolver.rs` can call them. Run `cargo build -p ironclaw_loop_support` to confirm.
- [ ] **Step 1: Read** `subagent_runtime_wiring.rs` for the harness + where the spawn deps and completion observer are built in `runtime.rs`. Note the runtime `ScopedFilesystem` handle. Confirm the binding-ref helpers (Step 0) and read the parent-actor handling at `completion_observer.rs:467` (note it sources `actor` from the terminal event; the boot path uses parent run state's `actor` and errors on `None` per the helper spec).
- [ ] **Step 2: Write the failing integration tests** — copy the harness from `subagent_runtime_wiring.rs`; add the flag toggle. Assert:
  - (a) `subagent_v2_enabled=true`: spawn a blocking child; drive to `Completed`; the await-edge file reaches `drained`; the parent leaves `BlockedDependentRun` and resumes with the result.
  - (b) **restart delivery (the core bet):** spawn a blocking child; drive to `Completed`; **drop and rebuild the composition over the SAME in-memory backend** (and clear in-memory gate state); run boot recovery; assert the parent resumes and the edge is `drained` — proving durable delivery survives restart.
  - (c) `subagent_v2_enabled=false`: same scenario uses the old gate path and writes **no** await-edge file; parent resumes via v1 unchanged.
- [ ] **Step 3: Implement** the shared resume helper, refactor 1.6b to use it, wire the flag-gated construction + injection + the detached boot-recovery routine in `runtime.rs`.
- [ ] **Step 4: Full gate** — `cargo test -p ironclaw_reborn_composition --test subagent_await_edge_e2e` → PASS; `cargo test --workspace` → green; `cargo clippy --all --benches --tests --examples --all-features` → zero warnings
- [ ] **Step 5: Commit**
```bash
cargo fmt && git add -A
git commit -m "feat(reborn): wire flag-gated await-edge delivery for blocking subagents (PR1)"
```

---

## Self-Review

**Spec coverage (design §9 PR 0 + PR 1):** flag → Task 0.2; `wrap_untrusted` visibility → 0.1; blocking status = reuse `BlockedDependentRun` + await-edge gate ref → no PR 0 variant task (per the Option-A decision) and used in 1.1/1.6a; await-edge file + store → 1.1/1.2; resolver + boot resolve → 1.4; crash recovery + idempotency + scope isolation (design §11) → 1.3; depth floor (already exists) → 1.5 (verification); flag-gated blocking delivery → 1.6a/b/c.

**Deferred (tracked, not silently dropped):** scope-wide capacity cap (background fan-out) → PR 2; background drain (`PostCapabilityStage::drain_settled` signature) → PR 2; `subagent_inspect`/`extend`/console/`cancel`, approval-bubbling → later PRs.

**Placeholder scan:** new code (1.1/1.2/1.4) has complete bodies. Test bodies in 1.4/1.6a/1.6b/1.6c are specified as "fake X + named assertions / mirror the existing harness" — concrete assertions, read-then-mirror of real scaffolding, not TODOs.

**Type consistency:** `AwaitEdge`/`AwaitEdgeState`/`EdgeTerminalKind` (1.1) consumed unchanged in 1.2/1.4/1.6. `FilesystemAwaitEdgeStore::transition` identical across 1.2/1.3/1.4. `AwaitEdgeSettler` (1.4) consumed in 1.6b. `AwaitEdgeWriter` (1.6a) consumed in 1.6c. `await_edge_gate_ref` (1.1) used in 1.6a. No new `TurnStatus` variant anywhere (Option A).
