# ironclaw_product_contracts

The product tier's neutral contract: the `ProductSurface` membrane every
transport calls through, the wire DTOs that cross it, and the product-side
ports whose implementations sit beside or below product. It converts three
review-enforced disciplines into Cargo facts — WebUI's "DTOs/descriptors only"
rule, operator's inverted contract ownership, and the extension host's port
implementations — so a transport or collaborator compiles against contracts
instead of the 50k+-line `ironclaw_assistant`.

- **Family / layer:** `crates/contracts/` / `contracts` · **Package:** `ironclaw_product_contracts` · **Manifest:** `crates/contracts/ironclaw_product_contracts/Cargo.toml`
- **Use this when:** a transport, channel package, operator surface, extension
  host/manager, or composition must speak the product boundary — invoke/query/
  stream, wire DTOs, or a product-side port.
- **Don't use this when:** you need the `ProductSurface` *implementation*,
  admission/delivery/handler workflow, or the frozen inventory of concrete
  command/view/capability constants (→ `ironclaw_assistant`); channel-adapter
  vocabulary (→ `ironclaw_extension_contracts`); HTTP (→ the transport
  crates).

## Public surface

31 shipped modules plus gated `test_support` (`src/lib.rs` is the source of
truth; the module-by-module table lives in [`AGENTS.md`](./AGENTS.md)). The
load-bearing clusters:

- **The membrane:** `surface` — `ProductSurface` (`invoke`/`query`/
  `stream_events`, frozen), `BoundProductSurface`, `ProductSurfaceCaller`,
  `ChannelInboundProductSurface`, the `ProductSurfaceError` family.
- **Operation shapes:** `descriptors` — `ProductSurfaceCommandDescriptor`,
  `ProductCapabilityDescriptor`, `ProductView` (the *types*; product keeps the
  concrete constants — that split is why `webui → product` survives by
  design).
- **Wire DTOs:** `inbound`, `inbound_requests`, `outbound`, `product_wire`
  (the `Reborn*` family), `projection`, `package_lifecycle`,
  `workspace_views`, `admin_users`, `operator_tools`, `views`.
- **Product-side ports, implemented elsewhere:** delivery (`delivery`),
  shared-conversation admission (`shared_admission`), admission context
  (`command`), account setup
  (`account_setup`), channel config (`channel_config`), prompt sources
  (`prompt_source`), lifecycle (`lifecycle_service`), operator control plane
  (`operator_llm`, `operator_service`, `operator_secrets`), IronHub
  (`ironhub`), and `project_service` — the family's first port implemented
  *below* product (`ironclaw_identity::projects::service`).
- **The boundary error:** `error::ProductOperationFailure` — six variants,
  plain payloads, projected onto `ProductSurfaceError` exactly once here.

## Depends on / consumed by

- **Internal deps:** `ironclaw_host_api` + `ironclaw_extension_contracts`
  (the one-way street granted for channel-facing DTO reuse) — enforced as an
  allowlist (`product_contracts_allowed`). Documented external carve-outs:
  `secrecy` (secret-bearing wire fields must not be plain `String`s) and
  `tokio` with `sync` only (the two continuation handles a transport holds).
- **Consumed by 14 workspace manifests** (reproduce:
  `grep -rl '^ironclaw_product_contracts = ' --include=Cargo.toml crates Cargo.toml | wc -l`)
  — webui, openai_compat, operator, extension host/manager, identity,
  composition, product, and others.

## Invariants

- **Never product, operator, the extension host, or any transport** as a
  dependency — allowlist plus a `BoundaryRule` naming the most damaging edges.
- **Ports declared here are implemented by exactly the crate that owns the
  behavior** — product, operator, extension host, extension manager,
  composition, identity — never by this crate, never by two.
  `reborn_extension_host_port_inversion.rs` pins implementors and holds the
  product-declared residue shrink-only (do not add a residue row — narrow the
  signature or move the type); `reborn_operator_port_inversion.rs` proves the
  operator residue is zero.
- **The descriptor/inventory split is pinned both directions** by
  `reborn_transport_product_boundary.rs`; the `ProductSurface` method set by
  `reborn_service_method_freeze_ratchet.rs`.
- **No kernel-typed error variants** in `error.rs` — the port-inversion scan
  fails on any mention of `TurnError`/`ironclaw_turns`/domain crates there.
- **Vendor neutrality with one bounded exception:** `operator_llm` may carry
  exactly the censused 6 DTOs / 3 methods / 2 vendors
  (`reborn_contracts_vendor_census.rs`); a fourth provider login arrives
  behind a neutral shape or as a package.
- **One import path for the ports** —
  `reborn_product_contract_location_scan.rs`.
- **Size ceiling** — `reborn_contracts_crates_carry_a_checked_size_ceiling`;
  `product_wire.rs` additionally carries an `arch-exempt: large_file` with
  owner #7008.

## Tests

```bash
cargo test -p ironclaw_product_contracts
cargo test -p ironclaw_architecture_tests
```

## See also

- Working rules, full module table, port/implementor map, and rulings:
  [`AGENTS.md`](./AGENTS.md) (canonical crate guidance; `CLAUDE.md` points
  here).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.1.3;
  `docs/reborn/target-architecture/families/contracts.md`.
