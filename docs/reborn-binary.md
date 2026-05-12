# `ironclaw-reborn` standalone binary

`ironclaw-reborn` is the standalone executable boundary for Reborn. It is separate from the current `ironclaw` binary so Reborn boot, config, state, and runtime composition can evolve without accidentally invoking v1 runtime paths.

This binary is available as the workspace package `ironclaw_reborn_cli` and builds the executable named `ironclaw-reborn`.

## Current status

`ironclaw-reborn` is an early operator/testing surface, not the default IronClaw runtime.

It currently supports:

```bash
ironclaw-reborn --help
ironclaw-reborn completion --shell bash
ironclaw-reborn completion --shell zsh
ironclaw-reborn doctor
ironclaw-reborn hooks list
ironclaw-reborn hooks list --json
ironclaw-reborn hooks list --verbose
ironclaw-reborn run
```

It intentionally does not yet support:

- replacing `ironclaw` behavior;
- daemon/service installation;
- web gateway/UI startup;
- v1 config, DB, settings, or secrets migration;
- production extension/tool execution;
- long-lived Reborn runtime services.

## Commands

### `completion`

Generates shell completion scripts without resolving Reborn home, reading v1 state, or creating directories.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- completion --shell zsh > ironclaw-reborn.zsh
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- completion --shell bash > ironclaw-reborn.bash
```

The zsh output keeps the v1 CLI guard around `compdef` so the generated script is safe when zsh completion functions are not loaded yet.

### `doctor`

Validates and reports Reborn boot configuration without creating state directories or starting runtime services.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- doctor
```

Expected fields include:

- `reborn_home`
- `home_source`
- `profile`
- `v1_state: not-used`
- `driver_registry: initialized`

### `hooks list`

Reports configured Reborn hooks without resolving Reborn home, reading v1 hook config, or creating directories.

The Reborn hook registry is not wired yet, so the command currently reports an explicit empty surface:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list --verbose
```

Expected fields include:

- `configured: 0`
- `status: not-wired`
- `v1_state: not-used`

### `run`

Initializes the minimal Reborn runtime shell and exits successfully. This is a smokeable composition shell, not full agent execution.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run
```

Expected fields include:

- `binary: ironclaw-reborn`
- `version`
- `reborn_home`
- `home_source`
- `profile`
- `v1_state: not-used`
- `driver_registry: initialized`
- `runtime_shell: initialized`

## State and config root

Reborn must not use the current v1 IronClaw state root by default.

Home resolution precedence:

1. `IRONCLAW_REBORN_HOME`
2. `~/.ironclaw/reborn`

The resolver rejects unsafe or misleading homes, including empty paths, relative paths, filesystem root, parent-directory components, and known v1 state-root aliases such as `$HOME/.ironclaw` or `IRONCLAW_BASE_DIR`.

## Profiles

Use `IRONCLAW_REBORN_PROFILE` to select the boot profile.

Supported values:

- `local-dev` (default)
- `production`
- `migration-dry-run`

Example:

```bash
IRONCLAW_REBORN_HOME="$PWD/.reborn-home" \
IRONCLAW_REBORN_PROFILE=production \
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- doctor
```

## Local smoke checks

Run these before changing Reborn CLI behavior:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_reborn_cli
cargo test -p ironclaw_reborn_config
cargo test -p ironclaw_architecture reborn
cargo clippy -p ironclaw_reborn_cli --all-targets -- -D warnings
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- --help
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- completion --shell zsh >/tmp/ironclaw-reborn.zsh
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list
IRONCLAW_REBORN_HOME="$(mktemp -d)/reborn-home" \
  cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run
```

## Adding commands

Future commands should follow the crate-local agent contract in:

```text
crates/ironclaw_reborn_cli/AGENTS.md
```

Short version:

1. add one command module under `crates/ironclaw_reborn_cli/src/commands/`;
2. register it in `commands::Command`;
3. resolve and pass `RebornCliContext` from dispatch only when the command needs boot config;
4. keep pure commands independent from Reborn home resolution;
5. add a binary smoke test through `env!("CARGO_BIN_EXE_ironclaw-reborn")`;
6. avoid v1 runtime imports and v1 state mutation unless explicitly scoped and guarded.

Do not port the current `src/cli/*` command tree wholesale. Port commands one at a time, starting with Reborn-owned or read-only surfaces.

## Release packaging decision

`ironclaw-reborn` is **not yet included in cargo-dist release artifacts**.

Current `dist plan --output-format=json` with `crates/ironclaw_reborn_cli` marked `dist = false` emits only the root `ironclaw` package artifacts. Removing `dist = false` alone is not enough to ship `ironclaw-reborn` in the existing `ironclaw-v*` release workflow because that workflow is shaped around the root `ironclaw` package tag. Enabling a standalone `ironclaw_reborn_cli` release also requires cargo-dist WiX metadata/template work and an explicit tag/versioning decision.

Follow-up issue: #3483 tracks packaging `ironclaw-reborn` in release artifacts.

Until #3483 is resolved, keep:

```toml
[package.metadata.dist]
dist = false
```

in `crates/ironclaw_reborn_cli/Cargo.toml` so releases do not silently claim to ship an unverified Reborn binary package.
