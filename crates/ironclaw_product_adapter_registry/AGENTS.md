# ironclaw_product_adapter_registry Agent Notes

- This crate owns ProductAdapter host-api section projection for IronClaw Reborn. Generic extension manifests, installation state, activation state, health, and credential bindings live in `ironclaw_extensions`.
- ProductAdapter declarations live in the single Extension Manifest v2, not a separate adapter manifest.
- Read `CLAUDE.md` for the full guardrail set before changing behavior.
- Do not load WASM components, perform HTTP egress, route webhooks, or read raw secret material from this crate.
- Do not add an env-var adapter declaration path. `ironclaw_extensions` installation state is authoritative.
- Credential bindings are generic extension installation state; this crate only validates ProductAdapter credential handles during projection.
- When manifests/installations are surfaced as ProductAdapter runtime entries, re-validate bindings against the current ProductAdapter sections before returning them.
- ProductAdapter host-api section projection rejects unknown TOML fields, inline secret material, and undeclared egress credential handles. Keep those invariants and add caller-level tests when changing them.
- Validation runs:
  - `cargo test -p ironclaw_product_adapter_registry`
  - `cargo clippy -p ironclaw_product_adapter_registry --all-targets -- -D warnings`
  - `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
