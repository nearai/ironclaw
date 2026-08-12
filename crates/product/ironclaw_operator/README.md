# ironclaw_operator

The deployment-operator control plane: LLM provider administration (registry
write side, operator-scoped keys, active provider+model selection, catalog
overlay, live reload, NEAR AI / OpenAI Codex logins), the operator log ring,
and OS service lifecycle. A different kind of "operator" than an installed
extension's own management surface — this is the person running the
deployment, not a channel. It implements ports declared in
`ironclaw_product_contracts` rather than depending on `ironclaw_assistant`,
un-inverting what would otherwise be an upward dependency from a control plane
into the conversational core.

- **Family / layer:** `product` / `products` · **Package:** `ironclaw_operator` · **Manifest:** `crates/product/ironclaw_operator/Cargo.toml`
- **Use this when:** changing LLM provider administration, operator log
  capture, or host service lifecycle.
- **Don't use this when:** changing conversation or channel behavior →
  `ironclaw_assistant`; extension lifecycle → `ironclaw_extension_host` /
  `ironclaw_extension_manager`; declaring a port a product-tier caller must
  name → `ironclaw_product_contracts` (declaring it here re-inverts the edge).

## Public surface

Implementations of the operator-service ports declared in
`ironclaw_product_contracts`:

| Port (contracts module) | Implemented here by |
| --- | --- |
| `LlmConfigService` (`operator_llm`) | `llm_admin::llm_config_service::RebornLlmConfigService` |
| `ModelSelectionPolicyStore` (`operator_llm`) | `llm_admin::model_selection_policy_store::FilesystemModelSelectionPolicyStore` |
| `ActiveModelReader` (`operator_llm`) | `llm_admin::active_model::ProviderActiveModelReader` |
| `OperatorLogsService` (`operator_service`) | `operator_logs::OperatorLogBuffer` |
| `OperatorServiceLifecycleService` (`operator_service`) | `operator_service_lifecycle::OperatorServiceLifecycle` |

(The fifth port in `operator_service`, `OperatorStatusService`, is implemented
by `ironclaw_composition` — readiness is the one answer only the assembly root
can compute.) Plus `nearai_login_callback_mount`, the crate's one HTTP route,
handed back as an `ironclaw_host_ingress::PublicRouteMount`.

## Depends on / consumed by

- **Normal workspace deps (8):** `ironclaw_product_contracts` (the ports),
  `ironclaw_llm` (provider mechanics — this *is* the LLM-vendor admin layer),
  `ironclaw_host_ingress` (route carrier), `ironclaw_host_api`,
  `ironclaw_common`, `ironclaw_filesystem`, `ironclaw_safety` — and
  `ironclaw_config`, a measured divergence from the family target ("boot
  values arrive as construction input"), recorded in `families/product.md`'s
  §6.9.2 correction and not forbidden by the BoundaryRule today.
- **Consumed by (2):** `ironclaw_composition` and the `ironclaw` binary.
- **No `ironclaw_secrets`:** key material goes through
  `ironclaw_product_contracts::operator_secrets::OperatorSecretValueStore`,
  implemented by `ironclaw_composition` (WS3 tightening; the BoundaryRule
  forbids the substrate under every dependency kind).

## Invariants

- **Implements ports, never declares them; never depends on
  `ironclaw_assistant`** — enforced twice:
  `reborn_operator_port_inversion.rs` (residue frozen at zero, manifest
  proved clean through `cargo metadata`) and the `ironclaw_operator`
  `BoundaryRule` in `reborn_dependency_boundaries.rs`.
- **Owns its one route, never where it mounts** — it hands back a carrier; no
  local mount types (the deleted `OperatorPublicRouteMount` duplicate is what
  once forced a composition-side shim).
- **Vendor names are licensed here** (PROPOSAL §8.2) — `nearai`, `codex`,
  `openai`, `anthropic` are this crate's subject matter; but the *ports* in
  contracts are bounded to the existing six vendor-named DTOs / three methods
  (`reborn_contracts_vendor_census.rs`) — a new vendor login arrives behind a
  neutral shape.

## Tests

```bash
cargo test -p ironclaw_operator
cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold
```

The ports' *contract* (argument threading, wire forms) is tested in
`ironclaw_product_contracts`; what belongs here is whether this implementation
answers correctly — key masking, catalog overlay precedence, log-ring bounds,
lifecycle state mapping.

## See also

Working rules: `AGENTS.md` · family rules: `crates/product/AGENTS.md` · design
record: `docs/reborn/target-architecture/families/product.md` (§6.9.2).
