---
name: ironclaw-reborn-architecture-review
description: Use when writing or reviewing a change in crates/ that adds a trait, a crate, a dependency edge, a re-export, or code in ironclaw_reborn_composition — or when deciding whether an abstraction, layer, or crate boundary is justified in the IronClaw Reborn stack.
---

# Reborn Architecture Review

Layer discipline here is enforced by machines, not vibes: `cargo test -p ironclaw_architecture` (38 boundary tests) and the per-crate contract tests are the real reviewers. Your job is the failures machines can't see: **mass pooling inside a crate** and **speculative abstraction**.

## Checklist (run all six; each is checkable)

1. **New trait? Demand the second implementation.** A trait with one production impl is a ritual, not a boundary. Before accepting it, name the concrete second impl (a real backend, not a test fake) or the enforced boundary it serves (e.g. it keeps a forbidden dep out of a crate — verifiable in Cargo.toml). "So we can swap formatters later" / "so callers can hold `Arc<dyn …>`" is the exact rationalization to reject — later can add the trait when later arrives; extraction from a concrete type is mechanical. Justified counter-examples to copy: `RootFilesystem`, `EmbeddingProvider`, `PolicySource`, and dependency-inversion ports whose impl *must* live up-layer (`SkillInferencePort`, `CapabilityDispatcher`). Unjustified precedent to not repeat: the `ironclaw_memory`/`ironclaw_memory_native` split (one provider; on audit watch).
2. **New code in `ironclaw_reborn_composition`? Prove it's assembly.** The crate's charter is service-graph wiring. If your change adds behavior — delivery logic, auth flows, a domain service, a serve surface — it belongs in an owning crate; the host-side-of-a-product role is a real crate (`ironclaw_webui` is the model). Composition gets the `build_*`/`with_*` wiring only.
3. **New dependency edge? Check both the rules and their shape.** Run `cargo test -p ironclaw_architecture`. Know its blind spots: most rules are blocklists, so a *new* crate is unruled by default — if you add a crate, add its boundary rule in the same PR (`crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`, `boundary_rules()`); never add a normal dep on the v1-only enclave (`ironclaw_engine`, `ironclaw_tui`, `ironclaw_gateway`, `ironclaw_oauth`, `ironclaw_embeddings`).
4. **New `pub use`? Name the downstream consumer and the test that closes the direct path.** The house pattern: composition's re-exports each carry a doc-comment citing consumer + boundary test. No glob re-exports of another crate at a crate root (the only sanctioned wildcard shapes: `ironclaw_host_api`'s documented intra-crate prelude, and `ironclaw_reborn_traces`' re-namespaced modules).
5. **New public surface? Copy the visibility kit**: `#![warn(unreachable_pub)]`, `pub(crate)` internals, sealed traits for strategy slots (`ironclaw_agent_loop/src/planner.rs:15-26` is the template), directory-of-modules lib.rs (no re-export wall — a boundary test enforces this for internal crates).
6. **File growing past 1,500 lines (3,000 = tracking issue)?** The rule is real (`.claude/rules/architecture.md` §5), but verify whether the pre-commit check exists before relying on it: `grep -n 'ARCH-SPRAWL' scripts/pre-commit-safety.sh`. Same for `#[allow(clippy::too_many_arguments)]`: require an `// arch-exempt: …, plan #NNNN` line above it; don't add bare allows.

## Rationalizations vs reality

| Rationalization | Reality |
| --- | --- |
| "Trait now, so we can swap later" | One impl = ritual. Add the trait with the second impl; extracting it later is mechanical. |
| "Composition already has similar code, I'll put it next to that" | Existing behavior-heavy code there is composition debt, not precedent. Precedent-by-pollution isn't placement. |
| "The boundary tests passed, so the architecture is fine" | They police edges, not interior mass or abstraction quality — the two ways this codebase actually decays. |
| "It's just a convenience re-export" | Every legitimate re-export here names its consumer and its enforcing test. No test, no re-export. |
| "This crate is in crates/, so it's current architecture" | Five crates in `crates/` are v1-only legacy. Check reverse-deps before building on one. |

## Verify

`cargo test -p ironclaw_architecture` · `bash scripts/check-boundaries.sh` (legacy inventory; may be noisy on HEAD, don't treat as the Reborn architecture gate) · `cargo clippy -p <crate> --all-targets --all-features -- -D warnings` · if routes changed: `cargo test -p ironclaw_webui --test webui_v2_descriptors_contract`.

**Worked good/bad examples** (before/after shapes, live exemplars, re-verify commands): [references/worked-examples.md](references/worked-examples.md) — the living curriculum; update it as the code evolves.
