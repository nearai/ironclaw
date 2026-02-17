---
description: Deep audit of the IronClaw crate for vulnerabilities, bugs, unfinished work, inconsistencies, and oversights
disable-model-invocation: true
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo audit:*), Bash(git diff:*), Bash(git log:*), Bash(git show:*), Bash(wc:*), Read, Grep, Glob, Task
argument-hint: "[path/to/crate]"
---

# Rust Crate Audit

You are performing a thorough audit of a Rust crate. Your goal is to find every vulnerability, bug, unfinished piece of work, inconsistency, and oversight before it ships. Leave no stone unturned.

## Step 1: Locate the crate

Parse `$ARGUMENTS`:
- If a path is provided, use it as the crate root.
- If empty, use the current working directory.

Verify it's a valid Rust crate by checking for `Cargo.toml`. If not found, stop and ask the user.

## Step 2: Understand the crate

Read `Cargo.toml` to understand:
- Crate name, version, edition
- Dependencies (look for outdated, unmaintained, or suspicious crates)
- Feature flags and their implications
- Build scripts (`build.rs`) if any

Read `CLAUDE.md`, `README.md`, or top-level documentation if present to understand intent and architecture.

Read `src/lib.rs` or `src/main.rs` to get the module tree. Then read each module's `mod.rs` or top-level file to build a mental map of the crate's structure before diving into details.
Read all Rust files (`src/*.rs`) to make sure everything is in context when you are reasoning.

## Step 3: Run the compiler's checks

Run these commands and capture output. Do NOT fix anything, just collect findings:

```
cargo fmt --check 2>&1
```

```
cargo clippy --all --benches --tests --examples --all-features -- -W clippy::all -W clippy::pedantic -W clippy::nursery 2>&1
```

```
cargo test --lib 2>&1
```

If any of these fail, record the failures as findings. If `cargo test` has ignored tests, note which ones and why.

Note: Integration tests (`--test workspace_integration`) require a PostgreSQL database and are expected to fail locally. Only report `--lib` test failures as blocking.

## Step 4: Scan for unfinished work

Search the entire `src/` tree for:

```
todo!
unimplemented!
fixme
FIXME
TODO
HACK
XXX
SAFETY:
stub
placeholder
temporary
```

For each match:
- Is it in production code or test code?
- Is it a genuine incomplete feature or a deliberate placeholder?
- Is there a tracking issue referenced?
- Could this panic at runtime?

Any `todo!()` or `unimplemented!()` in non-test code is **High severity** (runtime panic).

## Step 5: Audit for vulnerabilities and unsafe code

### 5a. Unsafe code

Search for all `unsafe` blocks. For each one:
- Is the safety invariant documented with a `// SAFETY:` comment?
- Is the invariant actually upheld by the surrounding code?
- Could the unsafe block be replaced with a safe alternative?
- Are there any pointer dereferences, transmutes, or FFI calls?

### 5b. Unwrap and panic paths

Search for `.unwrap()`, `.expect(`, `panic!`, `unreachable!` in non-test code. For each:
- Can this actually panic in production?
- Is there a code path that reaches this with None/Err?
- Should it be replaced with proper error handling (`?`, `.ok()`, `.unwrap_or_default()`)?

IronClaw convention: `.unwrap()` and `.expect()` are banned in production code. Any occurrence outside `#[cfg(test)]` blocks is a **High severity** finding.

### 5c. SQL and injection vectors

Search for string formatting used in SQL queries, shell commands, or HTML:
- `format!` used near `.execute(`, `.query(`, `Command::new(`
- String interpolation in query construction vs parameterized queries
- User input flowing into file paths (`Path::new`, `std::fs::`)

IronClaw has two database backends (PostgreSQL and libSQL). Check both for injection vectors.

### 5d. Cryptographic issues

If the crate uses crypto:
- Are comparisons constant-time? (look for `==` on secrets/hashes vs `subtle::ConstantTimeEq`)
- Is randomness from `OsRng` / `thread_rng` and not a fixed seed?
- Are keys/secrets zeroized after use? (`secrecy`, `zeroize` crates)
- Are deprecated algorithms used? (MD5, SHA1 for security, RC4, DES)

### 5e. Resource exhaustion

- Are there unbounded allocations? (`Vec` growing from user input without limits)
- Are there unbounded loops? (retry loops without max attempts)
- Are file reads bounded? (`std::fs::read_to_string` on user-provided paths)
- Are timeouts set on all network operations?
- Are there connection/resource leaks? (opened but never closed, missing `Drop`)

### 5f. Error handling

