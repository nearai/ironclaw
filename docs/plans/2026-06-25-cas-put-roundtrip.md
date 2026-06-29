# CAS put: fold directory pre-check into the write statement (3 → 1 round-trip)

Date: 2026-06-25
Branch: `fix/reborn-cas-put-roundtrip`
Crate: `ironclaw_filesystem`

## Problem (verified)

Every CAS `put` on the Postgres backend issues **3 DB round-trips on the happy
path**, not 1. `postgres_put_with_client`
(`crates/ironclaw_filesystem/src/postgres.rs:1081`) runs a directory pre-check
on a single held connection before the write:

1. `postgres_exact_entry_with_client` — `SELECT … WHERE path = $1`
   (rejects if an explicit directory exists at the exact path).
2. `postgres_has_child_entry_with_client` — `SELECT 1 … WHERE path >= $lower AND
   path < $upper LIMIT 1` (rejects if an implicit-directory child exists).
3. The `INSERT … ON CONFLICT` / `UPDATE` itself.

On the error path a 4th query (`postgres_current_version_with_client`) builds the
`VersionMismatch`. The pre-check (1 + 2) runs for **every** `CasExpectation`
variant (`Absent` / `Version` / `Any`).

At ~100–200 ms/round-trip cross-region, that is 300–600 ms per write, and a turn
issues dozens of CAS puts → latency cascade and pooled-connection hold time that
starves the small hosted pool.

### Backend parity baseline (verified)

- **in-memory** (`in_memory.rs:84`): whole `put` runs under one `tokio::Mutex`
  guard, no await points — atomic, zero round-trips. Directory invariant
  (implicit-child prefix scan) + CAS (`check_cas`) enforced in-proc. **No change.**
- **libsql** (`libsql.rs:142`): same 3-round-trip (4 for `Any`) shape as
  Postgres, but in this crate the connection is `Builder::new_local` (embedded
  SQLite, sub-ms in-process). Remote Turso is only wired at the composition
  layer. The cross-region latency cascade is the **Postgres production** path.
  **No behavior change required; semantics stay identical.** (Lane note: libsql
  `put` is left as-is to keep this PR minimal and Postgres-scoped; its
  round-trip count is not a production latency concern because the production
  RootFilesystem store is Postgres.)

## Fix (Postgres only, minimal)

Fold the directory pre-check into each CAS arm's single write statement using a
`NOT EXISTS` child-existence guard plus the existing `is_dir = FALSE` guard, so
the **happy path is 1 round-trip**.

- `Absent`: `INSERT … SELECT … WHERE NOT EXISTS (child-scan) ON CONFLICT (path)
  DO NOTHING`. The child-scan uses the half-open `[prefix/, prefix0)` range. If
  the target path is itself an explicit directory, the existing row has
  `is_dir = TRUE`; `ON CONFLICT DO NOTHING` writes 0 rows, same as a CAS clash —
  disambiguated on the (rare) 0-row error path.
- `Version`: `UPDATE … WHERE path = $p AND is_dir = FALSE AND version = $v`.
  An explicit directory at `path` has `is_dir = TRUE` → 0 rows. A child existing
  does not change that the target is a file row; but a `Version` CAS implies the
  file already exists, so a child under a *file* path is impossible by
  construction. We still add the `NOT EXISTS` child guard for defense-in-depth
  parity with `Absent`/`Any`.
- `Any`: `INSERT … SELECT … WHERE NOT EXISTS (child-scan) ON CONFLICT (path) DO
  UPDATE … WHERE root_filesystem_entries.is_dir = FALSE RETURNING version`.
  `RETURNING` removes the separate version read-back on the happy path.

### Error-path disambiguation (rare, not the hot path)

When the single statement affects 0 rows (or `RETURNING` yields nothing) we make
follow-up reads to decide between `directory_write_error` and `VersionMismatch`
— exactly the information the old pre-check produced, but only when the write
actually failed. This is consolidated into one `diagnose_put_failure` helper
shared by the `Absent` and `Version` arms (`Any` cannot version-mismatch, so it
returns `directory_write_error` directly). The directory check uses a new
`postgres_is_dir_with_client` that reads only the `is_dir` flag — it does not
read `OCTET_LENGTH(contents)`, so it never touches the (possibly TOAST'd) body.
The now-unused `postgres_exact_entry_with_client` free function is removed. The
happy path never pays for any of this.

Each folded `NOT EXISTS` child scan carries `LIMIT 1`, matching
`postgres_has_child_entry_with_client`, so the planner can use a limit-aware
plan when a directory has many children.

### Invariants preserved

- Reject writing a file where an explicit directory exists (`is_dir = TRUE`).
- Reject writing a file where an implicit-directory child exists (child range).
- CAS semantics (`Absent` / `Version(expected)` / `Any`) unchanged.

## Red → green / perf proof

This is a round-trip **count** fix, not a wrong-result fix. Proof strategy:

1. **Correctness (no regression):** keep all existing directory-invariant + CAS
   contract tests green, and add a `put`-level directory-rejection unit test for
   the SQL fold (explicit-dir + implicit-child) that runs against the in-memory
   path of the contract where possible; the Postgres SQL is exercised by the
   `--features postgres` contract build + any live Postgres test harness.
2. **Round-trip reduction:** show the happy-path SQL now issues **one**
   statement per CAS arm (was three: exact-entry + child-scan + write). Evidence
   = the diff (two pre-check helper calls removed from the happy path) + a
   statement-count assertion where a countable seam exists.

## Quality gate

- `cargo fmt --all`
- `cargo clippy -p ironclaw_filesystem --all-targets` (0 warnings)
- `cargo test -p ironclaw_filesystem`
- `cargo test -p ironclaw_filesystem --features postgres,libsql` (contract)

## File lane

Owned: `postgres.rs` `postgres_put_with_client` + its pre-check helpers (only
where they stop being used by the put happy path), contract tests. **Not
touched:** `append_file` / event-log path, `ironclaw_resources`, store crates,
`cas.rs`.
