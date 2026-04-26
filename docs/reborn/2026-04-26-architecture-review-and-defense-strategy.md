# IronClaw Reborn — Architecture Review and Defense Strategy

**Status:** External review / synthesis
**Date:** 2026-04-26
**Reviewer:** Illia (with Claude Code as conversational partner)
**Reviewed branches:**

- `reborn-architecture-docs` (5 design docs, 4,267 lines)
- `reborn-host-runtime-filesystem-example` (cumulative tip of the implementation stack — 107 commits, 15 crates)

**Related docs:**

- `docs/reborn/2026-04-24-os-like-architecture-design.md`
- `docs/reborn/2026-04-24-architecture-faq-decisions.md`
- `docs/reborn/2026-04-24-self-contained-crate-roadmap.md`
- `docs/reborn/2026-04-24-existing-code-reuse-map.md`
- `docs/reborn/2026-04-24-host-api-invariants-and-authorization.md`
- `docs/reborn/2026-04-24-os-like-architecture-feedback.md`
- `docs/reborn/2026-04-25-current-architecture-map.md`

---

## 1. Scope

This review focuses on **the gap between the design docs and the code**, plus the operational question that follows from it:

> Given the project expects hundreds of contributors using AI tools to write code, how do we eliminate the risk of breaking the kernel?

It does not duplicate the architectural rationale already covered in the 2026-04-24 docs, the prior feedback document, or the current-architecture-map snapshot. It assumes those as background.

What this review covers:

1. The implementation status across the 15 reborn crates, vs. the 11 originally proposed
2. Where the design holds up under inspection
3. Where the implementation has cracks worth addressing now
4. Whether invariants are actually declared and enforced between crates, or just described in prose
5. A prioritized tooling stack for invariant verification (proptest, loom, Kani, mutation testing, architecture tests)
6. A five-layer defense strategy for kernel integrity under hundreds of AI-tool-assisted contributors
7. Concrete next steps with effort estimates

---

## 2. Verdict in five lines

- The bones are right: `ironclaw_host_api` is cleanly isolated, authority types are not duplicated, every system service is trait-driven, and the filesystem-backed vertical slice runs end-to-end.
- The middle layer (`dispatcher` / `capabilities` / `approvals`) shows stack-of-PRs decomposition rather than a designed boundary.
- Three crates from the original proposal are still missing: `kernel`, `auth`, `network`. `host_runtime` is functionally the kernel; the docs should reflect that.
- Invariants are declared in three places (rustdoc, contract docs, test names) and enforced almost nowhere mechanically.
- The discipline holds today because the original authors are writing every PR. It will start to drift the first time a contributor outside that core works in the kernel.

---

## 3. Where the design holds up

These are the architectural properties most worth defending; they are the foundation everything else rests on.

### 3.1 `ironclaw_host_api` isolation is correct

Verified: `crates/ironclaw_host_api/Cargo.toml` lists zero `ironclaw_*` workspace dependencies. No system-service crate is referenced. Authority types (`ExecutionContext`, `ResourceScope`, `CapabilityGrant`, `CapabilityDescriptor`, `Action`, `Decision`, `MountView`, `ScopedPath`, `VirtualPath`) all live there and are consumed by every other crate without redefinition.

This is the single most important architectural property in the stack. If `host_api` were to drift — accept a circular import, leak a runtime concept, grow a behaviour helper — the layering of the entire system would weaken simultaneously. It hasn't.

### 3.2 Trait-first interfaces everywhere

Every system service is defined as a trait with at least one in-memory test impl and at least one production impl:

