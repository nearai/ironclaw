# ironclaw_reborn_migration

Standalone Reborn operator migration crate. It is legacy-free: do not add a
dependency on the retired root `ironclaw` package or recreate a v1 state import
path here.

## Current scope

- Ships `ironclaw-reborn-extension-ownership-migration`.
- Opens the Reborn target store through `RootFilesystem` backends.
- Rewrites installed extension ownership to the full tenant user set, plus any
  explicit `--include-user` bootstrap users.
- Supports dry-run output for operator review before writing state.

## Commands

```bash
cargo run -p ironclaw_reborn_migration --bin ironclaw-reborn-extension-ownership-migration -- \
  --target-libsql ./reborn-local-dev.db \
  --tenant-id default \
  --include-user default \
  --dry-run

cargo test -p ironclaw_reborn_migration --no-default-features --features libsql extension_ownership
```

## Guardrails

- Keep this crate as an operator utility over Reborn stores, not a serving path.
- Do not depend on `ironclaw`, `ironclaw_gateway`, `ironclaw_tui`, or any other
  retired v1 surface.
- Use `ironclaw_reborn_composition` migration-support seams for store
  construction when composition owns the store shape.
- Keep reports auditable and identifier-only; do not print credentials or
  profile contents.
