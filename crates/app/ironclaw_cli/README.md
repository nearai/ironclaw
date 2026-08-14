# ironclaw (the CLI crate)

The shipped binary — the one directory whose name and package name differ:
the directory is `crates/app/ironclaw_cli`, the **package and binary are
`ironclaw`** (PROPOSAL §5.1). It owns the command surface, the serve-loop
sequence (assemble a deployment, obtain a product surface, start the web
gateway), the binding tables that link concrete extension packages, a small
set of first-party registrars, credential-visibility policy, and the
administrative token minter. It is the leaf of every dependency chain: nothing
in the workspace imports it.

- **Family / layer:** `app` / `app` · **Package:** `ironclaw` (binary
  `ironclaw`) · **Manifest:** `crates/app/ironclaw_cli/Cargo.toml`
- **Use this when:** adding a command, changing serve wiring, linking a
  concrete extension package, or changing what the binary registers.
- **Don't use this when:** the change is domain behavior — every command is a
  thin caller into `ironclaw_composition` or a family crate, never a
  reimplementation; and never construct a wiring path composition could
  instead be asked to build.

## Public surface

None consumed in the workspace — it is a binary. Its own structure:

- `src/cli.rs` — the clap root; `src/commands/` — one command per file,
  dispatched through `Command::execute`; `src/context.rs` —
  `RebornCliContext` boot state.
- `src/commands/serve.rs` — `ironclaw serve`, driving
  `ironclaw_composition::build_reborn_runtime` and
  `ironclaw_webui::serve_webui_v2`.
- `src/runtime/native_extensions.rs` — the sanctioned concrete-extension
  binding table (the only place `ironclaw_slack_extension` /
  `ironclaw_telegram_extension` are linked), supplied to composition via
  `with_native_extension_factories` / `with_channel_extension_bindings`.
- The `AdminApiTokenMinter` implementation (`SignedSessionTokenMinter` in
  `src/commands/serve.rs`) — the sanctioned inversion of the port composition
  defines.

## Depends on / consumed by

- **Normal workspace deps (14):** `ironclaw_composition`, `ironclaw_config`,
  `ironclaw_operator`, `ironclaw_webui`, `ironclaw_auth`,
  `ironclaw_host_api`, `ironclaw_product_contracts`,
  `ironclaw_extension_contracts`, `ironclaw_trace_commons`, and — uniquely in
  the workspace — the concrete extension packages and hosting crates the
  binary links directly: `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_extension_support`,
  `ironclaw_slack_extension`, `ironclaw_telegram_extension`. Re-derive with
  `grep -n '^ironclaw' crates/app/ironclaw_cli/Cargo.toml`.
- **Consumed by:** nothing (asserted — the dependency set is pinned
  *exactly* by `assert_workspace_deps_exactly` for package `ironclaw` in
  `reborn_dependency_boundaries.rs`; a new dependency is a reviewed gate
  change).

## Invariants

- **Only the binary names a concrete extension package** — composition
  receives bindings as opaque, pre-built handles.
- **`main` stays a thin bootstrap**:
  `reborn_composition_boundaries.rs::reborn_binary_main_is_thin_bootstrap`.
- **`AGENTS.md` beside this file is gate-pinned**:
  `reborn_dependency_boundaries.rs` requires it to exist and to keep the
  phrases "one command per file", "RebornCliContext", and "no v1 runtime
  imports" — and asserts the one-command-per-file layout by path.
- Commands that need boot config receive `RebornCliContext` from dispatch;
  pure commands must not force Reborn home resolution.

## Tests

```bash
cargo test -p ironclaw                     # incl. tests/smoke.rs binary smoke tests
cargo test -p ironclaw_architecture_tests reborn
cargo clippy -p ironclaw --all-targets -- -D warnings
```

## See also

Working rules (command layout, adding a command, the serve subcommand):
`AGENTS.md` · family rules: `crates/app/AGENTS.md` · design record:
`docs/internal/reborn/target-architecture/families/app.md` (§6.10.2).
