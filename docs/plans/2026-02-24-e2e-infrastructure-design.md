# E2E Testing Infrastructure Design

**Date:** 2026-02-24
**Status:** Approved
**Goal:** Deterministic browser-level E2E tests for the IronClaw web gateway using Python + Playwright, with a mock LLM backend for CI reliability.

---

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Assertion style | Deterministic DOM-first | Claude vision optional later; DOM assertions are fast, cheap, reliable |
| Language | Python + pytest + Playwright | Rich browser automation ecosystem, async/await, separate from Rust tests |
| LLM backend | Mock HTTP server | Canned OpenAI-compat responses; deterministic, fast, zero cost |
| Initial scope | 3 scenarios | Connection + Chat + Skills; covers highest-bug-rate areas |
| Architecture | Subprocess + Playwright | Tests the real binary end-to-end; proven pattern from existing ws_gateway tests |

---

## Architecture

```
                  pytest
                    |
         +----------+-----------+
         |                      |
   mock_llm.py           ironclaw binary
   (canned responses)    (cargo build --features libsql)
   127.0.0.1:{port}      127.0.0.1:{port}
         |                      |
         +----------+-----------+
                    |
              Playwright
              (headless Chromium)
              DOM assertions
```

**Flow:**

1. pytest session starts
2. Session-scoped fixture builds ironclaw binary (or reuses cached)
3. Session-scoped fixture starts mock LLM on OS-assigned port
4. Session-scoped fixture starts ironclaw subprocess pointing to mock LLM, gateway on OS-assigned port, libSQL in-memory
5. Function-scoped fixture launches Playwright browser, navigates to gateway with auth token
6. Each test uses Playwright locators + DOM assertions
7. Teardown kills ironclaw and mock LLM

---

## Directory Structure

```
tests/e2e/
  conftest.py              # pytest fixtures: build binary, start ironclaw, mock LLM, browser
  mock_llm.py              # OpenAI-compat HTTP server with canned responses
  helpers.py               # Shared utilities (wait_for_ready, selectors)
  scenarios/
    __init__.py
    test_connection.py     # Auth, tab navigation, connection status
    test_chat.py           # Send message, SSE streaming, response rendering
    test_skills.py         # Search, install, remove lifecycle
  pyproject.toml           # Dependencies
  README.md                # How to run locally and in CI
```

---

## Mock LLM Server

A minimal async HTTP server that speaks the OpenAI Chat Completions API.

**Endpoint:** `POST /v1/chat/completions`

**Behavior:**
- Parses the `messages` array from the request body
- Pattern-matches the last user message content to select a canned response
- Returns a well-formed `ChatCompletionResponse` with `id`, `choices[0].message`, `usage`
- Supports `stream: true` by returning SSE chunks with `delta` objects (critical: IronClaw streams responses via SSE to the browser)

**Canned response table:**

| Pattern (regex) | Response |
|-----------------|----------|
| `hello\|hi\|hey` | `Hello! How can I help you today?` |
| `2\+2\|2 \+ 2\|two plus two` | `The answer is 4.` |
| `skill\|install` | `I can help you with skills management.` |
| `.*` (default) | `I understand your request.` |

**Streaming format:**

```
data: {"id":"mock-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"The "},"finish_reason":null}]}

data: {"id":"mock-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"answer is 4."},"finish_reason":null}]}

data: {"id":"mock-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

**Implementation:** `aiohttp.web` (async, lightweight). No tool call support needed for initial 3 scenarios.

**Health check:** `GET /v1/models` returns `{"data": [{"id": "mock-model"}]}`.

---

## Fixtures

### Session-scoped (run once per test session)

**`ironclaw_binary`**
- Checks if `./target/debug/ironclaw` exists
- If missing or stale, runs `cargo build --no-default-features --features libsql`
- Returns the binary path
- Timeout: 300s (first build can be slow)

**`mock_llm_server`**
- Starts `mock_llm.py` as subprocess on `127.0.0.1:0` (OS-assigned port)
- Parses port from stdout (server prints `Mock LLM listening on 127.0.0.1:{port}`)
- Polls `GET /v1/models` until ready (timeout 10s)
- Yields `(process, url)`
- Kills process on teardown

**`ironclaw_server(ironclaw_binary, mock_llm_server)`**
- Starts the ironclaw binary with environment:

```
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=0
GATEWAY_AUTH_TOKEN=e2e-test-token
GATEWAY_USER_ID=e2e-tester
CLI_ENABLED=false
LLM_BACKEND=openai_compatible
LLM_BASE_URL={mock_llm_url}
LLM_MODEL=mock-model
DATABASE_BACKEND=libsql
LIBSQL_PATH=:memory:
SANDBOX_ENABLED=false
SKILLS_ENABLED=true
ROUTINES_ENABLED=false
HEARTBEAT_ENABLED=false
```

- Parses actual gateway port from ironclaw stdout (`Gateway listening on 127.0.0.1:XXXX`)
- Polls `GET /api/status` until ready (timeout 60s)
- Yields the base URL (`http://127.0.0.1:{port}`)
- Sends SIGTERM on teardown, SIGKILL after 5s grace

### Function-scoped (fresh per test)

**`page(ironclaw_server)`**
- Launches Playwright Chromium (headless)
- Creates new browser context (isolated cookies/storage)
- Creates new page with viewport 1280x720
- Navigates to `{base_url}/?token=e2e-test-token`
- Waits for network idle
- Yields the `Page` object
- Closes browser context on teardown

---

