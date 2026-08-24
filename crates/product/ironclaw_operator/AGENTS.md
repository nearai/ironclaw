# Agent Map — ironclaw_operator

Working rules for the deployment-operator control plane. Orientation lives in
`README.md`; family rules in `crates/product/AGENTS.md`.

Both guidance files are new with the WS5 operator row. Before it, this crate
had **neither guidance nor a boundary rule** — the audit's clearest
correlation was guidance-presence ↔ discipline, and this crate is the worked
example: its `ironclaw_assistant` dependency survived every earlier sweep
because nothing was watching.

## Start Here

- Read `src/lib.rs` first, then:
  - `llm_admin/llm_config_service.rs` — `RebornLlmConfigService`, the
    `LlmConfigService` implementation the WebChat Inference tab drives.
  - `llm_admin/provider_admin.rs` — the typed provider/model admin surface the
    standalone CLI and the product command workflow share.
  - `llm_admin/provider_repo.rs`, `llm_admin/llm_key_store.rs`,
    `llm_admin/llm_catalog.rs`, `llm_admin/llm_reload.rs`,
    `llm_admin/active_model.rs`, `llm_admin/resolved_llm.rs` — the provider
    catalog overlay, the operator-scoped key store, the live reload handle.
  - `llm_admin/nearai_login_serve.rs` — the one Axum route this crate owns
    (the public NEAR AI login callback).
  - `llm_admin/nearai_mcp.rs` — NEAR AI MCP endpoint/bootstrap config.
  - `llm_admin/model_selection_policy_store.rs` — tenant-scoped filesystem
    persistence for the operator-owned user model policy.
  - `llm_admin/user_model_preference_store.rs` — caller-scoped filesystem
    persistence for user model preferences.
  - `llm_admin/mod.rs` — the `llm_admin` re-export surface `lib.rs` lifts to
    the crate root.
  - `operator_logs.rs` — the operator log ring (`OperatorLogBuffer`) and its
    `tracing` layer.
  - `operator_service_lifecycle.rs` — OS-service install/start/stop/status.
  - Re-derive this list with `rg --files crates/product/ironclaw_operator/src`.
    Every entry above except the two `operator_*.rs` files lives under
    `src/llm_admin/`, so a non-recursive `ls` of `src/` cannot reproduce it.
- Read the contract this crate implements *against*, never the crate beside
  it: `crates/contracts/ironclaw_product_contracts/CLAUDE.md` —
  `operator_llm`, `operator_service`, `operator_tools`, `surface`.

## The one rule

**This crate implements ports; it does not declare them, and it does not
depend on `ironclaw_assistant`.**

`ironclaw_operator` and `ironclaw_assistant` both sit in the `products`
layer. They are siblings. The layer matrix permits `products → products`, so
the dependency was *legal and invisible* — which is why it needed a
purpose-built gate rather than a matrix entry. Two gates hold it:

- `reborn_operator_port_inversion.rs` — the residue of product-declared
  traits this crate implements is **frozen at zero and shrink-only**; the
  manifest is proved clean through `cargo metadata` (not a literal path, so a
  directory move fails loudly); and each inverted port is pinned as
  declared-in-contracts / not-re-declared-in-product / implemented-by-its-owner.
- The `ironclaw_operator` `BoundaryRule` in
  `reborn_dependency_boundaries.rs`.

