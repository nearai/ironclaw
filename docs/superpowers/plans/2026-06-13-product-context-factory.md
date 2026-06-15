# ProductContextFactory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the scattered, branch-only `run_origin` plumbing with a single ingress resolver (`ironclaw_product_context`) that stamps a generic `ProductTurnContext` onto every turn.

**Architecture:** A generic, product-agnostic `ProductTurnContext` lives in `ironclaw_turns` (the bottom crate that defines `SubmitTurnRequest`). A new `ironclaw_product_context` crate owns the pure resolver (`resolve_inbound`, `resolve_web_ui`) — the single place trust/adapter/surface/owner become an origin. The four submit sites map their rich local types to generic inputs and call the resolver; downstream code reads the persisted context.

**Tech Stack:** Rust, tokio, serde, cargo workspace. Spec: `docs/superpowers/specs/2026-06-13-product-context-factory-design.md`.

**Working dir:** repo root (worktree on branch `context-slice-4828`).

**Conventions:** No `.unwrap()`/`.expect()` in production code (tests fine). No edits under `assets/prompts/`. Run `cargo fmt` before each commit. The repo gate is `cargo clippy --all --benches --tests --examples --all-features` (zero warnings) + `cargo test`.

---

## File Structure

- `crates/ironclaw_turns/src/origin.rs` — **rewrite**: delete `TurnRunOrigin`; add `TurnOriginKind`, `TurnSurfaceType`, `RunOriginAdapter`, `TurnOwner`, `ProductTurnContext`.
- `crates/ironclaw_turns/src/{request.rs,status.rs,store.rs,run_profile/host.rs,memory.rs,lib.rs}` — **modify**: swap the `run_origin` field for `product_context`.
- `crates/ironclaw_product_context/` — **create**: new crate (`Cargo.toml`, `src/lib.rs`) with `InboundClassification` + resolver.
- `Cargo.toml` (workspace root) — **modify**: add the new crate to members.
- `crates/ironclaw_conversations/{Cargo.toml,src/inbound.rs}` — **modify**: depend on `product_context`; call `resolve_inbound`.
- `crates/ironclaw_product_workflow/{Cargo.toml,src/inbound_turn.rs,src/reborn_services.rs}` — **modify**: depend on `product_context`; call resolver.
- `crates/ironclaw_reborn_composition/{Cargo.toml,src/runtime.rs,src/communication_context.rs}` — **modify**: depend on `product_context`; resolver at local-dev webui; provider passes context.
- `crates/ironclaw_reborn/src/loop_driver_host.rs`, `crates/ironclaw_turns/src/run_profile/runtime_context.rs` — **modify**: carry/render `product_context`.

---

## Task 1: Generic context types in `ironclaw_turns`

**Files:**
- Modify (rewrite): `crates/ironclaw_turns/src/origin.rs`
- Modify: `crates/ironclaw_turns/src/lib.rs:72`

- [ ] **Step 1: Write the failing test** — append to `crates/ironclaw_turns/src/origin.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_turn_context_round_trips_through_json() {
        let ctx = ProductTurnContext {
            origin: TurnOriginKind::Inbound,
            surface_type: Some(TurnSurfaceType::Channel),
            adapter: Some(RunOriginAdapter::new("telegram").unwrap()),
            owner: TurnOwner::Personal {
                user: ironclaw_host_api::UserId::new("u1").unwrap(),
            },
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let back: ProductTurnContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, back);
    }

    #[test]
    fn run_origin_adapter_rejects_empty() {
        assert!(RunOriginAdapter::new("").is_err());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ironclaw_turns origin:: 2>&1 | tail -5`
Expected: FAIL — `ProductTurnContext` / `TurnOriginKind` not found.

- [ ] **Step 3: Rewrite `crates/ironclaw_turns/src/origin.rs`** (replace the entire `TurnRunOrigin` definition above the test module with):

