# Testing Standard (AI Agents)

## Required

- Add or update tests for behavior-changing code.
- Keep unit tests close to implementation (`mod tests {}`) when that is the local convention.
- Use `#[tokio::test]` for async behavior.
- Prefer real implementations or lightweight stubs over complex mocks.
- Run lint checks and address warnings before committing (`cargo clippy --all --benches --tests --examples --all-features`).

## Verification commands

Run relevant checks before considering work complete:

- `cargo fmt`
- `cargo check`
- `cargo test` (or targeted tests when iterating)

For feature-gated changes, also run:

- `cargo check --no-default-features --features libsql`
- `cargo check --all-features`

## Test quality

- Assert externally visible behavior, not internal implementation details.
- Keep tests deterministic and avoid timing-sensitive flakiness where possible.
