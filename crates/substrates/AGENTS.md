# `crates/substrates/` — privileged mechanism, never authority

**Layer(s):** `substrates` · **Crates:** 7 · **May depend on:** `ironclaw_host_api`
(contracts), `ironclaw_observability`, plus exactly three charted sibling edges
(below) · **Depended on by:** every layer above — kernel services mediate
filesystem/secrets/network for the tiers above them, domain crates hold direct
filesystem handles (the storage-placement rule), and `ironclaw_composition`
constructs the concrete backends.

## What this family is

The durable, reusable mechanisms the kernel mediates: storage fabric, database
connection admission, secret custody, network policy and transport, safety
scanning, and cross-cutting tracing. A substrate does real privileged work and
enforces its own fail-closed local invariant — mount containment, single-writer
admission, one-shot lease consumption, private-address denial — but it never
decides whether the caller was *entitled* to invoke it. That decision is made
in `crates/kernel/` before the call arrives.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_filesystem`](./ironclaw_filesystem) | Storage fabric: `RootFilesystem` trait, `ScopedFilesystem` mount enforcement, mount catalog, CAS floor, disk/libSQL/Postgres/in-memory backends | Bytes or records need a durable home behind the one trait |
| [`ironclaw_documents`](./ironclaw_documents) | Bounded, structure-preserving OOXML transforms and deterministic HTML-subset PDF rendering | Document bytes need addressable reads or typed, loss-averse transforms without filesystem authority |
| [`ironclaw_libsql_runtime`](./ironclaw_libsql_runtime) | libSQL connection admission: one bounded read pool + exactly one writer lane per database | You need a libSQL connection — this is the only legal source |
| [`ironclaw_network`](./ironclaw_network) | Egress policy and hardened outbound transport; the workspace's only `reqwest` owner | An outbound HTTP call must be policy-checked before it exists |
| [`ironclaw_observability`](./ironclaw_observability) | Zero-cost-when-off latency-trace macros; exactly one dependency (`tracing`) | You want to time an operation without adopting a tracing stack |
| [`ironclaw_safety`](./ironclaw_safety) | Injection detection, input validation, leak scanning, credential detection, display redaction | Untrusted text needs scanning or redaction at a trust boundary |
| [`ironclaw_secrets`](./ironclaw_secrets) | Encrypted secret custody, one-shot leases, credential broker | Secret material needs storing, leasing, or brokering |

**Mediation story — who may call each crate directly.** This is the one thing
every crate here answers differently (`families/substrates.md` requires it
stated per crate):

- `ironclaw_filesystem` — kernel services and domain record owners hold direct
  trait handles; that breadth is the point (28 normal-dep consumers, measured
  via `cargo metadata` 2026-08-05).
- `ironclaw_libsql_runtime` — exactly `ironclaw_filesystem`,
  `ironclaw_triggers`, `ironclaw_composition` (measured fan-in 3; the three sit
  in three families, which is why the crate exists).
- `ironclaw_network` — `ironclaw_host_runtime` is the production caller;
  composition constructs. Measured today the direct-consumer set is wider:
  `extension_host`, `extension_manager`, and the sandbox lane also hold normal
  edges (5 total) — standing narrowing targets, not charter.
- `ironclaw_observability` — anyone (7 consumers); the only crate here with no
  security-relevant surface.
- `ironclaw_safety` — any caller needing detection (17 consumers).
- `ironclaw_secrets` — the tightest. The auth engine (`ironclaw_auth`) is the
  chartered direct consumer (it owns token custody); everything else is meant
  to arrive through kernel staging or a `product_contracts` port. Measured
  today: 8 normal consumers (`assistant`, `auth`, `composition`,
  `extension_host`, `extension_manager`, `host_runtime`, `sandbox`, `stress`);
  the surplus is tracked narrowing (PROPOSAL §6.2.2, `extension_manager` in
  #7095), not permission.
- `ironclaw_documents` — pure byte-transform callers may invoke it directly;
  filesystem path selection, overwrite policy, and writes stay in the mediated
  host-runtime caller.

## What never belongs here

- **An authority decision** (who may call, trust ceilings, approvals) → that is
  `crates/kernel/`; a substrate that starts deciding duplicates the kernel with
  none of its guarantees.
- **Domain record grammar or service identity** (what is true about a trigger,
  a thread, a memory item) → `crates/domains/`, built on top of this family.
- **Product or vendor behavior, branching, or naming** → `crates/extensions/`
  packages and `crates/product/`; every crate here is vendor-blind.
- **Ambient credentials or a standing network client handed to an unmediated
  caller** → every credential or egress path here is scoped, leased, or
  policy-checked per call; runtime credential *injection* is the kernel's
  obligation handling, not this family's.
- **SQL, schema, migrations, or transactions in the admission runtime** → the
  backend crates that own their records (`ironclaw_filesystem`,
  `ironclaw_event_store`, the two ADR'd exceptions).
- **A second libSQL pool anywhere in the workspace** → the single-writer
  invariant only holds while `ironclaw_libsql_runtime` is the sole pool home.
- **Upward edges** — nothing here depends on anything above substrates, and a
  lattice of substrate-on-substrate edges beyond the three charted ones would
  recreate the driver-cone leakage this family exists to prevent.
- **A placeholder or demo backend running as a real security boundary.**

## The rules, and what enforces them

All gates below run with `cargo test -p ironclaw_architecture_tests`.

- **Layer matrix.** Every crate declares `[package.metadata.ironclaw] layer =
  "substrates"`; `reborn_workspace_crates_declare_layers_and_follow_layer_matrix`
  (`reborn_dependency_boundaries.rs`) enforces the 7-layer ladder
  (`contracts < substrates < runtimes < kernel < loops < products < app`).
- **Named forbidden edges.** `reborn_crate_dependency_boundaries_hold` carries
  `BoundaryRule` entries for `ironclaw_filesystem`, `ironclaw_libsql_runtime`,
  `ironclaw_secrets`, and `ironclaw_network`, each forbidding the high-signal
  upward edges by name.
- **Sibling edges are closed at three (plus observability).** The only
  family-internal normal edges are `filesystem → safety` (one sensitive-path
  predicate), `filesystem → libsql_runtime` (connection admission), and
  `secrets → filesystem` (custody over the fabric) — plus `filesystem →
  observability` (tracing). Same-layer edges are outside the layer matrix's
  reach, so each is inventoried with an owner in
  `reborn_same_layer_edge_inventory.rs`
  (`reborn_every_same_layer_edge_is_inventoried_and_no_entry_is_stale`); a new
  one fails the build.
- **The persistence/driver rule (PROPOSAL §11.2.6), both halves.**
  *Admission is singular:* only `ironclaw_libsql_runtime` may construct a
  libSQL pool — enforced today via the driver-dependency gate: the `deadpool`
  allowlist in `ADDITIONAL_DRIVER_ALLOWLISTS`
  (`reborn_persistence_driver_boundary.rs`) is exactly
  `{ironclaw_filesystem, ironclaw_libsql_runtime}`, so no other crate can even
  link the pool library. *Driver deps are closed and shrink-only:*
  `only_chartered_crates_link_the_postgres_driver` and
  `only_chartered_crates_link_the_other_persistence_drivers` pin the
  `deadpool-postgres`/`libsql`/`deadpool`/`tokio-postgres` linker sets as exact
  equalities — growth fails, and a crate that drops a driver must leave the
  list in the same PR.
- **The kernel does not leak substrate handles upward:**
  `reborn_host_runtime_services_do_not_expose_lower_substrate_handles`.
- **A family directory is never a compilation or trust unit.** The enforced
  truth is each crate's declared layer; family placement is ownership and
  discoverability only (PROPOSAL §5). Moving a crate between families is not a
  rename — the family word never enters the crate name (§5.1).

## Crossing out of this family

- **Down to `crates/contracts/`** (`ironclaw_host_api`) — the vocabulary these
  mechanisms accept; contracts do no work, substrates do the work.
- **Up to `crates/kernel/`** — when the question is "may this caller do this at
  all"; grants, staging, and obligations live there.
- **Sideways to `crates/events/`** — append-only evidence; `event_store` is a
  consumer built on the filesystem fabric, never a mutable resource here.
- **Up to `crates/domains/`** — record grammar and service identity built on
  the fabric ("what is true about this entity").
- **Up to `crates/app/ironclaw_composition`** — backend selection; it opens
  each physical database exactly once and wires the shared runtime.

## Sources

[`docs/internal/reborn/target-architecture/families/substrates.md`](../../docs/internal/reborn/target-architecture/families/substrates.md)
(full charter, boundaries, security posture) · PROPOSAL §6.2 (per-crate
contracts), §8 (dependency model), §11.2.6 (persistence rule) · the gates named
above.
