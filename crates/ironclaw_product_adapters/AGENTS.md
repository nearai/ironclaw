# Agent Map — ironclaw_product_adapters

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for feature shape (`test-support`, `host-auth-mint`, etc.).
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_product_adapter_registry/AGENTS.md`
  - `crates/ironclaw_product_workflow/AGENTS.md`
  - `crates/ironclaw_wasm_product_adapters/CLAUDE.md`
  - `crates/ironclaw_outbound/AGENTS.md`

## What This Crate Owns

- ProductAdapter contracts: typed inbound/outbound DTOs, adapter trait, workflow bridge types, and fakes.
- External refs for actor/conversation/reply identity and presentation metadata.
- Adapter auth evidence contracts, declared egress, adapter capability descriptors (`ProductAdapterCapabilities`/`ProductCapabilityFlag`), projection/outbound envelope DTOs, and redaction helpers.
- Adapter-safe parse/render/health boundaries independent of concrete product workflow.

## Do Not Move In Here

- Kernel/dispatcher internals, canonical user/thread resolution, turn coordination, or workflow orchestration.
- ProductAdapter registry installation state or WASM runner implementation.
- Direct network egress outside `ProtocolHttpEgress`.
- Public constructors for sealed trusted auth evidence in production code.

## Validation

- Fast local check: `cargo test -p ironclaw_product_adapters`
- Contract checks: `cargo test -p ironclaw_product_adapters --test product_adapter_contract`
- Review-regression checks: `cargo test -p ironclaw_product_adapters --test review_findings_contract`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Inbound DTOs carry structured external refs only; no flattened string identities.
- Adapters return parsed DTOs; host/product workflow stamps trusted context and performs canonical binding.
- Delivery failures are best-effort status metadata, not transcript/run failure state.
- Keep validation/redaction tests close to DTO changes.
