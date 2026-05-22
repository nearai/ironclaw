# Reborn CLI Agent Contract

This crate owns the standalone `ironclaw-reborn` command surface. Keep it small, explicit, and safe for agents to extend.

## Command layout

- Use one command per file under `src/commands/`.
- Register each command in `src/commands/mod.rs` and dispatch through `Command::execute`.
- Keep `src/cli.rs` as the clap root only: parse top-level CLI and hand off to command modules.
- Put shared process/env boot state in `RebornCliContext` from `src/context.rs`.

## Boundaries

- Commands that need Reborn boot config must receive `RebornCliContext` from dispatch instead of reading env directly. Pure commands that do not need boot config (for example, shell completion generation) must not force Reborn home resolution.
- Keep commands side-effect free unless the command name and issue explicitly require mutation.
- Use `IRONCLAW_REBORN_HOME` / `~/.ironclaw/reborn`; do not write current v1 state.
- no v1 runtime imports: do not depend on root `ironclaw`, `src/agent`, channels, worker, DB, setup, service, sandbox, or `ironclaw_engine`.
- Do not add workspace dependencies beyond `ironclaw_reborn_composition`, `ironclaw_reborn_config`, and `ironclaw_reborn_webui_ingress` (host-owned WebUI serve lifecycle) without an architecture test update and explicit PR rationale.

## Adding a command

1. Add `src/commands/<name>.rs` with a clap `Args` type and an `execute` method.
2. Add a variant to `commands::Command`.
3. If the command needs boot config, resolve `RebornCliContext` in `commands::Command::execute` and pass it into the command handler.
4. If the command is pure, do not resolve `RebornCliContext` just to run it.
5. Add a binary smoke test in `tests/smoke.rs` that invokes `env!("CARGO_BIN_EXE_ironclaw-reborn")`.
6. If the command can touch state, assert it uses Reborn home only and does not create/read v1 DB/settings/secrets.
7. Run:
   - `cargo test -p ironclaw_reborn_cli`
   - `cargo test -p ironclaw_architecture reborn`
   - `cargo clippy -p ironclaw_reborn_cli --all-targets -- -D warnings`
