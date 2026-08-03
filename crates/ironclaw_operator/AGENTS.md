# Agent Map — ironclaw_operator

## Start Here

- Read `CLAUDE.md` in this directory for the contract; this file is the map.
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
  - `llm_admin/mod.rs` — the `llm_admin` re-export surface `lib.rs` lifts to the
    crate root.
  - `operator_logs.rs` — the operator log ring (`OperatorLogBuffer`) and its
    `tracing` layer.
  - `operator_service_lifecycle.rs` — OS-service install/start/stop/status.
  - Re-derive this list with `rg --files crates/ironclaw_operator/src`. Every
    entry above except the two `operator_*.rs` files lives under `src/llm_admin/`,
    so a non-recursive `ls` of `src/` cannot reproduce it.
- Read the contract this crate implements *against*, never the crate beside it:
  - `crates/ironclaw_product_contracts/CLAUDE.md` — `llm_config`,
    `operator_service`, `operator_llm`, `operator_tools`, `surface`.

## What This Crate Owns

- **LLM provider administration.** The registry write side, operator-scoped
  API keys, the active provider+model selection, the catalog overlay, live
  provider hot-swap, and the NEAR AI / OpenAI Codex logins. This *is* the
  LLM-vendor admin layer, and PROPOSAL §8.2's vendor rule names it as one of
  the few places a vendor name may appear in code.
- **The operator log ring.** A bounded in-memory buffer fed by a `tracing`
  layer, queried through `OperatorLogsService`.
- **OS service lifecycle.** Install/start/stop/status for the host process.

## What This Crate Does Not Own

- **Ports.** Every product-facing port this crate satisfies is *declared* in
  `ironclaw_product_contracts` and implemented here. Do not add a trait here
  that a product-tier caller must name — that is the inversion the WS5 operator
  row removed.
- **`ironclaw_product`.** There is no dependency on it, and there must not be:
  operator is product's sibling, not its consumer. Enforced twice —
  `reborn_operator_port_inversion.rs` (through `cargo metadata`) and the
  `ironclaw_operator` `BoundaryRule` in `reborn_dependency_boundaries.rs`.
- **Routers.** This crate builds one route and hands it back as an
  `ironclaw_host_ingress::PublicRouteMount` (a router plus its ingress policy
  descriptors). Composition decides where it mounts. Do not add a `Router` that
  something outside this crate is expected to nest, and do not declare a local
  mount type — the host-owned carrier is the vocabulary.
- **Assembly.** Composition wires this crate's services together; the crate
  itself constructs nothing it did not get handed.

## Do Not Move In Here

- Product workflow, admission, delivery, or bindings — `ironclaw_product`.
- Runtime/lane machinery (host runtime, MCP, wasm, scripts, turns, runner,
  loop host) — the boundary rule forbids all of them.
- Transports (`ironclaw_webui`, `ironclaw_reborn_openai_compat`) and the
  assembly root (`ironclaw_reborn_composition`).
- Extension lifecycle or catalog logic — `ironclaw_extension_host`.

## Known Debt

- **Direct `ironclaw_secrets` edge.** The key store reaches the secret store
  directly. CHECKLIST WS3 removes it behind a `product_contracts` port
  ("port replacements land first", PROPOSAL §12.1b), so the boundary rule
  deliberately does *not* forbid `ironclaw_secrets` yet.
- **`llm_admin/provider_admin.rs` `include_str!`s CLI source** (into
  `crates/ironclaw_reborn_cli/src/commands/config/init.rs`). Inventoried by
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
