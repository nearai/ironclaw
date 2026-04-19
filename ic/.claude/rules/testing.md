---
paths:
  - "src/**/*.rs"
  - "tests/**"
---
# Testing Rules

## Test Tiers

| Tier | Command | External deps |
|------|---------|---------------|
| Unit | `cargo test` | None |
| Integration | `cargo test --features integration` | Running PostgreSQL |
| Live | `cargo test --features integration -- --ignored` | PostgreSQL + LLM API keys |

Run `bash scripts/check-boundaries.sh` to verify test tier gating.

## Key Patterns

- Unit tests in `mod tests {}` at the bottom of each file
- Async tests with `#[tokio::test]`
- No mocks, prefer real implementations or stubs
- Use `tempfile` crate for test directories, never hardcode `/tmp/`
- Regression test with every bug fix (enforced by commit-msg hook)
- Integration tests (`--test workspace_integration`) require PostgreSQL; skipped if DB is unreachable