## Test Scenarios

### Scenario 1: Connection and Tab Navigation (`test_connection.py`)

Tests auth, initial page load, and tab switching.

```
test_page_loads_and_connects:
  1. Assert page title or main container is visible
  2. Assert connection status indicator shows "Connected" (or equivalent)
  3. Assert all 6 tab buttons visible: Chat, Memory, Jobs, Routines, Extensions, Skills

test_tab_navigation:
  1. For each tab in [Chat, Memory, Jobs, Routines, Extensions, Skills]:
     a. Click the tab button
     b. Assert the corresponding panel container becomes visible
     c. Assert no error toasts appear
  2. Return to Chat tab
  3. Assert chat input is visible and focusable

test_auth_rejection:
  1. Navigate to base_url without token (no ?token= param)
  2. Assert auth screen / login prompt appears (not the main app)
```

### Scenario 2: Chat Message Round-Trip (`test_chat.py`)

Tests the full message flow: user input -> gateway -> mock LLM -> SSE -> browser rendering.

```
test_send_message_and_receive_response:
  1. Locate chat input element
  2. Type "What is 2+2?"
  3. Press Enter (or click Send button)
  4. Wait for assistant message to appear (timeout 15s)
  5. Assert user message bubble contains "What is 2+2?"
  6. Assert assistant message bubble contains "4"
  7. Assert no error toasts visible

test_multiple_messages:
  1. Send "Hello"
  2. Wait for response containing "Hello" or "help"
  3. Send "What is 2+2?"
  4. Wait for response containing "4"
  5. Assert message count >= 4 (2 user + 2 assistant)

test_empty_message_not_sent:
  1. Focus chat input
  2. Press Enter with empty input
  3. Assert no new messages appear after 2s
```

### Scenario 3: Skills Lifecycle (`test_skills.py`)

Tests ClawHub search, install, and remove through the browser UI.

Note: ClawHub registry blocks non-browser TLS fingerprints but Playwright is a real browser, so this works. Tests are skipped if ClawHub is unreachable.

```
test_skills_tab_visible:
  1. Click Skills tab
  2. Assert skills panel is visible
  3. Assert search input is present

test_skills_search:
  1. Click Skills tab
  2. Type "markdown" in search input
  3. Click Search (or press Enter)
  4. Wait for results (timeout 15s)
  5. Assert at least one result card is visible
  6. Assert result cards contain: name, version, description fields

test_skills_install_and_remove:
  1. Search for a skill
  2. Override window.confirm to auto-accept: page.evaluate("window.confirm = () => true")
  3. Click Install on first result
  4. Wait for installed skills list to update (timeout 15s)
  5. Assert skill appears in installed section
  6. Click Remove on the installed skill
  7. Wait for installed section to update
  8. Assert skill is gone from installed list
```

---

## Port Discovery

IronClaw logs `Gateway listening on 127.0.0.1:XXXX` at startup. The fixture reads stdout line-by-line until it finds this pattern, extracts the port.

```python
async def wait_for_port(process, pattern=r"Gateway listening on .+:(\d+)", timeout=60):
    """Read process stdout until we find the listening port."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = await asyncio.wait_for(
            process.stdout.readline(), timeout=deadline - time.monotonic()
        )
        if match := re.search(pattern, line.decode()):
            return int(match.group(1))
    raise TimeoutError("ironclaw did not report listening port")
```

Same pattern for the mock LLM server.

---

## Dependencies

```toml
# tests/e2e/pyproject.toml
[project]
name = "ironclaw-e2e"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = [
    "pytest>=8.0",
    "pytest-asyncio>=0.23",
    "playwright>=1.40",
    "aiohttp>=3.9",
    "httpx>=0.27",
]

[project.optional-dependencies]
vision = [
    "anthropic>=0.40",
]
```

---

## CI Integration

```yaml
# .github/workflows/e2e.yml
name: E2E Tests
on:
  schedule:
    - cron: "0 6 * * 1"  # Weekly Monday 6 AM UTC
  workflow_dispatch:
  pull_request:
    paths:
      - 'src/channels/web/**'
      - 'tests/e2e/**'

jobs:
  e2e:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/cache@v4
        with:
          path: target
          key: e2e-${{ hashFiles('Cargo.lock') }}
      - name: Build ironclaw
        run: cargo build --no-default-features --features libsql
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - name: Install E2E dependencies
        run: |
          cd tests/e2e
          pip install -e .
          playwright install chromium
      - name: Run E2E tests
        run: pytest tests/e2e/ -v --timeout=120
```

**Trigger policy:** Weekly + manual + PRs touching web gateway or E2E tests. Not on every PR.

---

## Future: Claude Vision Layer

Not in initial scope. Design accommodates it via:

- `conftest.py` fixture `claude_vision` wrapping `anthropic.Anthropic()`
- Helper `assert_visually(page, prompt)`: takes screenshot, sends to Claude vision API, asserts response
- Gated behind `@pytest.mark.vision`, only runs when `ANTHROPIC_API_KEY` is set
- Use cases: "no raw HTML visible in chat", "markdown renders correctly", "no layout breakage"

---

## Success Criteria

1. `pytest tests/e2e/ -v` passes locally with a pre-built ironclaw binary
2. All 3 scenarios (connection, chat, skills) exercise real browser interactions
3. Mock LLM provides deterministic responses (no flaky tests from LLM randomness)
4. CI workflow runs on web gateway changes and weekly schedule
5. Test failures produce clear error messages with screenshot artifacts
