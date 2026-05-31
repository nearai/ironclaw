# Agent Map - ironclaw_triggers

## Start Here

- Read `Cargo.toml` for backend feature shape.
- Read `src/lib.rs` for trigger domain contracts and repository traits.
- Use `docs/reborn/contracts/triggers.md` as the source of truth before changing behavior.

## What This Crate Owns

- Trigger records, schedule validation, source-provider evaluation, deterministic fire identity, and repository contracts.
- In-memory test behavior and durable trigger repository backends.
- Cron validation, including rejection of schedules that can fire more often than once per minute.

## Do Not Move In Here

- Poller lifecycle, worker startup/shutdown, routine bridges, or composition wiring.
- First-party trigger capabilities such as create/list/remove.
- Trusted inbound turn wiring, product adapter behavior, or outbound delivery resolution.
- Active-run reservation/back-pressure enforcement; later poller slices own that policy.

## Validation

- Fast local check: `cargo test -p ironclaw_triggers`
- Backend check: `cargo test -p ironclaw_triggers --features libsql`
- Lint check: `cargo clippy -p ironclaw_triggers --all-targets --all-features -- -D warnings`
- Boundary check after dependency changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`

## Agent Notes

- Fire identity is deterministic from `(tenant_id, trigger_id, fire_slot)`; do not add a separate fire-id ledger for replay/idempotency.
- `TriggerRepository` and `TriggerSourceProvider` are the extension points; use them instead of cross-crate shortcuts.
- Preserve tenant/trigger scoping in every repository operation, including global due queries.
- Validate records at repository boundaries and keep focused tests for schedule, identity, round-trip, due-query, and scoped remove behavior.