**Adding a port.** Declare the trait in `ironclaw_product_contracts`,
implement it here, and add a row to `INVERTED_PORTS` in the gate. Do not
declare a product-facing trait in this crate: a product-tier caller would then
have to name `ironclaw_operator`, which re-inverts the edge in the other
direction. (The port table itself is in `README.md`; grep `operator_llm` for
the DTO module — a `llm_config` module does not exist, #7018 consolidated it.)

## What This Crate Owns

- **LLM provider administration.** The registry write side, operator-scoped
  API keys, the active provider+model selection, the catalog overlay, live
  provider hot-swap, and the NEAR AI / OpenAI Codex logins. This *is* the
  LLM-vendor admin layer, and PROPOSAL §8.2's vendor rule names it as one of
  the few places a vendor name may appear in code — `nearai`, `codex`,
  `openai`, `anthropic` and friends are this crate's subject matter, not
  leakage. The corollary: the *ports* live in a contracts crate, and §8.2
  bounds the vendor vocabulary there to the existing six DTOs / three methods
  (`reborn_contracts_vendor_census.rs`) — do not add a seventh vendor name; a
  new vendor login belongs behind a neutral shape.
- **The operator log ring.** A bounded in-memory buffer fed by a `tracing`
  layer, queried through `OperatorLogsService`.
- **OS service lifecycle.** Install/start/stop/status for the host process.

## What This Crate Does Not Own

- **Ports.** Every product-facing port this crate satisfies is *declared* in
  `ironclaw_product_contracts` and implemented here.
- **Routers.** This crate builds one route and hands it back as an
  `ironclaw_host_ingress::PublicRouteMount` (a router plus its ingress policy
  descriptors). Composition decides where it mounts. Do not add a `Router`
  that something outside this crate is expected to nest, and do not declare a
  local mount type — the deleted `OperatorPublicRouteMount`/
  `OperatorProtectedRouteMount` duplicates are what once forced a
  composition-side shim whose entire body was
  `PublicRouteMount::new(mount.router, mount.descriptors)`.
- **Assembly.** Composition wires this crate's services together; the crate
  itself constructs nothing it did not get handed.

## Do Not Move In Here

- Product workflow, admission, delivery, or bindings — `ironclaw_assistant`.
- Runtime/lane machinery (host runtime, MCP, wasm, scripts, turns, runner,
  loop host) — the boundary rule forbids all of them.
- Transports (`ironclaw_webui`, `ironclaw_openai_compat`) and the
  assembly root (`ironclaw_composition`).
- Extension lifecycle or catalog logic — `ironclaw_extension_host`.

## Known Debt

- ~~**Direct `ironclaw_secrets` edge.**~~ **Discharged by CHECKLIST WS3.** The
  key store holds
  `ironclaw_product_contracts::operator_secrets::OperatorSecretValueStore`;
  `ironclaw_composition::RuntimeOperatorSecretValueStore` implements it
  over the substrate, and the boundary rule now forbids `ironclaw_secrets` here
  under every normal dependency kind. **What stays in this crate is policy** —
  the `llm_provider_<id>_api_key` handle derivation and the fail-closed
  behaviour when a store call errors. What left is every substrate concern: the
  scope (the port fixes it, so this crate cannot name one), the lease protocol
  behind `read`, and the substrate's error detail (only a stable classification
  string crosses). Two tests travelled with the behaviour rather than being
  faked here — `read_is_repeatable_across_reloads` and the #4673
  production-store reproduction now live with the port's implementor in
  `ironclaw_composition`, because a fake asserting its own repeatability
  proves nothing. Adding a secret substrate back to this crate is a boundary
  failure, not a convenience.
- **`llm_admin/provider_admin.rs` `include_str!`s CLI source** (into
  `crates/app/ironclaw_cli/src/commands/config/init.rs`). Inventoried by
  `reborn_cross_crate_include_scan.rs`, which is still report-only; CHECKLIST
  WS2 flips it to enforcing.
- **`nearai_mcp.rs` is forked.** `ironclaw_extension_host/src/nearai_mcp.rs`
  is a near-verbatim copy of the endpoint/config half. This crate's copy is the
  richer one (it also holds the env and `LlmConfig` loaders and the bootstrap
  outcome enum). The fork is deliberate: `ironclaw_operator` is a `products`
  crate, so once `extension_host` drops to `loops` an import of this copy would
  be an upward edge — the fork is what *prevents* an `extension_host → operator`
  dependency. Removal is owned by the CHECKLIST WS2 row **"Kill the cross-crate
  `include_str!` reach-ins (gmail/github/nearai-mcp manifests)"**, not by the
  strays row above it, which measured the claim and handed it over. The required
  replacement mechanism is **package inventory supplied by the binary**: the
  binary provides the endpoint configuration alongside the manifest, retiring
  the fork and the `include_str!` reach-in as one change rather than two.
- **`ironclaw_config` edge.** The family target says boot values arrive as
  construction input, not a config dependency; the edge is live in
  `operator_service_lifecycle.rs` and not forbidden by the BoundaryRule —
  recorded in `families/product.md` §6.9.2's dated correction.

## Testing

Per-crate: `cargo test -p ironclaw_operator`. The ports' *contract* (argument
threading, error projection, wire forms, object safety) is tested in
`ironclaw_product_contracts`; what belongs here is whether **this**
implementation answers correctly — key masking, catalog overlay precedence,
config-file writes, the log ring's bounds, and service-lifecycle state
mapping.

A contracts-tier test can only pin that the port hands an implementation its
arguments and that the shape admits a per-caller answer. It cannot pin that
this crate filters by caller. Do not rely on it for that.
