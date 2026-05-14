# ironclaw_product_adapter_registry Agent Notes

- This crate owns ProductAdapter manifest + installation registry contracts for IronClaw Reborn. It is the typed analog of v1 channel manifest + `activated_channels` state.
- Read `CLAUDE.md` for the full guardrail set before changing behavior.
- Do not load WASM components, perform HTTP egress, route webhooks, or read raw secret material from this crate.
- Do not add an env-var adapter declaration path. Registry state is authoritative.
- Credential bindings store opaque `SecretHandle`s only; never raw secret material.
- When mutations cross writes (manifest replaced; activation flipped), re-validate the affected installation against the current manifest before persisting or surfacing the change.
- Manifest parsing rejects unknown TOML fields, inline secret material, and undeclared egress credential handles. Keep those invariants and add caller-level tests when changing them.
- Validation runs:
  - `cargo test -p ironclaw_product_adapter_registry`
  - `cargo clippy -p ironclaw_product_adapter_registry --all-targets -- -D warnings`
  - `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
