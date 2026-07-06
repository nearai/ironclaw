# Testing Guide

This guide covers IronClaw's testing strategy, test tiers, patterns, and how to write tests for different parts of the system.

## Testing Philosophy

IronClaw uses **test-first discipline**: every bug fix must include a regression test, and every feature change should include tests that validate the change. The goal is to catch regressions early and document expected behavior.

**Core Principle:** Test through the **caller**, not just the helper. When a helper controls a side effect (HTTP, DB write, OAuth flow, tool execution, UI mutation), test at the call site, not the helper alone.

## Testing Tiers

IronClaw has three tiers of tests:

### Tier 1: Unit Tests (Fast, Isolated)
- **Purpose:** Test local logic (functions, structs, pure computation)
- **Speed:** <1 second each
- **Count:** ~10,000 tests
- **External:** None (mocked if needed)
- **Command:** `cargo test --lib`
- **When to use:** Most tests; default choice
- **Example:** Test a sanitizer function, encoder, parser

### Tier 2: Integration Tests (Medium, Real Dependencies)
- **Purpose:** Test runtime behavior, database interactions, routing
- **Speed:** 1-60 seconds per test
- **Count:** ~1,000 tests
- **External:** PostgreSQL (optional), libSQL (built-in)
- **Command:** `cargo test --test '*'` or `cargo test --features integration`
- **When to use:** Testing a feature end-to-end with real DB or services
- **Example:** Test that a turn flows through the agent loop correctly

### Tier 3: E2E Tests (Slow, Full Stack)
- **Purpose:** Test user-visible flows (browser UI, API, CLI)
- **Speed:** Minutes to hours
- **Count:** ~100 tests
- **External:** Browser (Playwright), LLM API, live services
- **Command:** `cargo test -- --ignored` (run only marked tests)
- **When to use:** Testing complete workflows that the user would perform
- **Example:** Test that a Slack message flows through to a tool execution and back

## Running Tests

### Quick Smoke Test
```bash
# Run all fast tests (unit + some integration)
cargo test --lib
```

### Full Test Suite (Local)
```bash
# Run unit + integration tests (no external services)
cargo test
```

### Full Test Suite with PostgreSQL
```bash
# Start PostgreSQL
docker-compose up -d postgres

# Run all tests with PostgreSQL backend
IRONCLAW_HOOKS_POSTGRES_URL="postgres://ironclaw:ironclaw@127.0.0.1:5432/ironclaw" \
  cargo test --features postgres
```

### E2E Tests Only
```bash
# Run tests marked with #[ignore]
cargo test -- --ignored
```

### Single Test or Module
```bash
# Run one test
cargo test test_my_feature

# Run all tests in a module
cargo test -p ironclaw_agent_loop

# Run tests in a file
cargo test --test executor_happy_paths

# Run tests matching a pattern
cargo test --lib safety
```

### Watch Mode
```bash
# Re-run tests on file changes
cargo watch -x test

# Watch only unit tests
cargo watch -x "test --lib"
```

## Test Structure

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitizer_removes_injection() {
        let input = "Hello {{prompt_injection}}";
        let result = sanitize(input);
        
        assert_eq!(result, "Hello");  // Injection removed
    }

    #[test]
    fn test_sanitizer_preserves_safe_text() {
        let input = "Hello world";
        let result = sanitize(input);
        
        assert_eq!(result, "Hello world");
    }
}
```

**Key Points:**
- Test one behavior per test
- Use descriptive names (`test_*_should_*` or `test_*_when_*`)
- Test both happy path and edge cases
- Use assertions that show what failed (`assert_eq!`, `assert!`, `unwrap()` is OK in tests)

### Integration Test Example

```rust
// tests/executor_happy_paths.rs
#[tokio::test]
async fn test_loop_executes_tool_and_returns_result() {
    // Setup: Create a minimal executor environment
    let db = setup_test_db().await;
    let mut executor = ExecutorBuilder::default()
        .db(db.clone())
        .build();

    // Act: Run a simple turn
    let result = executor.execute(ExecutorRequest {
        thread_id: "test-thread",
        message: "what's 2+2?",
        ..Default::default()
    }).await;

    // Assert: Verify the result
    assert_ok!(result);
    assert_eq!(result.exit, LoopExit::Done);
    assert!(result.output.contains("4"));
}
```

**Key Points:**
- Use `#[tokio::test]` for async tests
- Set up real but minimal dependencies (in-memory DB, mocked HTTP)
- Test through the public API (call the actual executor, not internals)
- Assert the entire flow, not just one part

### Contract Test Example

Contract tests verify that a component meets an interface contract:

```rust
// crates/ironclaw_executor/tests/executor_happy_paths.rs
// Tests that the Executor trait is correctly implemented
#[tokio::test]
async fn executor_contract_simple_message() {
    // All implementations of Executor should pass this test
    // (e.g., LlmExecutor, CodeActExecutor, etc.)
}
```

## Test Patterns