```rust
use serde::{Deserialize, Serialize};

use ironclaw_host_api::{AgentId, ProjectId, UserId};

/// How this turn run was initiated. Generic — no product/channel specifics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOriginKind {
    WebUi,
    Inbound,
    ScheduledTrigger,
}

/// The conversation surface a turn arrived on / replies to. Generic dm-vs-channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSurfaceType {
    Direct,
    Channel,
}

/// Generic adapter identity carried into the turn context. Bounded validated string;
/// callers convert their rich adapter id (e.g. `ProductAdapterId`, `AdapterKind`) into this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOriginAdapter(String);

/// Mirrors `AdapterKind`'s validation bound in `ironclaw_conversations` so that
/// any valid `AdapterKind` always converts without silent narrowing. If
/// `AdapterKind`'s limit changes, update this constant to match.
const MAX_RUN_ORIGIN_ADAPTER_BYTES: usize = 512;

impl RunOriginAdapter {
    pub fn new(value: impl Into<String>) -> Result<Self, crate::TurnError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_RUN_ORIGIN_ADAPTER_BYTES {
            return Err(crate::TurnError::InvalidRunOriginAdapter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who owns this turn, for delivery-preference scoping and slice rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnOwner {
    Personal {
        user: UserId,
    },
    SharedAgent {
        agent: AgentId,
        project: Option<ProjectId>,
    },
}

/// Generic, persisted product context for one turn. Resolved once at ingress by
/// `ironclaw_product_context`; rendered into the model-visible runtime context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductTurnContext {
    pub origin: TurnOriginKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_type: Option<TurnSurfaceType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<RunOriginAdapter>,
    pub owner: TurnOwner,
}
```

- [ ] **Step 4: Add the error variant** — in `crates/ironclaw_turns/src/status.rs` (find the `TurnError` enum; if errors live elsewhere, grep `enum TurnError`), add:

```rust
    #[error("invalid run-origin adapter: must be 1..=512 bytes")]
    InvalidRunOriginAdapter,
```

(If `TurnError` is not the right type or not `thiserror`, define `RunOriginAdapter::new` to return `Result<Self, &'static str>` instead and adjust the test to `.is_err()`.)

- [ ] **Step 5: Update exports** — `crates/ironclaw_turns/src/lib.rs:72`, replace `pub use origin::TurnRunOrigin;` with:

```rust
pub use origin::{
    ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnOwner, TurnSurfaceType,
};
```

- [ ] **Step 6: Run to verify pass**

Run: `cargo test -p ironclaw_turns origin:: 2>&1 | tail -5`
Expected: PASS (2 tests). NOTE: the crate will still have other compile errors from `run_origin` references — those are fixed in Task 3. If `cargo test` cannot build the crate yet, run `cargo test -p ironclaw_turns --lib origin 2>&1 | tail` after Task 3 instead; for now confirm `origin.rs` itself type-checks via `cargo build -p ironclaw_turns 2>&1 | grep "origin.rs"` showing no errors in that file.

- [ ] **Step 7: Commit**

```bash
git add crates/ironclaw_turns/src/origin.rs crates/ironclaw_turns/src/lib.rs crates/ironclaw_turns/src/error.rs
git commit -m "feat(turns): generic ProductTurnContext types, replacing TurnRunOrigin"
```

---

## Task 2: New `ironclaw_product_context` crate + resolver

**Files:**
- Create: `crates/ironclaw_product_context/Cargo.toml`
- Create: `crates/ironclaw_product_context/src/lib.rs`
- Modify: workspace root `Cargo.toml` (members list)

- [ ] **Step 1: Create `crates/ironclaw_product_context/Cargo.toml`**

```toml
[package]
name = "ironclaw_product_context"
version = "0.1.0"
edition = "2021"

[dependencies]
ironclaw_turns = { path = "../ironclaw_turns", version = "0.1.0" }
ironclaw_host_api = { path = "../ironclaw_host_api", version = "0.1.0" }

[dev-dependencies]
```

