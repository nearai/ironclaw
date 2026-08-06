# ironclaw_composition

The assembly root — deployment selection and dependency wiring, exclusively.
It is the one crate in the workspace allowed to see everything (49 normal
workspace dependencies, measured), because its job is to construct every other
family's owners: it selects a deployment's shape through closed choices
(runtime substrate, storage backend, profile, mode), opens each physical
database exactly once, invokes owner factories, computes fail-closed
readiness, and exposes the result as service-shaped handles. **It wires
owners; it never becomes one** — if a module here computes a policy decision,
renders a prompt, or owns a domain record shape, it belongs to the family that
owns the concept.

- **Family / layer:** `app` / `app` · **Package:** `ironclaw_composition` · **Manifest:** `crates/app/ironclaw_composition/Cargo.toml`
- **Use this when:** wiring an owning crate's factory into a deployment,
  choosing a backend by profile, adding a readiness check, or adding a
  service-graph handle.
- **Don't use this when:** the change is behavior — a record shape, a policy
  computation, a prompt, a route handler → the owning family's crate; naming
  a concrete extension package or minting the admin token →
  `crates/app/ironclaw_cli` (the binary alone holds those).

## Public surface

- `build_reborn_runtime` (`src/runtime.rs`) — the assembly entry point (the
  name pinned by the composition boundary test), delegating to the equally
  public `build_runtime`; `build_runtime_substrate` (`src/factory.rs`) is the
  `pub(crate)` builder underneath.
- `RebornHostBindings` / `RebornRuntimeInput` (`src/input.rs`) — how a binary
  supplies bindings (channel adapters arrive as opaque, pre-built handles) —
  and `RebornBuildError` (`src/error.rs`).
- `RebornRuntime` — the service-graph handle: composed `HostRuntime`,
  `TurnCoordinator`, product surface, product-auth services, readiness —
  service methods, never raw substrate handles (test-support accessors are
  `#[cfg(any(test, feature = "test-support"))]` and ship zero bytes in
  production).
- The token-minting port the binary implements (`AdminApiTokenMinter` is
  declared in `ironclaw_product_contracts::admin_users`; composition defines
  the seam and deliberately does not satisfy it).

Orientation notes: LLM catalog wiring is **not** here — `llm_catalog` belongs
to `ironclaw_operator` (`crates/product/ironclaw_operator/src/llm_admin/`);
this crate's `src/llm_admin/` holds only `nearai_mcp.rs` (first-boot
provisioning, PROPOSAL §6.10.1's open mechanism decision) and
`openai_compat_serve` (the port *implementations* the OpenAI-compat mount
consumes — they name `ironclaw_threads`/`ironclaw_turns`/
`ironclaw_event_streams`, which the owner crate's BoundaryRule forbids, so
composition holding them is the target state, not debt).

## Depends on / consumed by

- **Normal workspace deps (49):** essentially every owning crate in every
  other family — the one designed exception to the family boundary rule.
- **Consumed by (1):** the `ironclaw` binary. Nothing else may import this
  crate, and no substrate may reach back into it
  (`reborn_composition_boundaries.rs::no_substrate_crate_depends_on_composition_root`).

## Invariants

- **Service-shaped public API** and a reviewed `pub use` wall:
  `composition_public_api_is_service_shaped`,
  `composition_public_pub_use_surface_matches_snapshot` (snapshot:
  `docs/plans/composition-pubuse.snapshot`),
  `composition_public_pub_use_entries_name_their_consumer` — all in
  `reborn_composition_boundaries.rs`.
- **No prompt content**: `composition_root_embeds_no_prompt_content` — prompt
  text lives in the owning crate's `prompts/` (loop tier:
  `crates/loop/ironclaw_loop_host/prompts/`); composition keeps assembly and
  the boot-time `std::fs` seeding of `SYSTEM.md` only.
- **Mass is governed by a ratchet**: `bash scripts/ci/check-composition-budget.sh`
  (budget file `scripts/ci/composition-budget.toml`) — ceilings equal today's
  observed values: **40,423 production LOC** and **814 `Arc<dyn>` sites**,
  plus the share metric (658 bp). Re-ratchet down in the PR that evicts;
  raising requires a stated rationale.
- **Fail-closed readiness**: production and migration-dry-run profiles refuse
  local-only or missing required handles — a gate on whether a deployment may
  serve traffic, never on what a request may do (that is the kernel's).
- **Never names a concrete extension package** — bindings arrive pre-built
  from the binary; installed-tier hooks only through the registrar
  (`composition_crate_installs_installed_tier_only_through_registrar`).

## Tests

```bash
cargo test -p ironclaw_composition
cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries
bash scripts/ci/check-composition-budget.sh          # the mass/dispatch ratchet
scripts/reborn-e2e-rust.sh                           # for production wiring changes
```

## See also

Module spec: `CONTRACT.md` — named in the root `AGENTS.md` Module Specs table;
the spec is the tiebreaker · family rules: `crates/app/AGENTS.md` · design
record: `docs/reborn/target-architecture/families/app.md` (§6.10.1).
