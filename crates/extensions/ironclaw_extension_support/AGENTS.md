# Agent Map — ironclaw_extension_support

## Start Here

- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_host_runtime/AGENTS.md`
  - `crates/ironclaw_filesystem/AGENTS.md`

## Where The Packages Live

- **Not here.** Every installable package is its own self-contained directory at
  `crates/extensions/packages/<extension-id>/` — manifest, prompts, schemas,
  committed `wasm/`, and the `wasm-src/` guest that produced it, together
  (PROPOSAL §5). This crate is the shared *support* crate beside them: the
  package inventory (`src/packages/`) and the native executors that serve many
  packages at once.
- A package module here embeds its package's data with
  `include_str!`/`include_bytes!` through `../../../packages/<id>/…`. If you add
  a package, add its directory under `packages/` and its module here, and put it
  in the `PACKAGES` table — the catalog is derived from that table, not from a
  directory scan.
- **One package module is deliberately not in `PACKAGES`: `nearai`.** Its
  shipped `[mcp].server` is a placeholder the host rewrites from the operator's
  LLM-admin bootstrap configuration, and a `fn() -> PackageBundle` cannot
  produce that value. Its embeds still live here, with every other package's;
  the patch lives with the endpoint authority in `ironclaw_extension_host`,
  which calls `packages::nearai::nearai_bundle()`. Read that module's header
  before assuming the omission is an oversight — it is the documented shape.
- Rebuilding a WASM guest: `./scripts/build-wasm-extensions.sh --first-party`,
  then `python3 scripts/ci/check-wasm-artifact-freshness.py --update`. The
  freshness gate fails if you edit `wasm-src/` and do not do both.

## What This Crate Owns

- Concrete first-party userland extension implementations that ship with IronClaw.
- Deterministic tool behavior behind narrow explicit request types.
- Scoped handles granted by host runtime or composition.

## The Executor/Adapter Seam

Every tool here is an **executor**, never a capability handler. That means:

- It takes a request type this crate defines (`GsuiteDispatchRequest`,
  `WebAccessDispatchRequest`, `SkillUrlFetchContext`, …) carrying only
  contracts-layer values the host hands it per invocation — `ResourceScope`,
  `CapabilityId`, `Arc<dyn RuntimeHttpEgress>`, `Arc<dyn RootFilesystem>`.
- It returns this crate's own error type (`…DispatchError`,
  `SkillManagementCapabilityError`), which carries a
  `RuntimeDispatchErrorKind` and optionally the `ResourceUsage` burned before
  the failure. The caller maps it.
- The `FirstPartyCapabilityHandler` impl, the `CapabilityManifest` that declares
  the tool, and the registry insertion live **outside** — in
  `ironclaw_host_runtime::first_party_tools` for the always-on builtins, or in
  the binary's `FirstPartyHandlerRegistrar` for the binary-registered ones.

`ironclaw_host_runtime` and `ironclaw_extension_registry` are on this crate's forbidden
list (`reborn_dependency_boundaries.rs`), so this is enforced, not a
convention. If an executor cannot be written without one of them, the seam is
in the wrong place — move less, not the rule.

## Do Not Move In Here

- Host runtime composition, authorization, approvals, resource accounting, or capability registry wiring.
- Capability-handler implementations or the capability manifests that declare them.
- Loop-facing skill context ports, turn-run adapters, or Reborn composition wiring.
- Raw secrets, network clients, dispatcher handles, or ambient host authority.

## Validation

- Fast local check: `cargo test -p ironclaw_extension_support`
- Caller check after tool behavior changes: `cargo test -p ironclaw_host_runtime --test first_party_coding_tools`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold`
