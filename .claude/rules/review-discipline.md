---
paths:
  - "src/**/*.rs"
---
# Review & Fix Discipline

Hard-won lessons from code review -- follow these when fixing bugs or addressing review feedback.

**Fix the pattern, not just the instance:** When a reviewer flags a bug (e.g., TOCTOU race in INSERT + SELECT-back), search the entire codebase for all instances of that same pattern. A fix in `SecretsStore::create()` that doesn't also fix `WasmToolStore::store()` is half a fix.

**Propagate architectural fixes to satellite types:** If a core type changes its concurrency model (e.g., `LibSqlBackend` switches to connection-per-operation), every type that was handed a resource from the old model must also be updated. Grep for the old type across the codebase.

**Schema translation is more than DDL:** When translating a database schema between backends (PostgreSQL to libSQL, etc.), check for:
- **Indexes** -- diff `CREATE INDEX` statements between the two schemas
- **Seed data** -- check for `INSERT INTO` in migrations (e.g., `leak_detection_patterns`)
- **Semantic differences** -- document where SQL functions behave differently (e.g., `json_patch` vs `jsonb_set`)

**Feature flag testing:** When adding feature-gated code, test compilation with each feature in isolation:
```bash
cargo check                                          # default features
cargo check --no-default-features --features libsql  # libsql only
cargo check --all-features                           # all features
```

**Regression test with every fix:** Every bug fix must include a test that would have caught the bug. Add a `#[test]` or `#[tokio::test]` that reproduces the original failure. Exempt: changes limited to `src/channels/web/static/` or `.md` files. Use `[skip-regression-check]` in commit message or PR label if genuinely not feasible. The `commit-msg` hook and CI workflow enforce this automatically.

**Zero clippy warnings policy:** Fix ALL clippy warnings before committing, including pre-existing ones in files you didn't change. Never leave warnings behind.

**Transaction safety:** Multi-step database operations (INSERT+INSERT, UPDATE+DELETE, read-then-write) MUST be wrapped in a transaction. Never assume sequential calls are atomic. This applies to both postgres and libsql backends.

**UTF-8 string safety:** Never use byte-index slicing (`&s[..n]`) on user-supplied or external strings -- it panics on multi-byte characters. Use `is_char_boundary()` or `char_indices()`. Grep for `[..` in changed files.

**Case-insensitive comparisons:** When comparing user-supplied strings (file paths, media types, extension names), normalize to lowercase with `.to_ascii_lowercase()`. Path comparisons must be case-insensitive on macOS/Windows.

**Decorator/wrapper trait delegation:** When adding a new method to `LlmProvider` (or any trait with decorator wrappers), update ALL wrapper types to delegate. Grep for `impl LlmProvider for` to find all implementations. Test through the full provider chain.

**Sensitive data in logs & events:** Tool parameters and outputs MUST be redacted before logging or broadcasting via SSE/WebSocket. Use `redact_params()` before any `tracing::info!`, `JobEvent`, or SSE emission that includes tool call data.

**Test temporary files:** Use the `tempfile` crate. Never hardcode `/tmp/...` paths.

**Trust boundaries in multi-process architecture:** Data from worker containers is untrusted. The orchestrator MUST validate: tool domain, nesting depth (server-side tracking), and parameter sensitivity.

**Mechanical verification before committing:**
- `cargo clippy --all --benches --tests --examples --all-features` -- zero warnings
- `grep -rnE '\.unwrap\(|\.expect\(' <files>` -- no panics in production
- `grep -rn 'super::' <files>` -- prefer `crate::` for cross-module imports (`super::` OK in tests/intra-module)
- If you fixed a pattern bug, `grep` for other instances across `src/`
- Run `scripts/pre-commit-safety.sh` to catch UTF-8, case-sensitivity, hardcoded /tmp, and logging issues