(Match the `edition`/version style of a sibling crate's `Cargo.toml` if it differs.)

- [ ] **Step 2: Add to workspace members** — root `Cargo.toml`, in `[workspace] members = [...]`, add `"crates/ironclaw_product_context"` (keep the list sorted if it is).

- [ ] **Step 3: Write the failing test** — create `crates/ironclaw_product_context/src/lib.rs` with the test first:

```rust
//! Single owner of turn-origin/surface/owner classification at ingress.

use ironclaw_turns::{
    ProductTurnContext, RunOriginAdapter, TurnOriginKind, TurnOwner, TurnSurfaceType,
};

/// Ingress classification. Callers collapse their (trust policy, trigger-adapter) signal
/// into one value, so the resolver cannot receive a contradictory pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundClassification {
    /// Trusted ingress whose adapter is the trusted-trigger adapter.
    TrustedTrigger,
    /// Trusted ingress, non-trigger adapter.
    TrustedOther,
    /// Untrusted ingress (adapter identity is irrelevant — never a trigger).
    Untrusted,
}

/// Resolve an inbound submission into a generic product context.
///
/// `ScheduledTrigger` is minted ONLY when `classification == TrustedTrigger`.
/// Any other combination yields `Inbound` — an untrusted caller cannot mint a trigger origin.
pub fn resolve_inbound(
    classification: InboundClassification,
    adapter: RunOriginAdapter,
    surface_type: Option<TurnSurfaceType>,
    owner: TurnOwner,
) -> ProductTurnContext {
    let origin = match classification {
        InboundClassification::TrustedTrigger => TurnOriginKind::ScheduledTrigger,
        InboundClassification::TrustedOther | InboundClassification::Untrusted => {
            TurnOriginKind::Inbound
        }
    };
    ProductTurnContext::new(origin, surface_type, Some(adapter), owner)
}

/// Resolve a WebUI submission. Always `WebUi`, no adapter/surface.
pub fn resolve_web_ui(owner: TurnOwner) -> ProductTurnContext {
    ProductTurnContext::new(TurnOriginKind::WebUi, None, None, owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::UserId;

    fn owner() -> TurnOwner {
        TurnOwner::Personal { user: UserId::new("u1").unwrap() }
    }
    fn adapter() -> RunOriginAdapter {
        RunOriginAdapter::new("trigger").unwrap()
    }

    #[test]
    fn trusted_trigger_adapter_yields_scheduled_trigger() {
        let ctx = resolve_inbound(InboundClassification::TrustedTrigger, adapter(), None, owner());
        assert_eq!(ctx.origin, TurnOriginKind::ScheduledTrigger);
    }

    #[test]
    fn untrusted_trigger_adapter_yields_inbound_not_trigger() {
        let ctx = resolve_inbound(InboundClassification::Untrusted, adapter(), None, owner());
        assert_eq!(ctx.origin, TurnOriginKind::Inbound);
    }

    #[test]
    fn trusted_non_trigger_adapter_yields_inbound() {
        let a = RunOriginAdapter::new("telegram").unwrap();
        let ctx = resolve_inbound(InboundClassification::TrustedOther, a, Some(TurnSurfaceType::Channel), owner());
        assert_eq!(ctx.origin, TurnOriginKind::Inbound);
        assert_eq!(ctx.surface_type, Some(TurnSurfaceType::Channel));
    }

    #[test]
    fn web_ui_yields_web_ui_origin_no_adapter() {
        let ctx = resolve_web_ui(owner());
        assert_eq!(ctx.origin, TurnOriginKind::WebUi);
        assert!(ctx.adapter.is_none());
    }
}
```

- [ ] **Step 4: Run to verify pass** (the impl is in the same file, so it should pass directly)

Run: `cargo test -p ironclaw_product_context 2>&1 | tail -5`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/ironclaw_product_context Cargo.toml
git commit -m "feat(product-context): ingress resolver crate (resolve_inbound/resolve_web_ui)"
```

---

## Task 3: Swap `run_origin` → `product_context` in `ironclaw_turns`

**Files:**
- Modify: `crates/ironclaw_turns/src/request.rs:63`, `status.rs:324`, `store.rs:195`, `run_profile/host.rs:552,580,603`, `memory.rs` (child-run + persist/restore sites)
- Modify: in-crate test literals across `crates/ironclaw_turns/tests/*.rs` and `crates/ironclaw_turns/src/events.rs`

- [ ] **Step 1: Replace the field in the four struct defs.** In each of `request.rs:63`, `status.rs:324`, `host.rs:552`, change:

```rust
    pub run_origin: Option<TurnRunOrigin>,
```
to:
```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
```
In `store.rs:195` change `Option<crate::TurnRunOrigin>` → `Option<crate::ProductTurnContext>` with the same serde attrs. Update each file's imports (`use crate::TurnRunOrigin` → `use crate::ProductTurnContext`, or the glob).

- [ ] **Step 2: Update `LoopRunContext` builder** — `run_profile/host.rs`: rename `with_run_origin` → `with_product_context`, init `product_context: None` (line ~580), and the setter (line ~603):

```rust
    pub fn with_product_context(mut self, product_context: ProductTurnContext) -> Self {
        self.product_context = Some(product_context);
        self
    }
```

- [ ] **Step 3: Update `memory.rs`.** At the child-run `SubmitTurnRequest` template and the child `RunRecord` (the two sites that currently read `parent.run_origin.clone()`), rename to `parent.product_context.clone()` / `product_context: ...`. At the top-level submit (carries `request.run_origin`) and the persist/restore (`persistence_record()` copy, snapshot restore), rename `run_origin` → `product_context` consistently. Grep to find all: `grep -n "run_origin" crates/ironclaw_turns/src/memory.rs`.

- [ ] **Step 4: Delete the old enum reference paths.** `origin.rs` no longer defines `TurnRunOrigin` (done in Task 1). Confirm no `TurnRunOrigin` remains in the crate:

Run: `grep -rn "TurnRunOrigin" crates/ironclaw_turns/`
Expected: only matches inside test files you will fix next (ideally none).

- [ ] **Step 5: Fix in-crate test/struct literals.** Every `SubmitTurnRequest { … }`, `TurnRunState { … }`, `TurnRunRecord { … }`, `LoopRuntimeContext`/`LoopRunContext` literal that set `run_origin: None` must become `product_context: None`. Find them:

Run: `grep -rln "run_origin" crates/ironclaw_turns/`
For each file, replace `run_origin: None,` → `product_context: None,` and any `run_origin: Some(TurnRunOrigin::X)` → a `ProductTurnContext { … }` literal. Also `crates/ironclaw_turns/src/events.rs` test literal.

- [ ] **Step 6: Run to verify the crate builds + tests pass**

Run: `cargo test -p ironclaw_turns 2>&1 | grep -E "test result|error\[" | tail -20`
Expected: all suites `ok`, zero `error[`.

- [ ] **Step 7: Commit**

```bash
git add crates/ironclaw_turns
git commit -m "refactor(turns): carry product_context on turns, drop run_origin"
```

---

## Task 4: Wire the four ingress call sites

**Files:**
- Modify: `crates/ironclaw_conversations/{Cargo.toml,src/inbound.rs}`
- Modify: `crates/ironclaw_product_workflow/{Cargo.toml,src/inbound_turn.rs,src/reborn_services.rs}`
- Modify: `crates/ironclaw_reborn_composition/{Cargo.toml,src/runtime.rs}`

- [ ] **Step 1: Add the dependency** to all three crates' `[dependencies]`:

```toml
ironclaw_product_context = { path = "../ironclaw_product_context", version = "0.1.0" }
```

- [ ] **Step 2: Add a `TurnOwner` helper.** The owner derivation from `TurnScope` is shared. Add to `crates/ironclaw_turns/src/scope.rs` (so all callers reuse it):

```rust
impl TurnScope {
    /// Owner for product-context: explicit thread owner / actor → Personal,
    /// otherwise the agent-scoped owner.
    pub fn product_owner(&self, actor: &TurnActor) -> crate::TurnOwner {
        if let Some(user) = self.explicit_owner_user_id() {
            crate::TurnOwner::Personal { user: user.clone() }
        } else if let Some(agent) = &self.agent_id {
            crate::TurnOwner::SharedAgent {
                agent: agent.clone(),
                project: self.project_id.clone(),
            }
        } else {
            crate::TurnOwner::Personal { user: actor.user_id.clone() }
        }
    }
}
```

Add a unit test for it in `scope.rs` (`product_owner_prefers_explicit_then_agent_then_actor`). Run `cargo test -p ironclaw_turns scope:: ` → PASS. Commit this with Task 3's crate or separately.

- [ ] **Step 3: Conversations inbound** — `crates/ironclaw_conversations/src/inbound.rs`. The classification is derived from the typed `BindingResolutionPolicy`, not from `adapter_kind.is_trusted_trigger()`. Near the top of `handle_inbound_turn_inner` (where `binding_policy`, `route_kind`, and `adapter_kind` are in scope), add:

```rust
        // Origin classification is derived from the typed trust policy, never
        // re-derived from the adapter-kind string. `TrustedTrigger` is reachable
        // only when the trusted-trigger submit seam built this request with
        // `TrustedInboundKind::Trigger`; see `.claude/rules/types.md`.
        let classification = match &binding_policy {
            BindingResolutionPolicy::Trusted {
                kind: TrustedInboundKind::Trigger,
                ..
            } => ironclaw_product_context::InboundClassification::TrustedTrigger,
            BindingResolutionPolicy::Trusted { .. } => {
                ironclaw_product_context::InboundClassification::TrustedOther
            }
            BindingResolutionPolicy::Untrusted => {
                ironclaw_product_context::InboundClassification::Untrusted
            }
        };
        let surface_type = match &route_kind {
            ConversationRouteKind::Direct => Some(ironclaw_turns::TurnSurfaceType::Direct),
            ConversationRouteKind::Shared => Some(ironclaw_turns::TurnSurfaceType::Channel),
        };
        let run_adapter = ironclaw_turns::RunOriginAdapter::new(adapter_kind.as_str())
            .map_err(|e| InboundTurnError::InvalidCanonicalRef { reason: e.to_string() })?;
```

Then, **at the point in the function where the resolved `TurnScope` (`turn_scope`/`resolution.turn_scope`) and `actor` are both in scope** (below the binding resolution, where the `SubmitTurnRequest` is built), construct:

```rust
        let product_context = ironclaw_product_context::resolve_inbound(
            classification,
            run_adapter,
            surface_type,
            turn_scope.product_owner(&actor),
        );
```

Implementer notes:
- The `TrustedInboundKind` variant carried on `BindingResolutionPolicy::Trusted { kind }` is the typed signal from the trusted-trigger submit seam (`trusted_inbound_request_from_trigger` passes `TrustedInboundKind::Trigger`). Do not use `adapter_kind.is_trusted_trigger()` — that re-derives trigger-ness from the adapter-kind string, which the typed seam was specifically introduced to replace.
- First confirm the exact names: `grep -n "enum ConversationRouteKind" -A4 crates/ironclaw_conversations/src/` (variant names for the `surface_type` match). `AdapterKind` is bounded, so the error is unreachable in practice — still handle it, never `unwrap`.
- `classification`, `surface_type`, and `run_adapter` are computed near the top (where `adapter_kind`/`route_kind`/`binding_policy` exist); `product_context` is assembled lower where `turn_scope` + `actor` exist. Move the pieces accordingly rather than forcing one block.
- Thread `product_context` to the `SubmitTurnRequest`. Rename the `submit_or_replay` / `handle_inbound_turn_inner` `run_origin: Option<…>` parameter (line ~187) to `product_context: Option<ProductTurnContext>` and pass it through.

- [ ] **Step 4: Product-workflow inbound** — `crates/ironclaw_product_workflow/src/inbound_turn.rs:637`. The `AcceptedProductInboundTurn` already carries `adapter_id: ProductAdapterId`. Replace the `run_origin: Some(TurnRunOrigin::ProductInbound { adapter: … })` with:

```rust
            product_context: Some(ironclaw_product_context::resolve_inbound(
                ironclaw_product_context::InboundClassification::Untrusted,
                ironclaw_turns::RunOriginAdapter::new(self.adapter_id.as_str())
                    .map_err(|e| ProductWorkflowError::Transient { reason: e.to_string() })?,
                self.route_surface_type(), // map ResolvedBinding/route → Option<TurnSurfaceType>; if unavailable, None
                self.thread_scope_owner(),  // TurnOwner from thread_scope + actor
            )),
```

Notes: if a generic route/surface signal isn't readily available on `AcceptedProductInboundTurn`, pass `None` for `surface_type` (the live slice still renders origin); add a `// TODO(#follow-up): thread surface_type` only if genuinely unavailable. Derive owner from the `thread_scope`/actor already on the struct.

- [ ] **Step 5: WebUI (product_workflow)** — `reborn_services.rs:1728`. Replace `run_origin: Some(TurnRunOrigin::WebUiChat)` with:

```rust
            product_context: Some(ironclaw_product_context::resolve_web_ui(scope.product_owner(&actor))),
```

(Use the `scope` + `actor` in scope at that submit; confirm names.)

- [ ] **Step 6: WebUI (local-dev composition)** — `reborn_composition/src/runtime.rs:1312`. Same replacement as Step 5 with the local-dev `scope`/`actor`.

- [ ] **Step 7: Build + test the wired crates**

Run: `cargo test -p ironclaw_conversations -p ironclaw_product_workflow -p ironclaw_reborn_composition 2>&1 | grep -E "test result|error\[" | tail -25`
Expected: all `ok`, zero `error[`. Fix any remaining `run_origin`/`TurnRunOrigin` literals in those crates' tests (replace with `product_context`).

- [ ] **Step 8: Commit**

```bash
git add crates/ironclaw_conversations crates/ironclaw_product_workflow crates/ironclaw_reborn_composition crates/ironclaw_turns/src/scope.rs
git commit -m "refactor: resolve product_context at the four ingress submit sites"
```

---

## Task 5: Carry + render `product_context` (host + slice)

**Files:**
- Modify: `crates/ironclaw_reborn/src/loop_driver_host.rs` (create_host ~2035; provider call ~1525)
- Modify: `crates/ironclaw_turns/src/run_profile/runtime_context.rs` (`CommunicationRuntimeContext`, `CommunicationContextProvider`, `render_model_content`)
- Modify: `crates/ironclaw_reborn_composition/src/communication_context.rs` (provider impl)
- Modify: tests in `crates/ironclaw_reborn/tests/loop_driver_host.rs`

- [ ] **Step 1: create_host carries product_context** — `loop_driver_host.rs:2035`, replace the `with_run_origin` block:

```rust
        if let Some(product_context) = claimed.state.product_context.clone() {
            loop_run_context = loop_run_context.with_product_context(product_context);
        }
```

- [ ] **Step 2: Update `CommunicationContextProvider` + `CommunicationRuntimeContext`** — `runtime_context.rs`. Replace the `run_origin: Option<TurnRunOrigin>` field on `CommunicationRuntimeContext` with `product_context: Option<ProductTurnContext>` (or flatten to `origin`/`surface_type`/`owner` — pick one; the spec keeps `ProductTurnContext`). Change the trait method signature's `run_origin: Option<TurnRunOrigin>` param to `product_context: Option<ProductTurnContext>`.

- [ ] **Step 3: Render** — in `render_model_content`, replace the `TurnRunOrigin` match with a `TurnOriginKind` match that uses `surface_type` for the inbound line:

```rust
        if let Some(pc) = &comm.product_context {
            let origin_line = match pc.origin {
                TurnOriginKind::WebUi => "Run origin: WebUI chat; replies render in this chat.".to_string(),
                TurnOriginKind::Inbound => {
                    let surface = match pc.surface_type {
                        Some(TurnSurfaceType::Channel) => "this channel",
                        _ => "this conversation",
                    };
                    let adapter = pc.adapter.as_ref().map(|a| sanitize_prompt_string(a.as_str()))
                        .unwrap_or_else(|| "a connected product".to_string());
                    format!("Run origin: inbound message via {adapter}; replies post back to {surface}.")
                }
                TurnOriginKind::ScheduledTrigger => "Run origin: scheduled trigger fire.".to_string(),
            };
            parts.push(origin_line);

            if pc.origin == TurnOriginKind::ScheduledTrigger
                && matches!(comm.delivery_target, DeliveryTargetState::NoneSet)
            {
                // (keep the existing tool-visibility-gated warning text)
            }
        }
```

- [ ] **Step 4: Provider impl** — `communication_context.rs`: the provider's `communication_context(...)` takes `product_context: Option<ProductTurnContext>` and stores it on the returned `CommunicationRuntimeContext` (the live channel/delivery fetch is unchanged).

- [ ] **Step 5: Update host tests** — `loop_driver_host.rs` tests: the `RecordingCommunicationContextProvider` and the stub provider take/return `product_context`; the fixture that seeds `claimed.state.run_origin` now seeds `claimed.state.product_context = Some(ProductTurnContext { origin: ScheduledTrigger, .. })`; the assertion still checks the model request contains `"Run origin: scheduled trigger fire."`.

- [ ] **Step 6: Build + test**

Run: `cargo test -p ironclaw_turns -p ironclaw_reborn -p ironclaw_reborn_composition 2>&1 | grep -E "test result|error\[" | tail -25`
Expected: all `ok`, zero `error[`.

- [ ] **Step 7: Commit**

```bash
git add crates/ironclaw_reborn crates/ironclaw_turns crates/ironclaw_reborn_composition
git commit -m "refactor: render ProductTurnContext in the communication slice"
```

---

## Task 6: Full gate + dead-code sweep

**Files:** none new — verification + cleanup.

- [ ] **Step 1: Confirm no stale references**

Run: `grep -rn "TurnRunOrigin\|run_origin\|with_run_origin" crates/ tests/`
Expected: zero matches. Fix any stragglers (replace with `product_context`).

- [ ] **Step 2: fmt + clippy**

Run from the repo root: `cargo fmt && cargo clippy --all --benches --tests --examples --all-features 2>&1 | grep -cE "^warning:|^error"`
Expected: `0`.

- [ ] **Step 3: Full test gate**

Run: `cargo test 2>&1 | grep -E "test result: FAILED|^error" | head`
Expected: no output (zero failures). (Pre-existing parallel-run SIGABRT flake in the root `ironclaw` lib tests is unrelated — re-run `cargo test --lib -- --test-threads=1` to confirm green if it appears.)

- [ ] **Step 4: Commit + push**

```bash
git add -A
git commit -m "chore: product-context refactor — fmt/clippy/test gate green"
git push origin context-slice-4828
```

---

## Out of scope (do NOT implement here — tracked follow-ups from the spec)

- (b) Owner unification: `OutboundResolutionEngine` consuming the persisted `TurnOwner` instead of re-deriving the `CommunicationPreferenceKey`.
- Real channel-surface classification (replace the `extension_is_channel_surface` stub) — lands with #4778.
- `delivery_tools_visible` computed at the prompt boundary (two-truths fix).
- Untrusted-label prompt hardening (JSON-escaped/opaque-id rendering).
