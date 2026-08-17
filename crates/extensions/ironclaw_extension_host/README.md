# ironclaw_extension_host

The **generic extension host**: the one place any extension — regardless of
vendor or runtime — is installed, verified, bound, activated, delivered
through, and removed. It exists as its own crate because this machinery carries
a real trust job (ingress verification, binding, activation) that must stay
free of any vendor or product-specific branch; the crate boundary is what makes
"no concrete vendor name here" checkable rather than conventional.

- **Family / layer:** `extensions` / `loops` · **Package:** `ironclaw_extension_host` · **Manifest:** `crates/extensions/ironclaw_extension_host/Cargo.toml`
- **Use this when:** changing how *every* extension behaves — lifecycle, loading, verification, egress, channel identity/pairing/config, hosted-MCP registration.
- **Don't use this when:** the change is vendor-shaped (→ `packages/<id>/`), management UX (→ `ironclaw_extension_manager`), a durable record or manifest-schema change (→ `ironclaw_extension_registry`), or shared vocabulary (→ `ironclaw_extension_contracts`).

## Public surface

Grouped by responsibility (all modules are `pub`; see `src/lib.rs`):

- **Lifecycle authority** — `ExtensionLifecycleManager` (`product_lifecycle.rs`;
  the sole writer of installation-state transitions: operation lock, activation
  transactions, `install_policy`, `removal_cleanup`), `lifecycle_restore` (boot
  restoration), `ExtensionHost`/`SnapshotWatch` (`lifecycle.rs`) and the
  `ActiveSnapshot`/`Generation` active-installation view (`active.rs`).
- **Loading & binding** — `ExtensionEntrypoint`/`ExtensionBindings`
  (`entrypoint.rs`): native, WASM, and MCP loaders all produce this one binding
  shape (`loaders/`, `mcp.rs`, `mcp_discovery.rs`).
- **Ingress** — the vendor-blind router + manifest-recipe verifier
  (`ingress/router.rs`, `ingress/verifier.rs`): executes each channel's declared
  verification recipe (HMAC / shared-secret) identically for every vendor and
  mints the sealed verified-inbound evidence everything downstream trusts.
  Adapters parse only; they can never construct trust.
- **Channel services (generic, per-extension)** — `GenericChannelHostAssembly`
  (`channel_host.rs`, the ingress reconciler), delivery ports
  (`run_delivery_ports.rs`, `channel_delivery.rs`), egress transports
  (`egress.rs`, `channel_egress.rs` — credentials injected by the host at send
  time), identity + actor resolution (`ProviderIdentityActorResolver`,
  `provider_identity.rs`), and the pairing / connection / config / admin-config
  service cores (`channel_pairing*`, `channel_connection.rs`,
  `channel_config.rs`, `admin_configuration_*`).
- **Catalog** — `AvailableExtensionCatalog` (`available_extensions.rs`) + import:
  read by boot-time restore and the registration pipeline, which is why it
  lives here and not in the manager.
- **Hosted-MCP registration pipeline** — `hosted_mcp_admission.rs`,
  `hosted_mcp_manifest.rs`, `hosted_mcp_preparation.rs`,
  `hosted_mcp_discovery_authority.rs`, `mcp_catalog_safety.rs`: endpoint
  admission, manifest synthesis for user-registered servers, the
  preparation/discovery lifecycle, and remote-catalog safety screening.
  Deliberately separate from the shared install→activate→remove path.

## Depends on / consumed by

Depends on the kernel crates it hosts activity for (`capabilities`,
`authorization`, `approvals`, `processes`, `resources`, `trust`, `turns`,
`host_runtime`), the registry and `extension_support`, contracts
(`host_api`, `extension_contracts`, `product_contracts`, `loop_contracts`,
`common`), domain crates (`auth`, `attachments`, `conversations`, `llm`,
`outbound`, `skills`, `threads`, `triggers`), substrates, `ironclaw_mcp`,
`ironclaw_loop_host`, `ironclaw_host_ingress`, and `ironclaw_config` — the full
list is the `Cargo.toml`. It never depends on `ironclaw_assistant` as a normal
dependency (dev-only fixture edge exists; see the gate below) and never on
`ironclaw_extension_manager` in any dependency kind.

Consumed by: `ironclaw_extension_manager`, `ironclaw_composition`,
`ironclaw_cli`, `ironclaw_webui` (one documented edge — the pairing service
core behind the WebUI's pairing routes), and the root integration-test package
(dev). Re-derive with
`rg -l 'ironclaw_extension_host = ' Cargo.toml crates/*/*/Cargo.toml`.

## Invariants

- **No concrete vendor name, protocol route, or behavior branch.** Vendor
  behavior lives in `packages/*`; a name here trips
  `reborn_extension_specificity.rs` (`cargo test -p ironclaw_architecture_tests`).
- **The host never depends on the manager** — any dependency kind, dev included:
  `reborn_extension_manager_split.rs`.
- **Product-facing ports are declared in `ironclaw_product_contracts`**, never
  against a concrete product type; the residual product-symbol list is frozen
  shrink-only by `reborn_extension_host_port_inversion.rs`.
- **Registration vocabulary stays inside `src/hosted_mcp_*`** so a hosted-MCP
  concern cannot change the resting state of a channel or WASM package:
  `reborn_registration_pipeline_boundary.rs`.
- **No `/api/webchat/` route in production files** — what remains of the Axum
  surface is exactly the vendor-blind ingress router:
  `reborn_transport_product_boundary.rs`.
- **A durable direct-message admission proves a personal reply route.** After
  identity resolution and durable user-message admission, the generic
  post-admission observer upserts that actor/conversation into the owner-scoped
  DM-target catalog. `Rejected(_)`, command, no-op, and shared-conversation
  ingress never mutate the catalog; `RejectedBusy` still backfills the proven
  route so a retry does not lose discoverability. Provider-specific code is
  not involved.

## Tests

```bash
cargo test -p ironclaw_extension_host                          # unit + contract suites
cargo test -p ironclaw_extension_host --features test-support  # incl. lifecycle_contract (feature-gated)
cargo test -p ironclaw_architecture_tests                      # the boundary gates above
```

Contract suites live in `tests/`: `lifecycle_contract.rs` (requires
`test-support`), `lifecycle_restore_contract.rs`, `ingress_router_contract.rs`,
`admin_configuration_{store,service}_contract.rs`.

## See also

`crates/extensions/AGENTS.md` (the family rules and the unified extension
model); `docs/internal/reborn/target-architecture/families/extensions.md` §"ironclaw_extension_host";
`crates/extensions/ironclaw_extension_manager/AGENTS.md` for what deliberately
stayed here and why.