| Trait | Production impl | Test impl |
|---|---|---|
| `RootFilesystem` | `LocalFilesystem` | (in-memory) |
| `ResourceGovernor` | — | `InMemoryResourceGovernor` |
| `ScriptBackend` | `DockerScriptBackend` | in-process closures |
| `WasmHostFilesystem` | `WasmScopedFilesystem<F>` | — |
| `McpClient` | host-pluggable | — |
| `CapabilityDispatchAuthorizer` | `GrantAuthorizer` | — |
| `RunStateStore` | `InMemoryRunStateStore` | — |
| `ApprovalRequestStore` | `InMemoryApprovalRequestStore` | — |
| `EventSink` | `InMemoryEventSink`, `JsonlEventSink` | — |
| `ProcessStore` | (filesystem-backed via `ProcessServices::filesystem`) | in-memory |

This means system-service swappability is real, not aspirational. A future `S3RootFilesystem`, `PostgresRunStateStore`, or `OpenTelemetryEventSink` can be added by implementing the trait and wiring at composition time.

### 3.3 Authority types are not duplicated

`ExecutionContext`, `ResourceScope`, `CapabilityGrant`, `Action`, `Decision`, `MountView` exist in exactly one place (`host_api`) and are imported everywhere they are needed. No crate redefines a parallel `ResourceScope` for its own use, and no crate accepts a stringly-typed scope where the typed one belongs.

### 3.4 Test discipline is unusually strong

Test names read as an invariant checklist, not a feature exercise:

```
scoped_path_rejects_raw_host_paths_urls_and_traversal
mount_view_denies_unknown_alias_and_broader_child_permissions
concurrent_reservations_cannot_oversubscribe_scope
capability_access_denies_when_grant_is_for_different_principal_or_capability
spawn_access_requires_spawn_process_effect_in_addition_to_capability_effects
extension_id_rejects_path_like_or_uppercase_values
reserve_denies_runtime_quota_even_without_usd
tenant_limit_applies_across_projects
invocation_fingerprint_is_stable_and_input_redacted
```

These are claims about behaviour, not exercises of code. That discipline is rare and load-bearing — it makes the test suite a contract document rather than a regression-fixture pile.

### 3.5 The vertical slice works end-to-end

The filesystem-backed example (`crates/ironclaw_host_runtime/examples/reborn_host_runtime_filesystem.rs`) compiles clean and runs to completion:

```text
process_id=2550053d-2c54-4919-a19a-03756f5d2899
status=Completed
metadata_ref=/engine/tenants/tenant1/users/user1/process-results/<pid>.json
output_ref=/engine/tenants/tenant1/users/user1/process-outputs/<pid>/output.json
output={"message":"hello from filesystem HostRuntimeServices"}
```

What this proves: `HostRuntimeServices` composition root → `CapabilityHost::spawn_json` → `ProcessServices::filesystem` → durable artifact persisted under `/engine/tenants/{tenant}/users/{user}/...`. The same `spawn_json` call would produce in-memory results when given `ProcessServices::in_memory`. The OS-architecture claim that "filesystem is the universal persistence surface" is not aspirational — it runs.

---

## 4. Where the design has cracks

These are real concerns worth addressing now, while the surface is still small enough to refactor cheaply.

### 4.1 `ironclaw_dispatcher` knows about every runtime lane

`crates/ironclaw_dispatcher/Cargo.toml` lists `ironclaw_wasm`, `ironclaw_mcp`, and `ironclaw_scripts` as normal dependencies.

Consequences:

- Adding a new runtime lane requires editing `dispatcher`, violating open-closed.
- Every consumer of `dispatcher` transitively pulls all three runtime backends, including `wasmtime` and Docker client deps.
- The OS architecture docs explicitly call out "future runtime providers" as a design goal. The current shape fights that.

The clean shape: `dispatcher` defines a `RuntimeAdapter` trait. Each runtime crate (`ironclaw_wasm`, `ironclaw_mcp`, `ironclaw_scripts`) implements it. The composition root (`host_runtime`) registers the adapters at startup. `dispatcher` knows the trait, not the implementations.

This is the same pattern the docs already apply to filesystem and resource governance. The discipline broke down at the runtime-routing layer.

### 4.2 `dispatcher` / `capabilities` split looks like a stack-of-PRs artifact

