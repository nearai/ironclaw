# Development Workflows

This page covers common development tasks: fixing bugs, adding features, code review, and deployment.

## Workflow: Fixing a Bug

### 1. Understand the Bug

```bash
# Find related issues
gh issue list --search "label:bug"

# Get issue details
gh issue view <issue-number>

# Read related code
# Use the knowledge graph (see AGENTS.md: Code Discovery)
```

### 2. Write a Failing Regression Test

**Why?** Every bug fix must include a test that would have caught the bug.

```rust
// In tests/ or in the module:
#[test]
fn test_bug_scenario() {
    let input = /* the specific case that triggers the bug */;
    let result = function_with_bug(input);
    
    assert_eq!(result, /* what it should be */);  // Currently fails
}
```

Run it to verify it fails:
```bash
cargo test test_bug_scenario
# Should show: FAILED
```

### 3. Locate and Fix the Bug

```bash
# If you're not sure where the bug is, search:
cargo clippy              # Does clippy catch it?
cargo test               # What test is failing?
grep -r "bug_keyword" .  # Is there a TODO?
```

Fix the bug in the source code:

```rust
fn function_with_bug(input: &str) -> String {
    // Before: Incorrect logic
    // After: Corrected logic
}
```

### 4. Verify the Fix

```bash
# Run the regression test
cargo test test_bug_scenario
# Should show: ok

# Run all tests
cargo test

# Check formatting and linting
cargo fmt
cargo clippy -- -D warnings
```

### 5. Commit

```bash
git add .
git commit -m "fix(scope): description of the bug

Details about what was wrong and how it's fixed.
Include a reference to the issue: Fixes #123."
```

**Commit message format:**
- Type: `fix`, `feat`, `docs`, `style`, `refactor`, `test`, `chore`
- Scope: Module or area affected
- Message: What changed
- Body: Why it changed (optional but recommended)

## Workflow: Adding a Feature

### 1. Plan the Feature

```bash
# Check FEATURE_PARITY.md for tracked features
grep -i "feature-name" FEATURE_PARITY.md

# Decide where to build it:
# - New runtime behavior? → crates/ironclaw_reborn*
# - New tool? → crates/ironclaw_first_party_extensions or ironclaw_extensions
# - New gateway feature? → crates/ironclaw_gateway or ironclaw_reborn_webui_ingress
# - New channel? → crates/ironclaw_*_adapter

# Read the relevant architecture docs
# Example: Adding a capability? Read openwiki/architecture/overview.md
```

### 2. Write Tests First

```rust
// Test the feature before implementing it

#[test]
fn test_new_feature_basic_case() {
    let result = new_feature(input);
    assert_eq!(result, expected);
}

#[test]
fn test_new_feature_edge_case() {
    let result = new_feature(edge_case_input);
    assert!(result.is_ok());
}
```

Run tests to verify they fail:
```bash
cargo test test_new_feature
# Should show: FAILED (not yet implemented)
```

### 3. Implement the Feature

Add the implementation:

```rust
pub fn new_feature(input: &str) -> Result<Output> {
    // Implementation
}
```

### 4. Verify Tests Pass

```bash
cargo test test_new_feature
# Should show: ok

cargo test
# All tests should pass
```

### 5. Update Documentation

- **Feature parity:** Update [FEATURE_PARITY.md](/FEATURE_PARITY.md) with status
- **Docs:** Add docs in `/docs/` (user-facing)
- **OpenWiki:** Update [openwiki/](../) if architectural
- **Code comments:** Add doc comments (`///`) for public APIs

### 6. Commit

```bash
git add .
git commit -m "feat(scope): description of the feature

Why this feature was added. Include issue reference if applicable.

Includes tests: test_new_feature_basic_case, test_new_feature_edge_case"
```

## Workflow: Code Review

### 1. Claim the PR

```bash
# Comment on the PR
gh pr comment <pr-number> -b "Taking this for review"

# Mark as "Reviewing"
gh pr edit <pr-number> --state ready
```

### 2. Review Checklist

- [ ] **Scope:** Does the PR stay focused on one change?
- [ ] **Tests:** Are tests included and comprehensive?
- [ ] **Architecture:** Is the change in the right place?
- [ ] **Security:** Does it touch auth, secrets, or sandboxing?
- [ ] **Docs:** Are FEATURE_PARITY.md, README, and OpenWiki updated?
- [ ] **Code quality:** Is clippy clean? Is formatting correct?
- [ ] **Performance:** Are there obvious inefficiencies?

### 3. Test Locally

