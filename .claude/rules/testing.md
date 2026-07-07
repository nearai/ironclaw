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

## Integration-First Coverage (Reborn)

New or changed Reborn behavior that is production-wired ships with a
test in `tests/integration/` (authoring guide:
`tests/integration/CLAUDE.md`) — driven through the harness and
asserting at a seam (captured model request, egress recorder, store
reopen), not `wait_for_status(Completed)` alone.

- **Tier fallback:** crate-tier tests are acceptable only when the
  integration tier cannot reach the path; state why in the PR.
- **Don't wire the unwired:** never add test-only wiring for paths
  production doesn't wire — defer the scenario instead. A harness seam
  must cite the production call site it mirrors or exposes.
- **No skipped tests:** no `#[ignore]`d or TODO-pinned tests; every
  landed test runs and passes in CI. RED regression pins belong in the
  fix PR that turns them green.
- **Consolidate:** extend an existing suite before creating a new one
  (see CLAUDE.md Testing Discipline rule 2).
- **Coverage ratchet:** Reborn integration-tier coverage is also gated
  by a committed ratchet floor
  (`tests/integration/coverage-floor.toml`) — dry-run only until the
  post-#5656 recapture PR sets `enforce = true`; see the floor file's
  own header for the same-PR floor-raise workflow.

## Key Patterns

- Unit tests in `mod tests {}` at the bottom of each file
- Async tests with `#[tokio::test]`
- No mocks, prefer real implementations or stubs
- Use `tempfile` crate for test directories, never hardcode `/tmp/`
- Regression test with every bug fix (enforced by commit-msg hook)
- Integration tests (`--test workspace_integration`) require PostgreSQL; skipped if DB is unreachable

## Test Through the Caller, Not Just the Helper

**When a helper gates a side-effecting flow, the test must go through the caller — not just the helper in isolation.**

A whole class of bugs in this repo has the same shape: a wrapper function silently loses one of its inputs, and the unit test for the helper passes because it never crosses the layer where the input gets dropped.

Real examples (do not let these recur):

| Bug | Helper | What got lost | How a caller-level test would have caught it |
|-----|--------|--------------|------------------------------------------------|
| nearai/ironclaw#1948 | `McpServerConfig::has_custom_auth_header()` | Helper existed but `requires_auth()` never consulted it, so MCP triggered OAuth/DCR even with a user-set `Authorization` header | A test driving `mcp::factory::create_client_from_config()` with a header-bearing config and asserting zero OAuth-state side effects |
| nearai/ironclaw#1921 | `derive_activation_status(ext, has_owner_binding)` | Wrapper hardcodes the underlying classifier's `has_paired` axis to `false`, even though `classify_wasm_channel_activation` takes both bools | A test driving `extensions_list_handler` against a DB with a real `channel_identities` row and asserting `Active`, not `Pairing` |
| nearai/ironclaw#1502 | `window.open` mock `(url) => { window._lastOpenedUrl = url }` | Mock captured only the URL, silently swallowing `target` and `windowFeatures`; a regression to same-tab open would not fail | A mock capturing all three args plus an assert that `target === '_blank'` |

### When the rule applies

You must add a caller-level test (not just a helper-level unit test) when **all** of the following are true:

1. The helper is a **predicate, classifier, or transform** whose return value gates a side effect (HTTP call, DB write, UI mutation, OAuth flow, secret read, tool execution, sandbox launch, etc.).
2. There is **at least one wrapper or call site** between the helper and the side effect.
3. The helper has **more than one input** *or* its caller computes any of the inputs from the surrounding context.

If all three are true, a unit test on the helper alone is **not sufficient regression coverage**. You must additionally either:

- Add a test that drives the call site (`*_handler`, `factory::create_*`, `manager::*`), **or**
- Inline the helper into its single caller so there is no wrapper to silently drop an input.

### Where the test belongs

Most of these gaps are above unit-test scope and below e2e scope. Default to the **integration tier** (`cargo test --features integration`):

- `tests/<module>_integration.rs` for Rust integration tests against the public handler/factory surface
- `tests/multi_tenant_integration.rs` when the lost axis is per-user state
- `tests/e2e/scenarios/test_*.py` when the lost axis is browser-visible

Unit tests in `mod tests {}` are still fine for the helper itself, but they do not satisfy this rule.

### Mock hygiene corollary

When you mock a browser/runtime API in a test, the mock's signature must match the production call site's signature, and assertions should cover **every argument** the production code passes. A `(url) => {}` stub for a `window.open(url, target, features)` call site is a silent argument-loss bug waiting to happen.