| Crate | Source LoC | Test LoC | Test:src ratio |
|---|---:|---:|---:|
| `ironclaw_dispatcher` | 549 | 1,343 | 2.45× |
| `ironclaw_capabilities` | 798 | 2,179 | 2.73× |

Both crates sit in the orchestration layer. `capabilities` already depends on `authorization`, `dispatcher`, `approvals`, `run_state`, `processes`, `extensions`. It owns both `CapabilityHost` (the caller-facing facade) and `DispatchProcessExecutor` (the bridge between dispatch and process spawn). Two responsibilities glued together.

The 3:1 test-to-source ratio is a tell. When the test suite is three times the size of the source, the source is usually doing orchestration and the value lives in proving cross-component invariants. That is what this code is. But orchestration that thin doesn't always merit its own crate — it often wants to be a module under whichever neighbouring crate owns the consumer-facing API.

The branch history (107 commits, one branch per concept) is consistent with "split for review" rather than "split for modularity." Splitting work for review is good. Shipping the split as separate crates is not.

**Recommendation**: either (a) merge `dispatcher` into `capabilities` and rename the merged crate to reflect what it is, or (b) extract `DispatchProcessExecutor` out of `capabilities` so `capabilities` is the caller facade and `dispatcher` is the runtime router, and they have a clean unidirectional dependency.

### 4.3 `ironclaw_approvals` at 114 source lines is not a crate

It contains `ApprovalResolver`, `LeaseApproval`, `ApprovalResolutionError`. It depends on `authorization` and `run_state`. It has no behaviour independent of those.

A useful test: a crate justifies its existence when at least one consumer can use it without pulling its dependencies, or when its own test surface in isolation is meaningful. `approvals` fails both. It should be a module — most naturally under `authorization` or `run_state`.

The same critique applies, in lighter form, to `events` (388 src) — fine as a module under whatever owns durable persistence.

### 4.4 `Obligation` is in the docs but not in the code

`docs/reborn/2026-04-24-host-api-invariants-and-authorization.md` makes obligations central to the authorization model: an `Allow` decision carries follow-up requirements (`ReserveResources`, `RedactOutput`, `ApplyNetworkPolicy`, `EnforceOutputLimit`). The host coordinates the obligations; nothing executes until each is satisfied.

The `Obligation` type is not implemented. Today's code performs those follow-ups inline rather than as obligations attached to a returned decision. This means:

- The "one coherent authorization law" claim from the host-api invariants doc is currently aspirational.
- A new gate that needs to introduce a new follow-up requirement has no extension point.
- The pipeline order (`audit-before → execute → audit-after / reconcile / release`) is implicit, not data-driven.

**Recommendation**: either implement `Obligation` end-to-end (decision returns obligations, `CapabilityHost` enforces them in order), or remove it from the design doc. Don't ship an unimplemented contract in a public docs surface — readers will assume it's present and design around it.

### 4.5 `host_runtime` is the kernel — the docs need to reflect this

`crates/ironclaw_host_runtime/src/lib.rs` is 237 lines. It depends on 13 of the 15 reborn crates. Its sole purpose is composition wiring. That is a precise match for what the original docs said `ironclaw_kernel` should be: *"kernel composes the system... composition-heavy and logic-light."*

Either:

- Rename `host_runtime` → `kernel` to match the design vocabulary the team has been using, or
- Update the design doc to use `host_runtime` consistently and explain the rename

Today there is a phantom `ironclaw_kernel` in the docs that doesn't exist in the code, and a real `ironclaw_host_runtime` that the docs don't mention. New contributors trying to find the kernel will get confused.

### 4.6 `auth` and `network` are still missing — and they are load-bearing

The host-api invariants doc has whole sections on:

- `Action::UseSecret(handle, mode)` and `SecretLease`
- Network mediation, allowlists, SSRF rejection, private-IP denial
- Credential injection at the host boundary (one-shot, redacted in audit)