```bash
# Check out the PR
gh pr checkout <pr-number>

# Build it
cargo build

# Run tests
cargo test

# Test the feature manually
cargo run -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "test"
```

### 4. Comment

On GitHub:

- **"Approve"** if all checks pass
- **"Request changes"** if there are issues (provide specific feedback)
- **"Comment"** if you're still reviewing (nit-picks, questions)

### 5. Security-Sensitive Review

If the PR touches:
- **Auth:** Verify bearer token, CORS, origin checks
- **Secrets:** Verify no inline secrets; only env-var names
- **Sandboxing:** Verify isolation, resource limits
- **Approvals:** Verify leases are scoped to exact invocations
- **Network:** Verify allowlists, DNS checks

See [AGENTS.md: Security and Runtime Invariants](/AGENTS.md#security-and-runtime-invariants) for guidelines.

### 6. Merge

```bash
# When approved and CI passes:
gh pr merge <pr-number>  # --squash for single commit
```

## Workflow: Debugging Locally

### 1. Reproduce the Issue

```bash
# Set up minimal environment
export IRONCLAW_REBORN_HOME="$PWD/.reborn-debug"
export OPENAI_API_KEY="sk-..."

# Run the failing command
cargo run -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "..."
```

### 2. Add Debug Output

```rust
// In your code, use debug!() macro (not info!())
debug!("Variable: {:?}", variable);

// Run with debug logging
RUST_LOG=debug cargo run -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "..."
```

### 3. Use a Debugger

**VS Code with CodeLLDB:**

1. Set breakpoints in editor (click margin)
2. Press F5 to debug
3. Use watch panel to inspect variables

**Command-line:**

```bash
# Install LLDB (if not present)
# macOS: Included with Xcode
# Linux: sudo apt install lldb
# Windows: Use WinDbg or VS Code

# Run with debugger
rust-lldb ./target/debug/ironclaw-reborn

# Common commands:
# (lldb) breakpoint set -n function_name
# (lldb) run ... args
# (lldb) p variable_name
# (lldb) n (next)
# (lldb) c (continue)
```

### 4. Inspect State

```bash
# Check event store (local files)
cat ~/.ironclaw/reborn/events.jsonl | jq '.'

# Check PostgreSQL (if using)
psql -U ironclaw -d ironclaw -c "SELECT * FROM events LIMIT 10;"

# Check filesystem
ls -la ~/.ironclaw/reborn/
```

### 5. Test the Fix

Once you think you've fixed it, write a test:

```rust
#[test]
fn test_issue_is_fixed() {
    // Reproduce the original problem
    // Verify it's now fixed
}
```

## Workflow: Changing the Safety Layer

The safety layer is high-risk. Follow this process:

1. **Understand current behavior:** Read [crates/ironclaw_safety/CLAUDE.md](/crates/ironclaw_safety/CLAUDE.md)

2. **Write tests first:**
   ```rust
   #[test]
   fn test_new_safety_check() {
       let dangerous_input = /* attack vector */;
       let result = check_safety(dangerous_input);
       assert!(!result.is_safe);
   }
   ```

3. **Implement the check:**
   - Add detection logic to `crates/ironclaw_safety/`
   - Test on real attack vectors
   - Verify no false positives on legitimate input

4. **Review with security lens:**
   - Can this be bypassed?
   - Are there edge cases?
   - Is coverage sufficient?

5. **Update tests:**
   - Ensure safety layer tests are comprehensive
   - Run full test suite

6. **Commit:**
   ```bash
   git commit -m "feat(safety): add detection for new threat

   Detects XXX pattern. Tests added: test_*.
   Reviewed by @security-expert."
   ```

## Workflow: Adding a New Tool

1. **Design the tool:**
   - What capability does it provide?
   - What parameters does it take?
   - What are the outputs?
   - What permissions does it need?

2. **Create the manifest:**
   - Add `crates/ironclaw_first_party_extensions/assets/my_tool/manifest.toml`
   - Define schemas (`assets/my_tool/schemas/`)
   - Add prompts (`assets/my_tool/prompts/`)

3. **Implement the tool:**
   - Add WASM implementation or native code
   - Register in capability registry

4. **Add tests:**
   ```rust
   #[tokio::test]
   async fn test_my_tool_basic() {
       let result = execute_my_tool(params).await;
       assert_ok!(result);
   }
   ```

5. **Add docs:**
   - Tool prompt (guides LLM on usage)
   - Input/output schema (JSONL)

## Workflow: Deploying to Production

### 1. Prepare the Release

```bash
# Update CHANGELOG.md
# Update version in Cargo.toml
# Commit and tag

git tag v0.x.y
git push origin v0.x.y
```

### 2. Build Docker Image

```bash
# Build release image
docker build -f Dockerfile -t ironclaw:0.x.y .

# Test image
docker run ironclaw:0.x.y ironclaw-reborn --version

# Push to registry
docker push your-registry/ironclaw:0.x.y
```

### 3. Deploy

```bash
# Using Docker
docker run -d \
  -e IRONCLAW_REBORN_HOME=/data \
  -e IRONCLAW_REBORN_PROFILE=production \
  -e IRONCLAW_REBORN_POSTGRES_URL="$DB_URL" \
  -e IRONCLAW_REBORN_SECRET_MASTER_KEY="$SECRET_KEY" \
  -p 3000:3000 \
  ironclaw:0.x.y serve

# Using systemd (on Linux)
sudo systemctl restart ironclaw
```

### 4. Verify Deployment

```bash
# Health check
curl http://localhost:3000/health

# Check logs
journalctl -u ironclaw -f

# Test a simple command
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"message": "hello"}'
```

## Workflow: Refactoring

When refactoring large areas:

1. **Plan the refactor:**
   - What's changing and why?
   - What APIs are affected?
   - How will existing code adapt?

2. **Deprecate first:**
   ```rust
   #[deprecated(since = "0.x.y", note = "Use new_function instead")]
   pub fn old_function() { ... }
   ```

3. **Migrate gradually:**
   - Don't change everything at once
   - Update one caller at a time
   - Run tests after each change

4. **Update docs:**
   - Migration guide for users/developers
   - API documentation
   - OpenWiki sections

5. **Communicate:**
   - PR description explains the refactor
   - Code review focuses on correctness
   - Include link to migration guide

## Workflow: Performance Optimization

1. **Measure first:**
   ```bash
   cargo build --release
   time cargo run --release -- run --message "complex task"
   ```

2. **Profile:**
   ```bash
   # CPU profiling
   cargo install flamegraph
   cargo flamegraph --bin ironclaw-reborn

   # Memory profiling
   HEAPPROFILE=/tmp/ironclaw cargo run --release
   ```

3. **Optimize:**
   - Focus on hot paths (found via profiling)
   - Add benchmarks before and after
   - Verify it's actually faster

4. **Verify:**
   ```bash
   # Run benchmarks
   cargo bench

   # Run full tests
   cargo test
   ```

## Git Workflow

### Branch Strategy

```bash
# Create a feature branch
git checkout -b fix/issue-123-description
# or
git checkout -b feat/new-feature

# Push to origin
git push -u origin fix/issue-123-description
```

### Commit Hygiene

```bash
# Commit frequently (logical chunks)
git add specific_file.rs
git commit -m "fix(scope): small logical change"

git add another_file.rs
git commit -m "fix(scope): another logical change"

# Push all commits
git push
```

### Conflict Resolution

```bash
# If main has moved forward
git fetch origin
git rebase origin/main

# Fix conflicts
# ... edit files ...
git add .
git rebase --continue

# Force push (safe after rebase)
git push --force-with-lease
```

## Continuous Integration

GitHub Actions automatically:
- **On every push/PR:** Runs tests, clippy, fmt, deny
- **On merge to main:** Builds Docker images, updates coverage
- **On tag:** Builds releases, publishes artifacts

Check status:

```bash
# View workflow status
gh run list

# View details
gh run view <run-id> --log
```

## Common Gotchas

| Problem | Solution |
|---------|----------|
| "Tests fail in CI but pass locally" | Check env vars, file paths, OS differences |
| "Clippy warnings in CI" | Run `cargo clippy -- -D warnings` locally |
| "Slow tests" | Profile with flamegraph, optimize hot paths |
| "Feature not showing in CLI help" | Did you rebuild? `cargo build` |
| "Secret leaked in logs" | Use `debug!()` not `info!()` for REPL/TUI |
| "Timeout in tests" | Use `tokio::time::sleep`, not `std::thread::sleep` |

## See Also

- **[Setup Guide](setup.md)** — Environment setup
- **[Testing Guide](testing.md)** — How to write tests
- **[AGENTS.md](/AGENTS.md)** — Coding rules and practices
- **[CONTRIBUTING.md](/CONTRIBUTING.md)** — Contribution guidelines

---

**Last updated:** Auto-generated by OpenWiki. For workflow questions, check CONTRIBUTING.md or ask for help.
