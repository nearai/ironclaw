# ironclaw_extension_contracts

The neutral vocabulary of what an installable extension *is and exposes* —
adapter traits, manifest surfaces, auth recipes, lifecycle states, runtime
descriptors, and the sealed verified-inbound evidence — shared by lanes, hosts,
packages, and product without any of them importing the extension registry or
an owner. It is the single contract that keeps "installable package" a
four-way-separated responsibility (package / registry / host / product)
instead of a tangle: without it, either lanes depend on the registry crate or
packages depend on product.

- **Family / layer:** `crates/contracts/` / `contracts` · **Package:** `ironclaw_extension_contracts` · **Manifest:** `crates/contracts/ironclaw_extension_contracts/Cargo.toml`
- **Use this when:** you implement or consume an extension surface — a channel
  adapter, a tool adapter, a manifest descriptor, an auth recipe, an
  installation state — generically.
- **Don't use this when:** you need the registry or installation *stores* (→
  `ironclaw_extension_registry`), lifecycle execution/binding/ingress routing
  (→ `ironclaw_extension_host`), product lifecycle projections (→
  `ironclaw_product_contracts::package_lifecycle`), or anything vendor-named
  (→ the package under `crates/extensions/packages/`).

## Public surface

18 shipped modules (`src/lib.rs` is the source of truth; the module-by-module
charter table lives in [`AGENTS.md`](./AGENTS.md)). The load-bearing entries:

- `channel_adapter` — `ChannelAdapter` (implemented once per channel package)
  + its DTO family (`NormalizedInboundMessage`, `OutboundEnvelope`/`Part`,
  `DeliveryReport`, `TargetQuery`/`Candidate`, `ChannelError`,
  `ChannelAttachmentRef`, `ProductTriggerReason`).
- `tool_adapter` — `ToolAdapter` + `RestrictedEgress`; `egress` — channel
  egress transport vocabulary (`ProtocolHttpEgress`, `OutboundDeliverySink`).
- `extension` — `Extension`, `ExtensionEntrypoint` and bindings; `runtime` —
  `ExtensionRuntime` + validated `ExtensionAssetPath` (resolution *under* a
  package root stays in the registry as
  `ironclaw_extension_registry::resolve_asset_under`, an orphan-rule cost).
- `channel` / `recipe` / `memory` / `product_adapter_section` — the
  declarative manifest surfaces a manifest compiles into (schema and
  cross-field invariants here; manifest *parsing* in the registry).
- `state` — `InstallationState`, `LifecyclePublicState` (three-state public
  lifecycle; see `.claude` rule "Extensions wire public state").
  `AuthAccountState` is deliberately **not** here — it stays beside its engine
  in `ironclaw_auth`.
- `preference_target` — `PreferenceTargetCodec`, the one vendor-implemented
  port here; `channel_identity` — the identity hooks a host runs around
  binding.
- `verified_inbound` — the **sealed** channel/webhook mint family
  (`mark_request_signature_verified`, `mark_shared_secret_header_verified`);
  every entry point consumes a `VerifiedInboundGrant`, obtainable only by the
  generic ingress verifier.
- `hosted_mcp` + `lifecycle_id`; `auth_prompt`; `external`; `test_support`
  (feature-gated conformance suite + fakes).

## Depends on / consumed by

- **Internal deps:** `ironclaw_host_api` and nothing else — enforced as an
  allowlist (`extension_contracts_allowed`). No framework/driver/`tokio`.
- **Consumed by 20 workspace manifests** (reproduce:
  `grep -rl '^ironclaw_extension_contracts = ' --include=Cargo.toml crates Cargo.toml | wc -l`)
  — lanes (`mcp`, `sandbox`, `wasm`), the extension tier, channel packages,
  product, the two sibling port crates, and composition.

## Invariants

- **Traits here are open by design** — sealing one would forbid the
  extensibility the unified extension model is built on; a trait that needs a
  closed impl set belongs in an owner crate. The one closed thing is a
  *constructor* (the `verified_inbound` mint family), sealed by witness grant
  and pinned by `reborn_sealed_evidence_mint_ratchet.rs`.
- **One definition, one import path** —
  `reborn_extension_contract_location_scan.rs`; read its module doc before
  adding any `pub use` that names a type from here.
- **Allowlist + framework deny + size ceiling** —
  `reborn_crate_dependency_boundaries_hold`,
  `reborn_contracts_crates_hold_no_framework_dependencies`,
  `reborn_contracts_crates_carry_a_checked_size_ceiling`.
- **No vendor names** — `reborn_extension_specificity.rs` polices this crate
  like any other.

## Tests

```bash
cargo test -p ironclaw_extension_contracts
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules, module charter table, and placement history:
  [`AGENTS.md`](./AGENTS.md) (canonical crate guidance; `CLAUDE.md` points
  here).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.1.2;
  `docs/reborn/target-architecture/families/contracts.md`; the
  `reborn-extension-surfaces` skill for the end-to-end integration flow.
