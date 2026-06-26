# ironclaw_product_workflow_storage

Durable storage adapters for product workflow ports.

## Purpose

- Provide libSQL and PostgreSQL-backed implementations of
  `ironclaw_product_workflow::IdempotencyLedger`.
- Persist product inbound action reservations and terminal outcomes through
  `ironclaw_filesystem::RootFilesystem`.
- Preserve recovery-lease behavior for non-terminal reservations so retries do
  not dispatch the same side effect concurrently.
- Provide libSQL and PostgreSQL-backed scoped lifecycle installation stores for
  admin-shared and user-private package records.

## Boundaries

- This crate owns storage adapters only. Product workflow orchestration,
  ownership rules, and effective lifecycle resolution remain in
  `ironclaw_product_workflow`.
- Keep durable records behind the existing product workflow ports; do not add
  product workflow call paths around those traits.
- Use typed host and workflow values internally. Convert strings at boundaries.
- Keep libSQL and PostgreSQL behavior in parity when changing persistence
  semantics.

## Validation

Run targeted checks from the workspace root:

```bash
cargo test -p ironclaw_product_workflow_storage --features libsql
cargo test -p ironclaw_product_workflow_storage --features postgres --no-run
cargo check -p ironclaw_product_workflow_storage --features "libsql postgres"
cargo clippy -p ironclaw_product_workflow_storage --all-targets --features "libsql postgres" -- -D warnings
```

PostgreSQL runtime tests require `IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL`; when
it is unset, postgres contract tests compile and skip execution.
