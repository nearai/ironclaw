# Agent Map - ironclaw_triggers

## Start Here

- Read `Cargo.toml` for backend feature shape.
- Read `src/lib.rs` for trigger domain contracts and repository traits.
- Use `docs/internal/reborn/contracts/triggers.md` as the source of truth before changing behavior.

## What This Crate Owns

- Trigger records, schedule validation, source-provider evaluation, deterministic fire identity, and repository contracts.
- Trusted-trigger prompt safety (`src/prompt_safety.rs`). Minting the sealed
  `TrustedTriggerSubmitRequest` runs the injection scan, so "the prompt passed"
  is an invariant of the type rather than a step some submitter performs. Keep
  it there: a scan that lives in one `TrustedTriggerFireSubmitter` impl is lost
  the moment a second impl exists (moved here 2026-08-04, PROPOSAL §6.4.2).
  Severity policy stays in `ironclaw_safety`; this crate owns only the scanner
  instance and the mapping to `TriggerError::InvalidMaterialization`.
- In-memory test behavior and durable trigger repository backends.
- Deterministic poller tick logic behind trigger-owned repository/materializer/submitter/state-lookup ports.
- Cron validation, including rejection of schedules that can fire more often than once per minute, and rejection of invalid IANA timezone strings.
- Backend-specific trigger repository implementations may accept already-open database handles such as `Arc<libsql::Database>`.
- This crate must not own database URL/path/env parsing, bootstrap config, or generic database accessors.

## Do Not Move In Here

- Poller lifecycle, background worker startup/shutdown, routine bridges, or composition wiring.
- First-party trigger capabilities such as create/list/remove.
- Trusted inbound turn wiring, product adapter behavior, or outbound delivery resolution.
- libSQL/PostgreSQL handle construction, connection-string validation, production substrate selection, or shared Reborn database bootstrap.
- Composition/bootstrap owns those boundaries and passes typed handles into repository constructors.

## Validation

- Fast local check: `cargo test -p ironclaw_triggers`
- Lint check: `cargo clippy -p ironclaw_triggers --all-targets --all-features -- -D warnings`
- Boundary check after dependency changes: `cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold`

## Agent Notes

- Fire identity is deterministic from `(tenant_id, trigger_id, fire_slot)`; do not add a separate fire-id ledger for replay/idempotency.
- A prompt rejected by the safety scan is a **permanent** failure: the same durable prompt would fail identically on retry, so the poller advances the slot instead of re-firing.
- `TriggerRepository` and `TriggerSourceProvider` are the extension points; use them instead of cross-crate shortcuts.
- Preserve tenant/trigger scoping in every repository operation, including global due queries.
- Validate records at repository boundaries and keep focused tests for schedule, identity, round-trip, due-query, and scoped remove behavior.