### Pattern 1: Async Tests

```rust
#[tokio::test]
async fn test_async_operation() {
    let result = some_async_function().await;
    assert_ok!(result);
}

// For tests that need multi-threaded async
#[tokio::test(flavor = "multi_threaded")]
async fn test_concurrent_operations() {
    // Multiple tokio tasks can run concurrently
}
```

### Pattern 2: Test Fixtures and Helpers

```rust
// Define a test helper
async fn setup_test_environment() -> (TestDb, TestExecutor, TestLlm) {
    // Create test doubles
    (db, executor, llm)
}

#[tokio::test]
async fn test_with_fixtures() {
    let (db, executor, llm) = setup_test_environment().await;
    // Use fixtures in test
}
```

### Pattern 3: Parameterized Tests

```rust
#[test]
fn test_parser_on_multiple_inputs() {
    let cases = vec![
        ("input1", "expected1"),
        ("input2", "expected2"),
        ("input3", "expected3"),
    ];
    
    for (input, expected) in cases {
        let result = parse(input);
        assert_eq!(result, expected, "Failed for input: {}", input);
    }
}
```

### Pattern 4: Testing Error Cases

```rust
#[test]
fn test_invalid_input_returns_error() {
    let result = validate_input("");
    
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), ValidationError::EmptyInput);
}
```

### Pattern 5: Mocking External Services

```rust
// Use a mock HTTP server for tests
#[tokio::test]
async fn test_llm_provider_calls_api() {
    let mock_server = mockito::Server::new_async().await;
    
    // Mock the LLM API
    mock_server.mock("POST", "/v1/completions")
        .with_status(200)
        .with_body(r#"{"choices":[{"text":"hello"}]}"#)
        .create_async()
        .await;
    
    let provider = OpenAiProvider::new_with_url(mock_server.url());
    let result = provider.complete(request).await;
    
    assert_ok!(result);
}
```

## Code Coverage

### Viewing Coverage

```bash
# Generate coverage report (requires cargo-tarpaulin or cargo-llvm-cov)
cargo tarpaulin --out Html

# Open the report
open tarpaulin-report.html

# Or with llvm-cov
cargo llvm-cov --html
open target/llvm-cov/html/index.html
```

### Coverage Targets

- **Minimum:** 60% across the codebase
- **Safety layer:** >90% (security-critical)
- **Agent loop:** >80%
- **Capabilities:** >70%

Check the current coverage in the CI pipeline (GitHub Actions: `.github/workflows/coverage.yml`).

## Testing in CI/CD

### GitHub Actions Workflows

| Workflow | Purpose | Trigger | Time |
|----------|---------|---------|------|
| test.yml | Run all unit/integration tests | PR, push | ~10 min |
| reborn-tests.yml | Reborn crate tests | PR, push | ~5 min |
| e2e.yml | Browser-based E2E tests | Manual, dispatch | ~30 min |
| code_style.yml | Format, clippy, dependencies | PR, push | ~5 min |
| coverage.yml | Coverage measurement | Push to main | ~20 min |

### Pre-Commit Checks

The pre-commit git hook runs before you commit:

```bash
# Automatically runs on git commit:
1. cargo fmt -- --check        (formatting)
2. cargo clippy -- -D warnings  (linting)
3. cargo deny check              (dependencies)
4. UTF-8 validation
```

If these fail, fix them:
```bash
# Fix formatting
cargo fmt

# Fix clippy warnings
cargo clippy --fix
```

### Pre-Push Checks

The pre-push git hook runs before you push:

```bash
# Automatically runs on git push:
1. All pre-commit checks
2. Unit tests (cargo test --lib)
3. Regression tests (tests marked #[ignore])
4. Architecture boundary tests
```

If pre-push fails, either fix the issue or use `git push --no-verify` (sparingly).

## Writing Tests for Different Domains

### Testing Agent Loop Changes

```rust
// tests/agent_loop_contract.rs
#[tokio::test]
async fn test_loop_handles_capability_timeout() {
    let mut executor = TestExecutor::new();
    
    // Set a timeout on the capability
    executor.set_timeout(Duration::from_millis(100));
    
    // Request a capability that will timeout
    let result = executor.execute(ExecutorRequest {
        message: "call slow_tool",
        ..Default::default()
    }).await;
    
    // Verify timeout handling
    assert_eq!(result.exit, LoopExit::Error);
    assert!(result.error.contains("timeout"));
}
```

### Testing Capability Implementations

```rust
// crates/ironclaw_*/tests/capability_contract.rs
#[tokio::test]
async fn test_capability_conforms_to_host_api() {
    let host = TestHost::new();
    let capability = MyCapability::new();
    
    // Capability should implement the CapabilityPort trait
    let request = CapabilityRequest { ... };
    let response = capability.handle(request).await;
    
    assert_ok!(response);
}
```

### Testing Database Interactions

