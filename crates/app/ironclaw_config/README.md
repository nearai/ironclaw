# ironclaw_config

Boot-time configuration contracts for the standalone binary: Reborn home,
profile, and boot resolution, the `config.toml` schema, configuration seeding,
budget environment defaults, inline-secret rejection at parse time, and the
retired-section gravestone table. It is a pure leaf with a machine-enforced
**zero workspace dependencies** rule — the guarantee is the crate's entire
reason to be its own compilation unit.

- **Family / layer:** `app` / `substrates` (declared since the first layer
  gate, #5852 — the zero-dep rule, not the layer, is what protects it) ·
  **Package:** `ironclaw_config` · **Manifest:** `crates/app/ironclaw_config/Cargo.toml`
- **Use this when:** changing what an operator can put in `config.toml`, home
  or profile resolution, seeding, budget env defaults, or retiring a section.
- **Don't use this when:** the value is vendor-specific live configuration →
  package-owned `[admin_configuration]` in the extension's manifest (the only
  vendor tokens allowed here are the retired table names in
  `retired_sections.rs`; the surviving `GoogleSection` is sequenced behind
  the CLI shed, PROPOSAL §6.10.3); a crate outside this family needs a
  boot-time value → it arrives as construction input from composition, never
  as a dependency on this crate.

## Public surface

- `RebornHome` / `REBORN_HOME_ENV` (`home.rs`), `RebornProfile` /
  `REBORN_PROFILE_ENV` (`profile.rs`), `RebornBootConfig` (`boot.rs`).
- `RebornConfigFile` + the `config.toml` schema (`config_file.rs`,
  `deny_unknown_fields`), seeding (`config_seed.rs`), `RebornDoctorReport`
  (`doctor.rs`), budget env defaults (`budget.rs`).
- `reject_inline_secret` / `InlineSecretError` (`secrets_guard.rs`) — the
  fail-closed refusal of raw secret values typed into the file, at any depth.
- `RetiredSections` / `retired_config_key_guidance` (`retired_sections.rs`) —
  the compatibility window for sections this crate used to define: an
  existing file still parses, a retired *setup* key still fails `serve`
  closed, an inert section boots and says so.
- `capability_remediation.rs` — remediation text helpers (not dead: consumers
  in four crates).

## Depends on / consumed by

- **Normal workspace deps (0).** None, ever — in any dependency kind.
- **Consumed by (4, measured):** `ironclaw` (the binary),
  `ironclaw_composition`, `ironclaw_operator`, `ironclaw_extension_host`.
  The family target says only the assembly crate and the binary should hold
  the edge; the operator and extension-host edges are recorded divergences
  (families/product.md §6.9.2's correction), not precedent.

## Invariants

- **Zero workspace dependencies** — the `ironclaw_config` `BoundaryRule`
  (`reborn_dependency_boundaries.rs::reborn_crate_dependency_boundaries_hold`).
- **The boot-file layout is pinned**:
  `reborn_dependency_boundaries.rs::reborn_boot_config_file_layout_is_pinned`.
- **No vendor section may return**: `RebornConfigFile` names no vendor; the
  extension-specificity gate (`reborn_extension_specificity.rs`) is
  shrink-only, so adding one needs a baseline raise and a reviewed carve-out.
- Retiring a section is a `RETIRED_SECTIONS` row, never a deletion —
  deletion breaks every existing operator file against `deny_unknown_fields`
  (see `AGENTS.md` "Retiring a config section" for the procedure, and grep
  `docs/` too).

## Tests

```bash
cargo test -p ironclaw_config          # incl. profile_contract, doctor_contract, home_contract
cargo test -p ironclaw_architecture_tests
```

## See also

Working rules: `AGENTS.md` · family rules: `crates/app/AGENTS.md` · design
record: `docs/reborn/target-architecture/families/app.md` (§6.10.3) +
PROPOSAL §12.2 (the compatibility constraint).
