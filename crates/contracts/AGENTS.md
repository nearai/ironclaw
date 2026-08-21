# `crates/contracts/` — neutral vocabulary and ports; nothing here runs, stores, or decides

**Layer(s):** `contracts` (all 6 crates) · **Crates:** 6 · **May depend on:** nothing outside this family — the three foundational crates hold zero internal deps; the three port crates hold only the intra-family edges named below · **Depended on by:** every other family (re-derive the fan-in with `grep -rl '^ironclaw_host_api = ' --include=Cargo.toml crates tests Cargo.toml | wc -l`).

## What this family is

The vocabulary tier: the one family every other family depends on, and which
depends on nothing itself. A type earns a home here by passing the **four-part
admission test** — (a) it names a concept crossing an authority, host, or
product boundary; (b) it is neutral across vendor, runtime, storage, and
deployment; (c) two or more consumers need it without importing an owner;
(d) it carries no execution, persistence, policy engine, or workflow. Name the
two-plus consumers *before* adding the type. For a **dependency-inversion
port** the two consumers are the declaring caller and the implementing owner —
a port with one caller crate and one implementor crate **passes** (that
separation is the port's whole point), while a port whose caller and
implementor are the same crate **fails** however many impls it has inside that
crate (PROPOSAL §12.11 D-A/D-D; `.claude/rules/type-placement.md` states the
same rule from the trait side).

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_host_api`](./ironclaw_host_api) | The dependency-free authority vocabulary: ids/scopes/paths/mounts, capability/action/decision/approval shapes, the sealed `Authorized` witness and `CapabilityDispatcher` port, sanitized resolution/failure vocabulary, ingress/egress descriptors, runtime/trust vocabulary, and the complete canonical `turn` vocabulary | the type names authority or identity the whole workspace shares, and it can live with zero dependencies |
| [`ironclaw_common`](./ironclaw_common) | Domain-free cross-cutting primitives with persisted wire-compatibility guarantees (identity newtypes, pkce, hashing, paths, timezone, attachment formats) | the primitive is genuinely domain-free, has consumers across layers, and is data rather than behavior |
| [`ironclaw_prompt_envelope`](./ironclaw_prompt_envelope) | The untrusted-snippet envelope: `wrap_untrusted` over a closed source/trust vocabulary, hijack-marker rejection, byte cap | untrusted, source-attributed text is about to become model-visible |
| [`ironclaw_loop_contracts`](./ironclaw_loop_contracts) | The loop-tier port set: the `Loop*Port` family, `AgentLoopDriver`, run-profile vocabulary, and the `LoopExit` claim | a loop, hook, or host adapter must talk to the turn kernel without importing it |
| [`ironclaw_extension_contracts`](./ironclaw_extension_contracts) | What an installable extension *is and exposes*: `ChannelAdapter`/`ToolAdapter`, manifest surfaces, auth recipes, lifecycle states, sealed verified-inbound evidence | lanes, hosts, packages, and product must share extension vocabulary without importing the registry |
| [`ironclaw_product_contracts`](./ironclaw_product_contracts) | The `ProductSurface` membrane, product wire DTOs, and the product-side ports whose implementors sit beside or below product | a transport or collaborator must speak the product boundary without compiling `ironclaw_assistant` |

## What never belongs here

- **Implementations of any port declared here** → the crate that owns the
  behavior: the kernel implements `CapabilityDispatcher`
  (`ironclaw_capabilities::RuntimeDispatcher`); the loop-hosting tier
  implements the `Loop*Port` set; extension packages implement
  `ChannelAdapter`/`ToolAdapter`; product, operator, the extension host, the
  extension manager, composition, and `ironclaw_identity` implement the
  product-side ports. Two visible exceptions only: test doubles behind an
  explicit `test-support` gate, and pure forwarding impls over smart pointers.
- **Execution, persistence, or storage of any kind** → mechanism belongs in
  `crates/substrates/`; durable record grammar belongs in `crates/domains/`.
  This includes persistence *ports*: even a bare store trait belongs in the
  domain that owns the records, not in the vocabulary crate that describes
  them.
- **Rendering, parsing, or classification behavior** → that is workflow, and
  workflow lives above this family (product tier). Validation and
  serialization helpers on a type's own shape are fine.
- **HTTP frameworks, DB clients, WASM runtimes** → `axum` lives in
  `ironclaw_host_ingress` and the webui transport; drivers live in the
  substrates that charter them. Enforced by name (see gates below).
- **Logging or channel side effects** → the caller that owns the workflow.
  A contracts crate may not log; error projections defined here leave logging
  with each caller.
- **Vendor names** → `crates/extensions/packages/*`, `ironclaw_llm` providers,
  `ironclaw_operator`, and recipes-as-data. Two censused, frozen exceptions
  exist in this family and are not licences:
  `ironclaw_product_contracts::operator_llm` (6 DTOs / 3 methods / 2 vendors,
  exact roster pinned) and `ironclaw_common::llm_costs` (shrink-only residue
  pending an owner call). Do not add a name to either.
- **A second import path for another contracts crate's vocabulary** — no
  contracts crate re-exports another contracts crate's port under its own path
  (the §11.2.4 re-export trap; the three location scans below fail on it).

## The rules, and what enforces them

All gates live in `crates/app/ironclaw_architecture_tests` (run:
`cargo test -p ironclaw_architecture_tests`).

- **Layer matrix.** Every crate here declares
  `[package.metadata.ironclaw] layer = "contracts"` — the bottom of
  `contracts < substrates < runtimes < kernel < loops < products < app`. A
  contracts crate may name only contracts crates. The family directory is
  ownership and discoverability only; the layer metadata is the enforced truth
  (`reborn_dependency_boundaries.rs`, over `cargo metadata`).
- **Internal-dependency allowlists**
  (`reborn_dependency_boundaries.rs::reborn_crate_dependency_boundaries_hold`):
  `ironclaw_host_api` — zero workspace deps, asserted against every other
  crate; `ironclaw_extension_contracts` — `{host_api}`;
  `ironclaw_product_contracts` — `{host_api, extension_contracts}`;
  `ironclaw_loop_contracts` — `{host_api, common, prompt_envelope,
  extension_contracts}` (the manifest today uses `host_api` +
  `extension_contracts`). `ironclaw_common` and `ironclaw_prompt_envelope`
  hold zero internal deps; an upward edge fails the layer matrix and a new
  sideways edge fails the same-layer inventory below.
- **Framework/driver deny**
  (`reborn_contracts_crates_hold_no_framework_dependencies`): no
  axum/hyper/tower/reqwest/tonic, no libsql/rusqlite/sqlx/tokio-postgres/
  deadpool, no wasmtime — across all six crates. The one carve-out is
  `ironclaw_loop_contracts`' `tokio` with the `rt` feature only, documented in
  its manifest and pinned in the test.
- **Size ceilings** (`reborn_contracts_crates_carry_a_checked_size_ceiling`):
  each crate carries a production-line ceiling raised only by explicit review,
  bounded below so banked slack fails too — the allowlist cannot see a crate
  that imports nothing and implements everything; this can.
- **Same-layer edges** (`reborn_same_layer_edge_inventory.rs`): exactly five
  intra-family edges are pinned with owners — `extension_contracts →
  host_api`, `loop_contracts → {host_api, extension_contracts}`,
  `product_contracts → {host_api, extension_contracts}`. A new one fails.
- **Port location, one import path**
  (`reborn_loop_port_location_scan.rs`,
  `reborn_extension_contract_location_scan.rs`,
  `reborn_product_contract_location_scan.rs`): one definition per contract
  workspace-wide, no re-export shadowing. A new `Loop*Port` needs its
  `LOOP_PORT_OWNERS` row in the same change.
- **Sealed evidence minting** (`reborn_sealed_evidence_mint_ratchet.rs`):
  `host_protocol_authenticator_is_implemented_only_by_the_host_transport` and
  `channel_ingress_verifier_is_implemented_only_by_the_generic_verifier` — the
  witness-grant seal on `Authorized`, bearer/session evidence, and
  verified-inbound evidence has a compiler half (no minting without a grant)
  and this architecture-test half (exactly one grant-producing implementor).
- **Vendor census** (`reborn_contracts_vendor_census.rs`):
  `reborn_contracts_family_names_llm_vendors_only_in_censused_scopes` and
  `reborn_d_e_sanctioned_vendor_api_surface_is_frozen` hold the two exceptions
  above at their exact rosters; `reborn_extension_specificity.rs` scans the
  family like every other crate.
- **Product-boundary splits** (`reborn_transport_product_boundary.rs`,
  `reborn_service_method_freeze_ratchet.rs`): descriptor *types* here, frozen
  operation *constants* in product, `ProductSurface` method set frozen.
- **Re-export discipline:** vocabulary is exported module by module — never a
  flat wildcard prelude — so a reader always sees which module a type comes
  from (`ironclaw_host_api`'s `lib.rs` states the no-prelude rule).

## Crossing out of this family

- **`crates/substrates/`** (up): the type needs real mechanism — I/O, disk,
  crypto, a driver. Contracts describes the shape a substrate accepts; the
  substrate does the work.
- **`crates/domains/`** (up): the type has a persistence story. Records and
  store traits live with their domain.
- **`crates/kernel/`** (up): something must *decide* or *mint* authority.
  The port lives here; the power to satisfy it lives there.
- **`crates/loop/`, `crates/extensions/`, `crates/product/`** (up): port
  implementations, the extension registry and hosting machinery, the
  `ProductSurface` implementation and the frozen command/view/capability
  inventory.
- **Down:** nowhere. This family is the floor; it depends on nothing outside
  itself.

## Sources

- Design record: [`docs/internal/reborn/target-architecture/families/contracts.md`](../../docs/internal/reborn/target-architecture/families/contracts.md);
  PROPOSAL §6.1 (per-crate as-built inventories), §8 (dependency model).
- Conventions this file follows: [`docs/internal/reborn/guidance-conventions.md`](../../docs/internal/reborn/guidance-conventions.md).
- Moving a crate between families is not a rename — the family word never
  enters the crate name (PROPOSAL §5.1).