`Action::UseSecret` and `Action::Network` exist as types in `host_api`. No system service implements either. The filesystem-backed example doesn't exercise either, so the gap is invisible if you only run the example.

This is an unimplemented contract on a public API. The right move is to either:

- Mark these `Action` variants as `#[doc(hidden)]` or feature-gated until the services exist, or
- Stand up minimal `ironclaw_auth` and `ironclaw_network` crates with the in-memory test impls before the next vertical slice (e.g. an MCP-backed capability that needs an OAuth token)

Whichever path is chosen, the design doc should be updated to reflect the timeline.

### 4.7 `ironclaw_filesystem` only has `LocalFilesystem`

The OS architecture's load-bearing claim is that the filesystem is the universal persistence surface — durable state, threads, mounted databases, mounted remote stores. Today there is exactly one backend.

The risk: the `RootFilesystem` trait may turn out to be poorly shaped for non-local backends (synchronous-style read/write/list/stat, no batching, no transaction semantics, no pagination on `list_dir`), and we won't find out until someone implements a Postgres or S3 mount.

**Recommendation**: stand up a trivial second backend (sqlite-backed, even a stub) **now**, while the trait is still malleable. The cost of changing the trait shape multiplies with every consumer.

---

## 5. Invariant discipline — declared yes, enforced no

The `host-api-invariants-and-authorization.md` doc is one of the strongest design assets in the project. Every invariant that matters is named in prose. The question is whether they are mechanically enforced in the code, and the honest answer is "almost nowhere."

### 5.1 Where invariants are declared (well)

1. **Crate-level rustdoc.** Every `lib.rs` carries a 3-5 line responsibility statement naming what the crate owns and what it does not. `ironclaw_resources`: *"No costed or quota-limited work should execute without an active reservation."* `ironclaw_authorization`: *"Evaluates authority-bearing host API contracts. It does not execute capabilities, reserve resources, prompt users, or reach into runtime internals."* Consistent across all 15 crates.

2. **Contract docs in `docs/reborn/contracts/`.** All 15 crates have a contract doc, totaling ~4,450 lines. Some name invariants explicitly (e.g. `resources.md` §2 "Core invariant"), most embed them in narrative prose.

3. **Invariant-named tests.** This is the strongest signal. Tests like `scoped_path_rejects_raw_host_paths_urls_and_traversal`, `concurrent_reservations_cannot_oversubscribe_scope`, `mount_view_denies_unknown_alias_and_broader_child_permissions` read as claims about behaviour, not exercises of code.

### 5.2 Where invariants are not enforced

1. **No mechanical dependency-direction enforcement.** No `cargo deny bans` rule beyond licenses. No architecture-test crate (the kind that walks `cargo metadata` and asserts forbidden edges). No pre-commit check for reborn crate boundaries. The architecture-docs branch explicitly promised: *"add checks that prevent forbidden imports, for example: `ironclaw_filesystem` cannot depend on product extensions."* That promise has not shipped.

2. **No method-level rustdoc on public traits.** `CapabilityDispatchAuthorizer::authorize_dispatch`, `ResourceGovernor::reserve`, `RootFilesystem::read_file` — none carry doc comments stating their invariants. A developer hovering over the trait in an IDE sees the signature and nothing else. Trait implementers must read external contract docs to know that grants must be exact-match, that reserve must precede execute, that `ScopedPath` traversal is rejected at construction.

3. **No "owns / does not own / forbidden deps" template on contract docs.** The architecture docs called for that exact structure. The contract docs describe boundaries narratively but don't follow a template a reviewer can grep for. Forbidden dependencies are not enumerated per crate.

### 5.3 The gap that matters

