# IronClaw E2E Tests

Browser-level end-to-end tests for the IronClaw web gateway using Python + Playwright.

## Prerequisites

- Python 3.11+
- Rust toolchain (for building ironclaw)
- Chromium (installed via Playwright)

## Setup

```bash
cd tests/e2e
pip install -e .
playwright install chromium
```

## Build ironclaw

The tests need the ironclaw binary built with libsql support:

```bash
cargo build --no-default-features --features libsql
```

## Run tests

```bash
# From repo root
pytest tests/e2e/ -v

# Run a single scenario
pytest tests/e2e/scenarios/test_chat.py -v

# With visible browser (not headless)
HEADED=1 pytest tests/e2e/scenarios/test_connection.py -v
```

## Architecture

Tests start two subprocesses:
1. **Mock LLM** (`mock_llm.py`) -- fake OpenAI-compat server with canned responses
2. **IronClaw** -- the real binary with gateway enabled, pointing to the mock LLM

Then Playwright drives a headless Chromium browser against the gateway, making DOM assertions.

## Scenarios

| File | What it tests |
|------|--------------|
| `test_connection.py` | Auth, tab navigation, connection status |
| `test_chat.py` | Send message, SSE streaming, response rendering |
| `test_skills.py` | ClawHub search, skill install/remove |
| `test_tool_approval.py` | Tool approval overlay (approve, deny, always, params toggle) |
| `test_sse_reconnect.py` | SSE reconnection handling |
| `test_html_injection.py` | HTML injection security |
| `test_extensions.py` | Extensions tab: install, remove, configure, OAuth, auth card, activate |

## Adding new scenarios

1. Create `tests/e2e/scenarios/test_<name>.py`
2. Use the `page` fixture for a fresh browser page
3. Use selectors from `helpers.py` (update `SEL` dict if new elements are needed)
4. Keep tests deterministic -- use the mock LLM, not real providers

## Mocking API responses with `page.route()`

For tabs that depend on external data (extensions, jobs, memory, routines), use
Playwright's `page.route()` to intercept the browser's HTTP requests to the
ironclaw gateway and return deterministic fixture JSON. This avoids needing
real installed binaries, live external services, or complex database setup.

### Basic pattern

```python
import json

async def test_something(page):
    # 1. Set up route intercepts BEFORE navigation triggers the fetch
    # Always use async def handlers — route.fulfill() is a coroutine and must be awaited.
    async def handle_tools(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"tools": [{"name": "echo", "description": "Echo"}]}),
        )

    await page.route("**/api/extensions/tools", handle_tools)

    # 2. Navigate / interact to trigger the fetch
    await page.locator('.tab-bar button[data-tab="extensions"]').click()

    # 3. Assert on the rendered DOM
    rows = page.locator("#tools-tbody tr")
    assert await rows.count() == 1
```

### Matching only the exact path

`**/api/extensions` matches `http://host/api/extensions` but NOT sub-paths
like `http://host/api/extensions/install`. For the bare list endpoint, add
a check inside the handler:

```python
async def handle_ext_list(route):
    path = route.request.url.split("?")[0]
    if path.endswith("/api/extensions"):
        await route.fulfill(json={"extensions": []})
    else:
        await route.continue_()   # Let sub-paths through to the real server

await page.route("**/api/extensions*", handle_ext_list)
```

### Mocking method-specific behaviour (GET vs POST)

```python
async def handle_setup(route):
    if route.request.method == "GET":
        await route.fulfill(json={"secrets": [...]})
    else:  # POST
        await route.fulfill(json={"success": True})

await page.route("**/api/extensions/my-ext/setup", handle_setup)
```

### Counting calls (for reload tests)

```python
calls = []

async def counting_handler(route):
    calls.append(1)
    await route.fulfill(json={"extensions": []})

await page.route("**/api/extensions", counting_handler)
# ... interact ...
assert len(calls) == 2   # called twice (initial + after some action)
```

### Applying the pattern to other tabs

| Tab | Key API endpoints to mock |
|-----|--------------------------|
| **Jobs** | `/api/jobs`, `/api/jobs/{id}`, `/api/jobs/{id}/events` |
| **Memory** | `/api/memory/search`, `/api/memory/tree`, `/api/memory/read` |
| **Routines** | `/api/routines`, `/api/routines/{id}/runs` |

### Injecting state directly via `page.evaluate()`

For purely client-side UI (components rendered entirely in JS without API calls),
call the JavaScript function directly to skip the network layer entirely:

```python
# Show an approval card without needing a real tool execution
await page.evaluate("""
    showApproval({
        request_id: 'test-001',
        thread_id: currentThreadId,
        tool_name: 'shell',
        description: 'Run something',
    })
""")
```

This is the pattern used in `test_tool_approval.py` and parts of
`test_extensions.py` (auth card, configure modal).
