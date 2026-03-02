# Database Standard (AI Agents)

## Dual-backend rule (mandatory)

Any new persistence operation must support both configured backends.

## Required workflow

1. Add method to `Database` trait (`src/db/mod.rs`) when introducing new persistence behavior.
2. Implement method in PostgreSQL backend (`src/db/postgres.rs`).
3. Implement method in libSQL backend (`src/db/libsql_backend.rs` and related modules).
4. Validate with feature-matrix checks:
   - `cargo check`
   - `cargo check --no-default-features --features libsql`
   - `cargo check --all-features`

## Schema translation checklist

When adding/modifying schema behavior across backends:

- Index parity verified
- Seed-data parity verified
- SQL semantic differences documented where behavior differs

## Prohibited

- Backend-specific features without guardrails or documented behavior differences.
