# ironclaw_wasm — guidance pointer

Canonical guidance for this crate lives in:

- [`CLAUDE.md`](./CLAUDE.md) — the crate's working rules, ABI-ownership rules,
  and safety rules. **Gate-pinned:**
  `wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel`
  (`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`)
  requires it to keep the `wasm_sandbox_core` domain-free wording; edit it with
  `cargo test -p ironclaw_architecture_tests` in hand.
- [`README.md`](./README.md) — orientation: what the lane is, public surface,
  measured edges, the `wit/` ownership story, tests.
- [`../AGENTS.md`](../AGENTS.md) — the `lanes/` family boundary (the lane
  contract), why `wit/` living inside this crate is load-bearing, and the
  family gates.

Contracts of record: `docs/reborn/contracts/wasm.md`,
`docs/reborn/contracts/runtime-workflows.md`,
`docs/reborn/contracts/network.md`.

Consolidated 2026-08-05 per `docs/reborn/guidance-conventions.md` rule 1 (one
canonical home per fact); this file previously duplicated the guardrails'
ownership list and the sandbox-core domain-free rule.
