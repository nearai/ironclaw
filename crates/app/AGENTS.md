# `crates/app/` — assembly and enforcement; composition wires owners, never becomes one

**Layer(s):** `app`, except `ironclaw_config` at `substrates` (declared since #5852; its zero-dep rule is what matters) · **Crates:** 4 · **May depend on:** everything — the one family whose crates are permitted to see the whole workspace · **Depended on by:** nothing; no other crate may import any of these (`ironclaw_config` is the exception by design: a pure leaf consumed by `ironclaw`, `ironclaw_composition`, `ironclaw_operator`, and `ironclaw_extension_host`)

## What this family is

The assembly root, the shipped artifact, the boot-configuration leaf, and the
enforcement suite. `ironclaw_composition` constructs every other family's
owners into a running deployment; `ironclaw_cli` produces the binary an
operator runs (package and binary name `ironclaw`); `ironclaw_config` is the
boot contract they read; `ironclaw_architecture_tests` fails the build when any
crate's dependency graph or public surface drifts from the declared model.
Seeing everything is not license to become anything: **composition holds
deployment *shape* — closed choices of runtime substrate, storage backend,
profile, and mode — never domain behavior.** The operative test for any change
to what the assembly crate contains: if a module computes a policy decision,
renders a prompt, or owns a domain record shape, it belongs to the family that
owns the concept.

## The crates

| Crate | Charter (one line) | Go here when |
| --- | --- | --- |
| [`ironclaw_composition`](./ironclaw_composition) | The assembly root: deployment selection, storage-backend choice, owner-factory invocation, readiness, service-graph handles | You are wiring an owning crate's factory into a deployment |
| [`ironclaw_cli`](./ironclaw_cli) | The binary `ironclaw`: command surface, serve wiring, concrete-extension binding tables, first-party registrars, the admin token minter | You are adding a command or linking a concrete package |
| [`ironclaw_config`](./ironclaw_config) | Boot contract: home/profile/boot resolution, `config.toml` schema, seeding, inline-secret rejection, retired-section gravestones | You are changing what an operator can put in `config.toml` |
| [`ironclaw_architecture_tests`](./ironclaw_architecture_tests) | Test-only mechanical enforcement of every boundary the design record claims | You are arming (or re-deriving) a boundary as a test |

## What never belongs here

- **Domain behavior of any kind** — a record shape, a state machine, a
  redaction rule. Each belongs to the family that owns the concept; every CLI
  command is a thin caller into the assembly root or a family crate.
- **Policy *content*, as distinct from policy *selection*.** Deployment mode
  and profile are data points the assembly root picks; what a policy permits
  is computed in the kernel family, never here.
- **Prompt content of any kind.** Loop-tier prompt assets live in
  `crates/loop/ironclaw_loop_host/prompts/`; composition keeps only assembly
  and the boot-time `std::fs` seeding of the on-disk `SYSTEM.md`
  (`composition_root_embeds_no_prompt_content` pins the direction).
- **HTTP route handler logic.** Mounting a prebuilt router behind an
  `ironclaw_host_ingress` carrier is fine; writing the handler is not.
- **Vendor flows**, beyond the family's two licensed shapes: the CLI's
  first-party registrar wiring for a small set of vendor integrations (the
  binary's equivalent of the extension-binding table), and the vendor
  knowledge held *as data* in `ironclaw_config`'s retired-section table.
- **Naming a concrete extension package anywhere but the binary.**
  `ironclaw_cli` alone links `ironclaw_slack_extension` /
  `ironclaw_telegram_extension` and builds the binding table; composition
  receives every binding as an opaque, pre-built handle and can never itself
  name a package.
- **Production code in the enforcement crate.** `ironclaw_architecture_tests`
  has no library surface, no runtime role, and only dev-only vocabulary
  imports (`ironclaw_host_api`, `ironclaw_product_contracts` — see its
  `Cargo.toml` for why each exists) — a change that makes a boundary test
  easier to pass, rather than the design more correct, is the failure mode
  this family's guidance exists to
  name.

## The rules, and what enforces them

- **"Wires owners, never becomes one."** Composition's public API stays
  service-shaped, its `pub use` wall matches a reviewed snapshot, every
  survivor names its consumer, and the binary's `main` stays a thin bootstrap:
  `cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries`
  (`composition_public_api_is_service_shaped`,
  `composition_public_pub_use_surface_matches_snapshot`,
  `composition_public_pub_use_entries_name_their_consumer`,
  `composition_root_embeds_no_prompt_content`,
  `reborn_binary_main_is_thin_bootstrap`,
  `composition_crate_installs_installed_tier_only_through_registrar`).
- **Composition's mass is governed by a ratchet, not taste.**
  `bash scripts/ci/check-composition-budget.sh` (budget:
  `scripts/ci/composition-budget.toml`) — current ceilings are tracked live in
  that file (`loc_ceiling`, `arc_dyn_ceiling`); run
  `bash scripts/ci/check-composition-budget.sh --print` for today's values
  rather than trusting a number restated in prose here. The share metric
  (`ceiling_bp = 658`) is a deliberately-frozen WS0 record, not a moving
  ratchet. One-directional: re-ratchet down in the same PR as any eviction;
  raising needs a one-line PR rationale.
- **Nothing depends on `app`.** No substrate reaches the composition root
  (`reborn_composition_boundaries.rs::no_substrate_crate_depends_on_composition_root`),
  and the CLI's dependency set is asserted *exactly*
  (`assert_workspace_deps_exactly` for package `ironclaw`, in
  `reborn_dependency_boundaries.rs` — which also pins
  `crates/app/ironclaw_cli/AGENTS.md`'s command-layout phrases).
- **`ironclaw_config` has zero workspace dependencies** — non-negotiable, the
  crate's entire reason to be a separate compilation unit; enforced by its
  `BoundaryRule`:
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_crate_dependency_boundaries_hold`
  (boot-file layout: `reborn_boot_config_file_layout_is_pinned`). A crate
  needing a boot-time value receives it as construction input from the
  assembly root, never as a direct `ironclaw_config` dependency.
- **The tree half is enforced too.** The package set and the documented tree
  must agree: `scripts/ci/check-target-tree.py`, and the path-keyed gates
  resolve crates through a pinned inventory
  (`cargo test -p ironclaw_architecture_tests --test reborn_crate_inventory`).
- **Layer declarations and the matrix:**
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries reborn_workspace_crates_declare_layers_and_follow_layer_matrix`.

## Crossing out of this family

- **Down to everywhere:** composition calls every family's own factory
  functions (module-owned initialization — never reconstructing what a crate's
  factory builds); the binary calls composition, `ironclaw_operator`, and the
  concrete packages it alone may name.
- **Trusted binding tables live only in the binary:** the
  package-to-adapter table (`native_extensions.rs`) and the
  `AdminApiTokenMinter` implementation — the port composition defines and
  deliberately does not satisfy.
- **Nothing crosses back in.** If a lower crate wants something from this
  family, the answer is construction input, a port, or a factory — never an
  `app` import.

## Sources

`docs/internal/reborn/target-architecture/families/app.md` · PROPOSAL §6.10.1–6.10.4,
§8 · gates: `crates/app/ironclaw_architecture_tests/tests/`
(`reborn_composition_boundaries.rs`, `reborn_dependency_boundaries.rs`,
`reborn_crate_inventory.rs`) + `scripts/ci/check-composition-budget.sh` +
`scripts/ci/check-target-tree.py` · module spec:
`ironclaw_composition/CONTRACT.md` (root `AGENTS.md` Module Specs table) ·
conventions: `docs/internal/reborn/guidance-conventions.md`.