- Are errors swallowed silently? (`let _ = ...`, `.ok()` discarding errors that matter)
- Do error types carry enough context to debug in production?
- Are there error type mismatches? (returning generic `anyhow::Error` where a typed error would prevent confusion)
- Is `thiserror` used consistently for error types (IronClaw convention)?

## Step 6: Check for inconsistencies

### 6a. Naming conventions

- Are types, functions, modules named consistently? (e.g., mixing `get_` and `fetch_`, `create_` and `new_`)
- Do similar operations follow the same patterns?

### 6b. Duplicate or near-duplicate code

Look for:
- Functions that do nearly the same thing with minor variations (candidates for generics or shared helpers)
- Repeated error mapping patterns that should be extracted
- Copy-pasted SQL queries or string templates with slight differences
- Identical struct definitions or conversion logic in different modules

### 6c. API consistency

- Do similar functions take arguments in the same order?
- Are return types consistent? (e.g., some functions return `Option<T>`, similar ones return `Result<T, E>`)
- Are visibility modifiers consistent? (`pub` where it should be `pub(crate)`, or vice versa)

### 6d. Dead code and unused items

- Are there functions, structs, or modules that nothing references?
- Are there `#[allow(dead_code)]` annotations that should be investigated?
- Are there feature-gated items where the feature is never enabled?

### 6e. Import style

IronClaw convention: use `crate::` imports, not `super::`. Flag any `super::` imports in non-test code.

## Step 7: Inspect for change oversights

### 7a. Partial refactors

- Are there old patterns coexisting with new patterns?
- Are there renamed types/functions where some call sites still use the old name via a compatibility alias?
- Are there comments referencing behavior that no longer exists?

### 7b. Trait implementation gaps

- If a trait is defined, do all intended types implement it?
- Are there `impl` blocks that look incomplete?
- Are `Default` implementations sensible?

IronClaw key traits: `Database` (~60 methods), `Channel`, `Tool`, `LlmProvider`, `SuccessEvaluator`, `EmbeddingProvider`. If any new methods were added to `Database`, verify both `postgres.rs` and `libsql_backend.rs` implement them.

### 7c. Test coverage gaps

- Are there public functions without any test?
- Are there error paths without tests?
- Are there recently-changed functions where the tests still assert old behavior?

### 7d. Documentation drift

- Do doc comments match actual function behavior?
- Are examples in doc comments still valid and compilable?

## Step 8: Dependency audit

Review `Cargo.toml` and `Cargo.lock`:
- Are there duplicate versions of the same crate in the lock file? (potential version conflicts)
- Are there dependencies with known security advisories? Run `cargo audit` to check (install with `cargo install cargo-audit` if not present).
- Are there heavy dependencies used for trivial functionality?
- Are dependency features minimal?

## Step 9: Present findings

Compile all findings into a structured report. Group by severity, then by category.

### Format

For each finding:

```
### [Severity] Category: One-line summary

**Location:** `file_path:line_number`
**Category:** Vulnerability | Bug | Unfinished | Inconsistency | Duplicate | Oversight | Style

**Description:**
Detailed explanation of the issue, why it matters, and how it could manifest.

**Suggested fix:**
Concrete suggestion with code if applicable.
```

### Severity levels

- **Critical**: Security vulnerability, data loss, or crash in production
- **High**: Bug that causes incorrect behavior, `todo!()`/`unimplemented!()` in prod code, or missing validation on trust boundaries
- **Medium**: Inconsistency, duplicate code, incomplete error handling, missing tests for important paths
- **Low**: Naming inconsistency, unnecessary complexity, documentation drift, minor dead code
- **Nit**: Style preference, optional improvement

### Summary table

End with a summary table:

| # | Severity | Category | File:Line | Finding |
|---|----------|----------|-----------|---------|

And a final tally: X Critical, Y High, Z Medium, W Low, V Nit.

## Rules

- Read every file before reporting on it. Never guess about code you haven't seen.
- Be specific. "This might have issues" is worthless. "Line 42 calls `.unwrap()` on a `Result` that returns `Err` when the DB connection is dropped" is useful.
- Distinguish certainty levels: "this IS a bug" vs "this COULD be a bug if X".
- Don't invent problems to look thorough. If the code is solid, say so.
- Focus on substance over style. Don't flag formatting unless it causes real confusion.
- Respect existing project conventions (check CLAUDE.md). Don't flag patterns the project explicitly endorses.
- When in doubt about severity, round up.
- For large crates (>50 files), prioritize: core logic > public API > internal utilities > tests > examples.
- Use the Task tool to parallelize file reading across modules when the crate is large.
- Do NOT fix anything. This is a read-only audit. Report findings for the user to action.
