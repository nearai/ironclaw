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
- Do not add workspace dependencies beyond `ironclaw_reborn_composition`, `ironclaw_reborn_config`, `ironclaw_reborn_traces`, and `ironclaw_reborn_webui_ingress` (host-owned WebUI serve lifecycle) without an architecture test update and explicit PR rationale. Provider registry/auth/model UX should enter through the Reborn composition provider-admin facade, not a separate CLI-only path.

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

## Single binary surface

The default `ironclaw-reborn` build includes the Reborn product/runtime surfaces
operators expect from one binary: model-provider wiring, WebChat v2 `serve`,
Slack host-beta mounts, OpenAI-compatible route mounts, and PostgreSQL-backed
hosted/production storage. Runtime config and env vars still fail closed unless
the operator explicitly starts `serve`, supplies WebUI auth, enables Slack, or
selects a hosted/production storage profile.

Compatibility Cargo feature names may remain in the manifest, but new commands
should not hide normal CLI surface area behind non-default features. If a route
or integration is present in the shipped binary, prefer runtime enablement and
config validation over compile-time omission.

The beta WebUI static crate runs the frontend bundler from Cargo build scripts,
so default, `webui-v2-beta`, and `--all-features` builds need Node.js/npm
available even though the generated `static/dist/` bundle is not committed.

With `--no-default-features`, `ironclaw-reborn --help` does not list `serve`
and `ironclaw-reborn serve …` returns `error: unrecognized subcommand`.
This is verified by `help_mentions_reborn_commands` in `tests/smoke.rs`,
which only asserts on the `serve` line under `#[cfg(feature =
"webui-v2-beta")]`. WebUI smoke tests (`serve_help_mentions_host_and_port`,
`serve_fails_closed_when_env_bearer_token_var_is_unset`, etc.) are
themselves feature-gated so default `cargo test -p ironclaw_reborn_cli`
runs include them while `--no-default-features` runs do not regress on a
missing feature flag.

The descriptor-level "all v2 routes are actually mounted" regression
lives at the composition layer in
`crates/ironclaw_reborn_composition/tests/webui_v2_serve.rs`
(`every_webui_v2_descriptor_is_mounted_on_composed_app`), not here —
that test drives the same `webui_v2_app` the CLI's `serve` hands to
`serve_webui_v2`, so a route that's declared in `webui_v2_routes()` but
forgotten by composition fails the build before the CLI binary smoke
tests run.
