# Agent Map — ironclaw_hooks_postgres

## Start Here

- No crate-local `CLAUDE.md` exists yet; use this map plus the contracts below.
- Read `src/lib.rs` first — it documents why this is a separate crate, the
  dual-backend split (Postgres here, libSQL in `ironclaw_hooks_libsql`, parity
  in `ironclaw_hooks_parity`), and the shared two-table typed schema.
- Read `Cargo.toml` for actual dependencies and the `postgres` feature gate.
- The trait contract this crate implements lives in
  `ironclaw_hooks::predicate_state` (`PredicateStateBackend`); the shared
  contract harness is exposed via that crate's `contract-tests` feature.

## What This Crate Owns

- The durable PostgreSQL `PredicateStateBackend` implementation: the atomic
  record-and-read transaction body, per-key advisory-lock serialization, the
  fail-closed per-key sample cap, and the per-scope LRU quota enforcement.
- The Postgres predicate-state schema (`hooks_predicate_invocations` /
  `hooks_predicate_values`) and its migration SQL.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- The `PredicateStateBackend` trait itself, the in-memory backend, or the
  libSQL backend (those live in `ironclaw_hooks` / `ironclaw_hooks_libsql`).
- Evaluator policy, hook-framework wiring, or backend selection.
- Secrets, raw connection strings, host names, schema details, or raw DB
  error text in errors, events, logs, or docs — `PredicateBackendError`
  payloads are sanitized (`DB_UNAVAILABLE_MSG` and the quota fail-closed
  constants); keep the raw error behind `tracing` only.

## Validation

- Fast local check: `cargo test -p ironclaw_hooks_postgres`
- Postgres-backed integration/adversarial tests are env-gated on
  `IRONCLAW_HOOKS_POSTGRES_URL` / `DATABASE_URL` and skip (passing) when no
  DB is reachable; run them against a live Postgres to exercise the advisory
  locks, cap fail-closed, and LRU eviction paths.
- If production persistence behavior changes, keep PostgreSQL/libSQL parity:
  update the libSQL counterpart (`ironclaw_hooks_libsql`) and the
  cross-backend parity suite (`ironclaw_hooks_parity`) in lockstep.

## Agent Notes

- Keep edits inside this crate unless the trait contract in `ironclaw_hooks`
  explicitly requires a neighboring crate change.
- The advisory-lock protocol is load-bearing: same-key writers serialize on a
  per-key `(int4,int4)` lock, scope-quota passes serialize on a per-scope
  `(int8)` lock folded with a kind byte, and victim eviction uses the
  NON-blocking `pg_try_advisory_xact_lock` to stay deadlock-free. Do not change
  lock derivation, isolation level, or fail-closed posture without updating the
  module-level docs and the disjointness/lock-equality unit tests.
- Cap and quota are fail-closed by contract: overflow returns
  `WindowOverflow`, an unenforceable scope quota returns `Unavailable`. Never
  silently drop-oldest or commit an over-quota scope.