The existing pre-commit hook (`scripts/pre-commit-safety.sh`) already enforces architectural rules elsewhere — ToolDispatcher routing (rule #7), SSE projection (rule #9), `CredentialName` containment in web code (rule #8). The team **knows how to do mechanical enforcement** and applies it to legacy code. They just haven't applied it to the reborn stack.

This means today's discipline rests on the authors. It will hold while the original five people are writing every PR. It will start to drift the first time a less-context-loaded contributor adds a dep without realizing it crosses a layer.

---

## 6. Tooling stack for invariant verification

This is the prioritized list of tools to invest in, ordered by leverage (highest first).

### 6.1 Cheap and high-leverage — do these first

1. **`tests/architecture.rs`** — a small workspace test that uses `cargo metadata` to walk the dependency graph and assert forbidden edges. Catches the case where a new crate is added without a deny rule, and catches accidental cross-layer imports. ~30 lines of code. Half a day of work.

2. **`cargo deny` `bans`** — TOML rules per kernel crate: `host_api` cannot import workspace ironclaw deps; `filesystem` cannot import `extensions` / `wasm` / `mcp` / `scripts`. Five minutes of TOML, runs on every PR. Complements the architecture test.

3. **`cargo public-api`** snapshots committed to the repo for `host_api` and the kernel crates. Any PR diverging from the snapshot requires explicit reviewer approval. Catches the "kernel public surface quietly grew" problem before it ships.

4. **`proptest`** on validators — paths, IDs, grant attenuation. Symbolic input space exploration in the same test framework you already use, no new toolchain. The existing test `scoped_path_rejects_raw_host_paths_urls_and_traversal` becomes a property over arbitrary strings, not a fixed list. Same-week effort.

5. **Per-trait method rustdoc** stating invariants. One line per public method. *"Returns Deny if no active grant matches the capability for this principal."* IDE-visible, refactor-resistant. Half a day for the kernel traits.

6. **CODEOWNERS** with two-approver requirement on `crates/ironclaw_host_api/**`, `crates/ironclaw_authorization/**`, `crates/ironclaw_resources/**`. Minutes to configure.

### 6.2 Mid-cost — do these in the next month

7. **Sealed traits** on every kernel-public trait. `CapabilityDispatchAuthorizer`, `ResourceGovernor`, `RootFilesystem` should use the sealed-trait pattern so external crates cannot implement them. Implementations come through host-controlled constructors. AI-using contributors cannot impl what they cannot impl.

8. **`loom`** on `ResourceGovernor` reservation accounting — the `concurrent_reservations_cannot_oversubscribe_scope` invariant is begging for a concurrency model checker, not just a `tokio::test`. Loom interleaves all possible thread orderings within bound and proves the absence of races.

9. **`cargo-mutants`** on kernel crates with a hard floor — if your test suite passes when the implementation is mutated, the suite is lying to you. Especially important for AI-authored tests, which tend to test the implementation rather than the contract.

10. **A `.github/workflows/kernel-guard.yml`** that detects PRs touching `crates/ironclaw_host_api/**` or `crates/ironclaw_authorization/**` or `crates/ironclaw_resources/**` and posts a comment listing the invariants the change must preserve, plus the test files that should be updated. Both AI reviewers and human reviewers see it.

### 6.3 Higher-cost — do these only if the cheaper layers prove insufficient

11. **Kani** on the 5–10 highest-value pure invariants in `host_api`: path validation, grant attenuation (child ⊆ parent ∩ policy), ledger arithmetic, ID validators. Bounded but exhaustive proofs. Adds CI minutes and a toolchain dependency. The case for Kani over proptest is when you want a proof rather than a high-confidence sample — usually the right escalation **after** proptest catches the easy stuff.

12. **Custom `dylint` lints** for project-specific rules — "no `tokio::spawn` in kernel crates," "no direct `std::fs` outside `ironclaw_filesystem` backends," "no `.unwrap()` on `Result` in `ironclaw_authorization` (already covered by clippy in spirit, but a dylint can encode the *why*)."

### 6.4 The right wrong tool

**`Kani` first** is the wrong move. The cost-benefit is heavily front-loaded against you: harness-writing time, toolchain CI image, slow proofs, team education, all before you've validated the cheaper alternatives would have caught the same bugs. Property tests give you 70% of the value for 5% of the cost. Architecture tests give you the dependency-direction guarantee Kani cannot.

The ordering matters: cheap mechanical checks first, then concurrency model checking on the few places that need it, then formal proofs on the small set of pure invariants where bounded exhaustive coverage genuinely beats high-confidence sampling.

---

## 7. Defense strategy — hundreds of contributors, AI-tool-assisted

The stated context: hundreds of contributors, many using AI tools, expectation of zero kernel breakage.

**"Zero risk" is not achievable, and chasing it produces theater.** What is achievable: kernel breakage *mechanically impossible* in 95% of cases, *visible at review time* in the remaining 5%, and *small-radius* when it slips through.

The strategy is a stacked defense. Each layer catches what the layer below missed. The human reviewer at the top sees only what survived everything else.

### Layer 0 — Make the wrong thing impossible to express

Encode invariants in the type system, not in tests.

- `ScopedPath` vs `VirtualPath` vs `HostPath` — already done. Push it harder.
- **Sealed traits** on every kernel-public trait — external crates cannot impl, must come through a host-controlled constructor.
- **`pub(crate)` aggressively** for internals; `cargo public-api` snapshots gate any expansion.
- **Marker types for trust** — `Trusted<T>` / `Sandboxed<T>` newtypes around values that flow through the dispatch path. Compiler rejects passing `Sandboxed` into a `Trusted`-only API.
- **No `From<&str>` / `Deref<Target = str>`** on validated newtypes (this rule already exists in `.claude/rules/types.md`; extend it to every kernel ID).

This is the cheapest layer. The compiler is the only enforcement that scales to hundreds of contributors with zero process overhead.

### Layer 1 — Make the wrong thing fail to build

- `cargo deny bans` rules per kernel crate (forbidden imports).
- `tests/architecture.rs` walking `cargo metadata`.
- Workspace `[lints]` forcing `unsafe_code = "forbid"`, `clippy::unwrap_used = "deny"`, `clippy::expect_used = "deny"` on kernel crates. The repo bans these in prose; have the compiler ban them too.
- Custom `dylint` lints for project rules — higher cost but catches what grep-based pre-commit hooks miss.

### Layer 2 — Make the wrong thing fail tests

- `proptest` on validators (paths, IDs, grant attenuation) — high signal, low effort.
- `loom` on `ResourceGovernor` reservation accounting — concurrency invariants need concurrency model checkers.
- `Kani` on the 5–10 highest-value pure invariants in `host_api` — bounded but exhaustive proofs.
- `cargo-mutants` on kernel crates with a hard floor — mutation testing exposes tests that pass without verifying anything.
- Coverage floors on kernel crates (e.g. 90%+) — but coverage is a weak metric on its own; pair it with mutation testing.

### Layer 3 — Make the wrong thing visible at review

- `CODEOWNERS` with two-approver requirement on kernel paths.
- A `.github/workflows/kernel-guard.yml` that detects kernel-touching PRs and posts a comment listing invariants to preserve and tests to update. AI and human reviewers both see it.
- A required PR template for kernel changes with explicit "what invariant am I changing" and "what test asserts the new invariant" fields. PRs without these are auto-flagged by a bot until filled.

### Layer 4 — Make AI tools complicit, not adversarial

AI tools follow rules they can see. They violate rules they can't.

- **Per-crate `CLAUDE.md`** in each kernel crate listing the invariants — not the design narrative. Short, imperative, scannable. The AI's context window is finite; tell it exactly what not to break.
- **Per-trait rustdoc invariants** as called out in section 5. The AI reads the rustdoc when it generates code that uses the trait.
- **`.claude/rules/kernel.md`** with a "before touching kernel crates, do X" checklist. The repo already uses this pattern for tools, gateway-events, and types; add a kernel rule.
- **Skill files** (`.claude/skills/touch-kernel.md`) for the meta-task "you are about to modify kernel code; here is the discipline." Triggered when the AI sees the relevant paths.
- **A pre-commit check** that flags PRs that *delete* assertions in kernel-crate tests without an explanatory comment. AI tools "fix" failing tests by weakening them; this catches that pattern specifically.

### Layer 5 — The structural premise that makes everything else possible

**Keep the kernel small.** This is the only layer that scales.

The architecture-docs branch said it: agent loop, gateway, TUI, missions, all product behavior — extensions, not kernel. If 99% of contributor work happens in `extensions/*`, then the kernel-touching surface is small enough that you can afford expensive discipline (two reviewers, mutation testing, formal proofs) on it. The hundreds of contributors don't experience the friction because they're not in the kernel; the few who are see proportionally heavier process.

If the kernel grows — if `dispatcher` keeps absorbing concerns, if `capabilities` becomes the new blob — then no amount of tooling saves you, because the kernel-touching population grows with it.

The most important defense isn't a tool. It's enforcing the *"does not own"* sections of the architecture docs ruthlessly, and rejecting any PR that quietly adds responsibility to a kernel crate. That requires either a maintainer who pushes back, or a mechanical check that detects scope creep (e.g. a `cargo public-api` snapshot diff that triggers extra review when kernel public surface grows).

---

## 8. Concrete next steps

### One week of work — the highest-leverage moves

1. **`cargo public-api` snapshot** of `host_api` and the kernel crates committed to the repo. Any PR that diverges from the snapshot requires explicit reviewer approval. *(Half a day.)*

2. **`cargo deny bans` for forbidden imports** + a small `tests/architecture.rs` walking `cargo metadata`. *(Half a day.)*

3. **Sealed traits** on `CapabilityDispatchAuthorizer`, `ResourceGovernor`, `RootFilesystem`. *(Few hours.)*

4. **CODEOWNERS** with a two-approver requirement on kernel paths. *(Minutes.)*

5. **Per-crate `CLAUDE.md`** files in each kernel crate, listing invariants imperatively. *(Half a day.)*

6. **Per-trait method rustdoc** on the kernel traits, one-line invariant statement per public method. *(Half a day.)*

7. **Decide and document**: rename `host_runtime` → `kernel` or update the docs to use `host_runtime` consistently. *(One hour for the rename, two hours for docs alignment.)*

That is approximately one week of work. It eliminates the bulk of "AI accidentally crosses a layer" failure modes.

### One month of work — the second tier

8. **`proptest`** harnesses for path validation, ID validators, grant attenuation. *(One day.)*

9. **`loom`** harness for `ResourceGovernor` concurrent reservation. *(One day.)*

10. **Stand up a second `RootFilesystem` backend** (sqlite-backed) to validate the trait shape against a non-local store. *(Two-three days.)*

11. **Implement `Obligation`** end-to-end, or remove it from the design doc. *(Three-four days for a real impl, one hour to remove.)*

12. **Stand up minimal `ironclaw_auth` and `ironclaw_network` crates** with in-memory stubs and the trait shape locked down. *(Three-four days each.)*

13. **Refactor decision**: merge `dispatcher` into `capabilities`, or extract `DispatchProcessExecutor` out of `capabilities`. *(Two days for the refactor; one hour for the architectural decision before that.)*

14. **`cargo-mutants` on kernel crates** with a hard floor in CI. *(Half a day to set up; ongoing maintenance.)*

15. **`.github/workflows/kernel-guard.yml`** posting an invariants reminder on kernel-touching PRs. *(Half a day.)*

### Quarter of work — the long tail

16. **`Kani` proofs** for the 5–10 highest-value pure invariants in `host_api`. *(One-two weeks of harness work, ongoing maintenance.)*

17. **Custom `dylint` lints** for project-specific rules. *(Two-three days.)*

18. **The `kernel` rename or doc realignment** if not done in the first week.

The strategic shape: kernel breakage becomes a process problem, not a technical problem, because the technical problem has been compressed to a surface small enough that process scales.

---

## Appendix A — Crate inventory

15 crates currently exist under `crates/`. The original architecture docs proposed 11.

### Original 11 — implemented

| Crate | Source LoC | Test LoC | Notes |
|---|---:|---:|---|
| `ironclaw_host_api` | 1,343 | 344 | Clean. Zero ironclaw deps. Authority types live here. |
| `ironclaw_filesystem` | 430 | 523 | Trait + `LocalFilesystem`. No DB or remote backend yet. |
| `ironclaw_resources` | 652 | 729 | Trait + `InMemoryResourceGovernor`. Tenant scope present. |
| `ironclaw_extensions` | 547 | 500 | Manifest + discovery. Depends on filesystem (correct). |
| `ironclaw_wasm` | 1,646 | 2,381 | wasmtime-backed. Test ratio justified by import categories. |
| `ironclaw_mcp` | 326 | 322 | Adapter trait. No production client impl in this crate. |
| `ironclaw_scripts` | 407 | 343 | Docker backend + in-process test backend. |
| `ironclaw_processes` | 1,708 | 1,834 | Process record/result/cancellation/lifecycle. Justified size. |

### Original 11 — not yet implemented

| Crate | Status |
|---|---|
| `ironclaw_kernel` | Functionally present as `ironclaw_host_runtime`; rename or doc update needed. |
| `ironclaw_auth` | Missing. `Action::UseSecret` exists in `host_api` with no service backing it. |
| `ironclaw_network` | Missing. `Action::Network` exists in `host_api` with no service backing it. |

### Decomposed beyond the original — added during implementation

| Crate | Source LoC | Test LoC | Assessment |
|---|---:|---:|---|
| `ironclaw_authorization` | 459 | 757 | Justified. Grant evaluation is its own concern. |
| `ironclaw_dispatcher` | 549 | 1,343 | Borderline. Test:src ratio 2.5× suggests orchestration. Knows about all runtime lanes (OCP violation). |
| `ironclaw_capabilities` | 798 | 2,179 | Borderline. Test:src ratio 2.7×. Owns both caller facade and process-spawn bridge. |
| `ironclaw_approvals` | 114 | 252 | Should be a module under `authorization` or `run_state`, not a crate. |
| `ironclaw_run_state` | 764 | 524 | Justified. Durable state for runs and approvals. |
| `ironclaw_events` | 388 | 128 | Could plausibly be a module under `host_runtime`. Test count is low. |
| `ironclaw_host_runtime` | 237 | 344 | The actual kernel. 237 lines is the right size for composition-only. |

Total source LoC across the reborn crates: ~10,000. Total test LoC: ~12,500.

---

## Appendix B — What this review intentionally does not cover

- **Engine v2 status, gateway state convergence, sandbox hardening** — these are tracked separately in the existing repo and not part of the reborn stack.
- **Migration plan from current IronClaw to Reborn** — covered in `2026-04-24-existing-code-reuse-map.md`. The reuse map is sound; this review does not revisit it.
- **Performance characteristics** — outside the scope of an architecture review. The vertical slice runs in milliseconds; whether it scales to thousands of concurrent dispatches is a different study.
- **WASM ABI design specifics** — `ironclaw_wasm` is the largest crate by source and deserves its own review focused on the host import surface, fuel/epoch interruption tuning, and capability schema design. This review treats it as a black box trait-impl.

---

## Appendix C — Verification

Findings in this review were verified against the repository at the tip of `reborn-host-runtime-filesystem-example` as of 2026-04-26. The filesystem-backed example was built and run to confirm end-to-end behaviour. Crate sizes were measured with `find ... | xargs wc -l`. Dependency edges were read from `Cargo.toml` files. Test names were collected from `crates/<name>/tests/` and grep over `mod tests` blocks.

If any specific finding has been addressed since 2026-04-26 (e.g. `Obligation` is now implemented, or `ironclaw_kernel` has been added), this review is out of date on that point and the codebase is the source of truth.
