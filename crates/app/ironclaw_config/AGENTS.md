# Agent Map — ironclaw_config

Working rules for the boot-configuration leaf. Orientation lives in
`README.md`; family rules in `crates/app/AGENTS.md`. (The `CLAUDE.md` beside
this file is a symlink alias of it, per `docs/reborn/guidance-conventions.md`;
this file is the canonical home for its working rules.)

## Start Here

- Read `README.md`, then `src/lib.rs` for exports, then area files:
  - `home.rs` — Reborn home resolution.
  - `profile.rs` — profile contracts.
  - `boot.rs`, `config_file.rs` — boot/config file loading.
  - `doctor.rs` — config diagnostics.
  - `secrets_guard.rs` — secret/config guardrails.
  - `retired_sections.rs` — the compatibility window for `config.toml`
    sections this crate used to define and no longer does.
- Neighboring consumers: `crates/app/ironclaw_cli/AGENTS.md`, `crates/app/ironclaw_composition/AGENTS.md`.

## What This Crate Owns

- Boot configuration contracts for the standalone IronClaw Reborn binary.
- Reborn home/profile/config-file/doctor/secrets-guard types and validation.
- Pure config parsing/validation helpers that can be shared by CLI and composition.

## Do Not Move In Here

- Runtime execution, product adapter workflow, host-runtime service construction, or CLI command dispatch.
- Writes to v1/current IronClaw state.
- Network, secret retrieval, database connection, or product side effects.
- **A new per-vendor `config.toml` section.** An extension's live configuration
  is package-owned, declared by its manifest `[admin_configuration]` — it does
  not get a typed section here. `RebornConfigFile` names no vendor, and the
  extension-specificity gate
  (`crates/app/ironclaw_architecture_tests/tests/reborn_extension_specificity.rs`) is
  shrink-only, so adding one needs a baseline raise and a reviewed carve-out.
  The only vendor tokens this crate may hold are the retired table names in
  `retired_sections.rs`.

## Retiring a config section

Deleting a section outright breaks every existing operator file, because
`RebornConfigFile` is `deny_unknown_fields`. Instead add a row to
`RETIRED_SECTIONS` (`retired_sections.rs`) naming the section, the *setup*
keys that should fail the boot closed, and what to do instead. Everything
else follows from the table: the parse-time split, the `serve` refusal, the
deprecation notice for inert sections, and `config set`'s migration guidance.
See PROPOSAL §6.10.3 / §12.2 for why the gravestone lives here rather than in
the package. **Grep `docs/` as well as `crates/`** — the Slack retirement
found five operator-facing docs still teaching a flag that had had no reader
for two weeks.

## Validation

- Fast local check: `cargo test -p ironclaw_config`
- Focused tests: `profile_contract`, `doctor_contract`, `home_contract`.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Keep config contracts deterministic and side-effect light.
- Use explicit Reborn home/profile inputs; do not read ambient env from deep helpers unless that is the contract being tested.
- Add compatibility tests for serialized config/profile shapes.