```rust
// tests/event_store_contract.rs
#[tokio::test]
async fn test_event_store_persists_and_retrieves() {
    let db = setup_test_db().await;
    let event = TestEvent { ... };
    
    // Write
    db.append_event(event.clone()).await?;
    
    // Read
    let retrieved = db.get_event(event.id).await?;
    
    // Verify
    assert_eq!(retrieved, event);
}
```

### Testing Safety Features

```rust
// tests/safety_contract.rs
#[test]
fn test_injection_detector_catches_prompt_injection() {
    let input = "Complete this: {{system_prompt}}";
    let result = detect_injection(input);
    
    assert!(result.is_injection);
    assert_eq!(result.injected_patterns, vec!["{{system_prompt}}"]);
}
```

## Test-First Bug Fix Workflow

When fixing a bug, follow this discipline:

1. **Write a failing test** that reproduces the bug:
   ```rust
   #[test]
   fn test_bug_reproduced() {
       let input = /* the case that triggers the bug */;
       let result = buggy_function(input);
       
       assert_eq!(result, /* expected */);  // This test currently fails
   }
   ```

2. **Verify the test fails**:
   ```bash
   cargo test test_bug_reproduced
   # Should output: thread '... test_bug_reproduced' panicked
   ```

3. **Fix the bug**:
   ```rust
   fn buggy_function(input: &str) -> String {
       // Fix the issue
   }
   ```

4. **Verify the test passes**:
   ```bash
   cargo test test_bug_reproduced
   # Should output: test test_bug_reproduced ... ok
   ```

5. **Run all tests**:
   ```bash
   cargo test
   ```

6. **Commit with a message**:
   ```bash
   git commit -m "fix(agent-loop): handle timeout in capability request

   Fixes #123. Added regression test test_bug_reproduced that
   captures the timeout scenario."
   ```

## Performance Testing

### Benchmarking

```rust
#[bench]
fn bench_sanitizer(b: &mut Bencher) {
    let input = /* complex injection */;
    
    b.iter(|| {
        sanitize(input)
    });
}
```

Run benchmarks:
```bash
cargo bench -p ironclaw_safety
```

### Stress Testing

For load testing, see `tools/ironclaw_stress/` for the stress test framework:

```bash
# Run a stress test
cargo run -p ironclaw_stress -- --requests 1000 --concurrency 10
```

## Debugging Tests

### Run a Single Test with Output

```bash
# See println! output
cargo test test_my_feature -- --nocapture

# See backtrace on failure
RUST_BACKTRACE=1 cargo test test_my_feature
```

### Debug with IDE

**VS Code:**
1. Install CodeLLDB extension
2. Create `.vscode/launch.json`:
   ```json
   {
     "version": "0.2.0",
     "configurations": [
       {
         "type": "lldb",
         "request": "launch",
         "name": "Debug Test",
         "cargo": {
           "args": [
             "test",
             "--lib",
             "test_my_feature",
             "--",
             "--nocapture"
           ]
         }
       }
     ]
   }
   ```

### Conditional Test Compilation

```rust
#[test]
#[cfg(feature = "slow_tests")]
fn test_slow_operation() {
    // This test only runs when feature is enabled
}

// Run with: cargo test --features slow_tests
```

## Common Test Issues

### Issue: Test Passes Locally but Fails in CI

1. **Check environment variables** — CI may not have your local env
2. **Check file paths** — CI uses different working directory
3. **Check timing** — CI is slower; timeout tests may be flaky
4. **Check OS** — CI may run on different OS than your machine

### Issue: Flaky Tests

Flaky tests pass sometimes, fail others. Causes:

- **Timing issues:** Use `tokio::time::sleep` instead of `std::thread::sleep`
- **Randomness:** Seed random number generators in tests
- **Shared state:** Each test should be independent
- **Network:** Mock external services instead of calling real APIs

Fix:

```rust
#[tokio::test]
async fn test_timing_sensitive() {
    // Use tokio sleep, not std sleep
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[test]
fn test_with_deterministic_rng() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    // Now rng is deterministic
}
```

### Issue: Test Hangs

If a test hangs:

```bash
# Kill hanging test (with timeout)
timeout 30 cargo test test_hanging

# Or run with timeout environment variable
TEST_TIMEOUT_SECS=30 cargo test
```

Find the cause:

- **Deadlock:** Check locks, mutexes, channels for circular waits
- **Infinite loop:** Add timeout or debug with println!
- **External service:** Mock external services; don't call real APIs

## See Also

- **[Setup Guide](setup.md)** — How to set up your environment
- **[Workflows: Code Review](workflows.md#code-review)** — How to review tests in PRs
- **[AGENTS.md: Testing](/AGENTS.md#test-discipline)** — Testing discipline rules
- **[COVERAGE_PLAN.md](/COVERAGE_PLAN.md)** — Coverage goals and strategy
- **[tests/e2e/CLAUDE.md](/tests/e2e/CLAUDE.md)** — E2E test documentation

---

**Last updated:** Auto-generated by OpenWiki. For test issues, check CI logs or ask for help.
