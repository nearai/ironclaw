# Worked Examples — Good and Bad Reborn Architecture Hygiene

Living curriculum for the checklist in `../SKILL.md`. Each example names its live in-tree exemplar and the command that re-verifies it still holds. Maintained under `ironclaw-reborn-skill-maintainer` rules.

## Contents
- 1. Traits: ritual vs boundary
- 2. Sealing a strategy trait
- 3. Re-exports: facade vs laundering
- 4. Placement: composition vs owning crate
- 5. File budget and `arch-exempt` annotations
- 6. Backend parity done right

## 1. Traits: ritual vs boundary

**BAD — the one-impl ritual**:

```rust
/// "Make it a trait so we can swap formatters later."
pub trait ProjectNameFormatter: Send + Sync {
    fn display_name(&self, name: &str) -> String;
}
pub struct DefaultProjectNameFormatter;   // ...and no second impl, ever
```

Why it's wrong: one production impl means the trait encodes no variation — it's ceremony. "Later" can add the trait mechanically when the second impl actually arrives. The correct first shape is a plain method or `pub(crate)` helper.

**GOOD — traits carrying real variation** (verify each with the grep):

| Exemplar | Variation behind it | Re-verify |
| --- | --- | --- |
| `RootFilesystem` (`crates/ironclaw_filesystem/src/root.rs`) | local / postgres / libsql / in-memory / composite / HSM / memory-adapter | `rg -n "impl RootFilesystem for" crates/ironclaw_filesystem/src crates/ironclaw_memory_native/src` |
| `PolicySource` (`crates/ironclaw_trust/src/sources.rs`) | AdminConfig / BundledRegistry / DevTrustOverride / SignedRegistry | `grep -n "impl PolicySource" crates/ironclaw_trust/src/sources.rs` |

**GOOD — one impl but a real boundary** (the acceptable exception): `SkillInferencePort` (`crates/ironclaw_skills/src/learning.rs`) has one production adapter — `SkillLearningInferenceAdapter` in `crates/ironclaw_extension_host/src/skill_learning.rs`, constructed by composition — because the port exists to keep LLM/runtime deps *out* of the skills domain crate. The justification is verifiable in Cargo.toml (`ironclaw_skills` has no LLM or runtime deps), not in a comment.

## 2. Sealing a strategy trait

When downstream code must be able to *hold* a strategy but never *implement or fork* it, copy the loop planner's shape (`crates/ironclaw_agent_loop/src/planner.rs`):

```rust
mod sealed { pub trait Sealed {} }
impl sealed::Sealed for crate::default_planner::DefaultPlanner {}

pub trait AgentLoopPlanner: sealed::Sealed + Send + Sync { /* ... */ }
pub(crate) trait AgentLoopPlannerInternal: AgentLoopPlanner { /* ... */ }
```

Use `crates/ironclaw_agent_loop/src/planner.rs` as the sealed-trait template, then pair the pattern with `#![warn(unreachable_pub)]` at the owning crate root and `pub(crate)` on strategy modules. Re-verify the template exists: `grep -n "mod sealed" crates/ironclaw_agent_loop/src/planner.rs`.

## 3. Re-exports: facade vs laundering

**GOOD — the house pattern**: a cross-crate re-export is legitimate exactly when a boundary test *closes the direct path*, and the re-export says so. From `crates/ironclaw_reborn_composition/src/lib.rs` (schematic):

```rust
/// Re-exported so `reborn_cli` never depends on `ironclaw_host_api` directly;
/// `reborn_cli_binary_crate_stays_separate_from_v1_root` enforces that edge.
pub mod host_api { pub use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId}; }
```

Consumer named, enforcing test named, items listed explicitly. No test, no re-export.

**BAD — laundering**: `pub use other_crate::*;` at a crate root, or republishing a type your only consumer already imports directly (it makes a common type look locally owned). If you find yourself writing a glob, the answer is a named list plus the question "which test forbids the consumer from importing the origin?"

## 4. Placement: composition vs owning crate

**BAD — precedent-by-pollution**: "Slack's host code lives in `ironclaw_reborn_composition`, so mine can too." Existing delivery observer, host state, setup, and channel route code there is composition debt — not precedent.

**GOOD — the host-side product crate**: `ironclaw_webui` is the model. It owns WebChat's listener, auth, and serve loop as its *own crate*, entering through the same composition seams as everything else. A new channel gets: a protocol-pure adapter crate (parse/render only — the boundary test bans host auth/credentials/delivery from it) **plus** a host-side crate for serving/verification/delivery, **plus** its dependency rule added to `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs` in the same PR. Composition gets only `build_*`/`with_*` wiring.

## 5. File budget and `arch-exempt` annotations

**GOOD** (from `crates/ironclaw_hooks/src/dispatch/mod.rs` — every over-limit allow carries the required annotation with a plan link):

```rust
// arch-exempt: too_many_args, needs HookInstallContext aggregation, plan #4088
#[allow(clippy::too_many_arguments)]
```

**BAD**: the bare `#[allow(clippy::too_many_arguments)]` with no annotation. Don't add bare allows; if you can't write the aggregation plan, that's the signal to do the aggregation instead.

## 6. Backend parity done right

Dual-backend persistence is not copy-paste-twice. The exemplar lives inside `ironclaw_hooks`: `src/postgres_backend/` (advisory locks, deadlock-free eviction) and `src/libsql_backend/` (single-writer mutex, `BEGIN IMMEDIATE`) implement one contract with *deliberately different* concurrency designs — and `tests/parity_matrix.rs` (+ `tests/parity_matrix/`, `tests/multi_host_adversarial.rs`) drives all backends through one adversarial scripted sequence and asserts byte-identical outcomes. Copy that shape for any new dual-backend surface. Re-verify by running the equivalence proof, not just by looking for the files: `cargo test -p ironclaw_hooks --features integration --test parity_matrix --test multi_host_adversarial` (18 tests; the `integration` feature is required — the targets do not build without it). Optional path check: `ls crates/ironclaw_hooks/src crates/ironclaw_hooks/tests`. (These were once three separate crates — `ironclaw_hooks_postgres`/`_libsql`/`_parity`; they no longer exist.)
