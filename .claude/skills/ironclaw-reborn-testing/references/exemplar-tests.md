# Exemplar Tests — What Good Looks Like at Each Tier

Living companions to the tier tree in `../SKILL.md`. Each exemplar is a real in-tree test to open and imitate; re-verify paths with the given command before citing them onward.

## Contents
- 1. Side-effect proof at the caller (the gold standard)
- 2. The scripted-model harness seam
- 3. The declare/enforce contract pair
- 4. Helper-only coverage: the cautionary shape
- 5. Silent skip vs loud skip
- 6. Naming your contract's tests

## 1. Side-effect proof at the caller

`tests/integration/group_approvals/scenario_gate_then_approve.rs` — drives a scripted `builtin.write_file` through the **real** stack: first-party runtime → `PermissionMode::Ask` → `TurnStatus::BlockedApproval` → real `ApprovalResolver::approve_dispatch` (lease issued) → `coordinator.resume_turn` → `Completed` — and then **asserts the file exists on disk**. The assertion target is the side effect itself, not a mock's call count. When your change gates a side effect, this is the shape: drive the public entry point, assert the world changed. Re-verify: `ls tests/integration/group_approvals/`.

## 2. The scripted-model harness seam

The in-process harness (code in `tests/integration/support/`, spec in `tests/integration/CLAUDE.md`) fakes exactly one thing: the vendor SDK at the bottom (`TraceLlm`). Everything else — product workflow, coordinator, scheduler, agent loop, the real `ironclaw_llm` retry/failover/circuit-breaker chain — executes for real, and assertions read *persisted state* (filesystem, thread history), never internals.

- **Right**: mock at the vendor-SDK seam; assert from durable state; `cargo test --test reborn_integration_<name>` runs offline with zero setup.
- **Wrong**: mocking at the gateway seam (skips the whole `ironclaw_llm` chain — that's the separate binary-replay tier's job); hand-building `TraceStep`s; asserting on internal structs.

## 3. The declare/enforce contract pair

Two tests lock the WebChat v2 surface from opposite sides — copy the pairing whenever policy is declared in one crate and enforced in another:

- `crates/ironclaw_webui/tests/webui_v2_descriptors_contract.rs` — locks the **declared** policy table per route (method, auth schemes, body/rate limits, CORS, audit class). Adding a route without updating it fails CI.
- `crates/ironclaw_webui/tests/webui_v2_handlers_contract.rs` — drives a **real axum router** against a stub facade, including the fail-closed case (`missing_caller_extension_returns_500`). Its header cites the test-through-the-caller rule; that's the level of intent-documentation to imitate.

The *enforcement* side (real HTTP 401/413/429/CORS through the composed app) lives in `crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs` — caller-level, not middleware unit tests.

## 4. Helper-only coverage: the cautionary shape

When a predicate selects what goes out the wire, the test must construct the real adapter and inspect the rendered egress body, not call the predicate directly. Use `crates/ironclaw_product_workflow/tests/outbound_delivery_contract.rs` and its Telegram egress case as the shape to copy. If you're adding a channel, write the caller-level adapter test on day one.

## 5. Silent skip vs loud skip

**BAD for PR-gated coverage**: `if docker_unavailable { return }` — the container security suite silently vanishes from CI and no gate notices. Existing Docker sandbox canaries still use soft skips; don't copy that pattern into new gate coverage.

**GOOD**: make absence loud — feature-gate the test (`#![cfg(all(feature = "postgres", feature = "integration"))]`, which `scripts/check-boundaries.sh` enforces for root `tests/`), or require an explicit opt-out env var and *fail* when the dependency is missing without it. A skipped security test that doesn't announce itself is indistinguishable from coverage.

## 6. Naming your contract's tests

`docs/reborn/contracts/conversation-binding.md` is the model contract doc: it names its proving test file (`crates/ironclaw_conversations/tests/inbound_contract.rs`) *and* the run command. `scripts/reborn-e2e-rust.sh` is the machine-readable contract→test map. If you implement contract behavior: extend the named test, and add the doc's "which tests prove this" line if it's missing.
