# IronClaw E2E Tests

Python/Playwright test suite that runs against a live ironclaw instance. Added in PR #553 ("Trajectory benchmarks and e2e trace test rig").

## Setup

```bash
cd tests/e2e

# Create virtualenv (one-time)
python -m venv .venv
source .venv/bin/activate   # or .venv\Scripts\activate on Windows

# Install dependencies
pip install -e .

# Install browser binaries (one-time)
playwright install chromium
```

Dependencies: `pytest`, `pytest-asyncio`, `pytest-playwright`, `pytest-timeout`, `playwright`, `aiohttp`, `httpx`. Optional: `anthropic` (vision extras). Requires Python >= 3.11.

## Running Tests

```bash
# Activate venv first
source .venv/bin/activate

# Run all scenarios (conftest.py builds the binary and starts all servers automatically)
pytest scenarios/

# Run a specific scenario
pytest scenarios/test_chat.py
pytest scenarios/test_sse_reconnect.py

# Run with verbose output
pytest scenarios/ -v

# Run with a specific timeout (default is 120s per test, set in pyproject.toml)
pytest scenarios/ --timeout=60

# Run with a headed browser (useful for debugging)
HEADED=1 pytest scenarios/
```

## Test Scenarios

| File | What it tests |
|------|--------------|
| `test_connection.py` | Gateway reachability, tab navigation, auth rejection (no token shows auth screen) |
| `test_chat.py` | Send message via browser UI, verify streamed response from mock LLM; also tests empty-message suppression |
| `test_html_injection.py` | XSS vectors injected directly via `page.evaluate("addMessage('assistant', ...)")` are sanitized by `renderMarkdown`; user messages are shown as escaped plain text |
| `test_skills.py` | Skills tab UI visibility, ClawHub search (skipped if registry unreachable), install + remove lifecycle |
| `test_sse_reconnect.py` | SSE reconnect basics plus keepalive comments, multi-tab fanout, restart recovery/history rebuild, stale reconnect IDs, and connection-limit handling |
| `test_tool_approval.py` | Approval card appears, buttons disable on approve/deny, parameters toggle via `page.evaluate("showApproval(...)")`; the waiting-approval regression uses a real HTTP tool call |
| `test_extension_uninstall_cleanup.py` | Real install/setup/remove coverage for WASM tools, WASM channels, OAuth-backed shared Google tools, and MCP servers; verifies uninstall deletes stored secrets from the libSQL `secrets` table while preserving shared credentials until the last referencing extension is removed |
| `test_oauth_refresh.py` | Hosted Gmail OAuth regression: complete setup via `/oauth/callback`, expire the stored access token in libSQL, trigger a real `gmail` tool call through `/api/chat/send`, and verify refresh goes through the mock `/oauth/refresh` proxy without forwarding `client_secret` |
| `test_dom_resource_limits.py` | DOM pruning at MAX_DOM_MESSAGES cap, no setInterval timer leaks across SSE reconnect cycles, streaming message preservation during pruning |
| `test_coding_project_flow.py` | Coding-agent UX: project chrome renders after create+active, `!` prefix toggles the shell-mode input badge, `!echo hello` dispatches through `/api/chat/send mode=shell`, renders a shell turn card with exit-code badge, persists across reload, rejects 409 when the thread has no project, and the `github_repo` newtype rejects malformed slugs at the HTTP boundary |

## `helpers.py`

Shared constants and utilities imported by every test file and `conftest.py`.

- **`SEL`** — dict of CSS/ID selectors for all DOM elements (chat input, message bubbles, approval card, tab buttons, skill search, etc.). Update this dict when frontend HTML changes; tests import selectors from here rather than hardcoding them.
- **`TABS`** — ordered list of tab names: `["chat", "memory", "jobs", "routines", "extensions", "skills"]`.
- **`AUTH_TOKEN`** — hardcoded to `"e2e-test-token"`. Used by `conftest.py` when starting the server (`GATEWAY_AUTH_TOKEN`) and by the `page` fixture when navigating (`/?token=e2e-test-token`).
- **`wait_for_ready(url, timeout, interval)`** — polls a URL until HTTP 200 or timeout; used to wait for the gateway and mock LLM to become available.
- **`wait_for_port_line(process, pattern, timeout)`** — reads a subprocess's stdout line-by-line until a regex match; used to extract the dynamically assigned mock LLM port from `MOCK_LLM_PORT=XXXX`.

## `conftest.py` and Fixtures

All fixtures are defined in `tests/e2e/conftest.py`. Running `pytest scenarios/` from the `tests/e2e/` directory picks up this conftest automatically (it is one level above `scenarios/`).

### Session-scoped fixtures (run once per `pytest` invocation)

| Fixture | What it does |
|---------|-------------|
| `ironclaw_binary` | Checks `target/debug/ironclaw`; if absent, runs `cargo build --no-default-features --features libsql` (timeout 600s). |
| `mock_llm_server` | Starts `mock_llm.py --port 0`, reads the assigned port from stdout, waits for `/v1/models` to return 200. Yields the base URL. |
| `ironclaw_server` | Starts the ironclaw binary with a minimal env (see below), waits for `/api/health` (timeout 60s). Yields the base URL. On teardown sends **SIGINT** (not SIGTERM) so the tokio ctrl_c handler triggers a graceful shutdown and LLVM coverage data is flushed. |
| `hosted_oauth_refresh_server` | Starts a second ironclaw instance with a dedicated libSQL DB and `GOOGLE_OAUTH_CLIENT_ID=hosted-google-client-id`, while still pointing `IRONCLAW_OAUTH_EXCHANGE_URL` at `mock_llm.py`. Yields a dict with `base_url`, `db_path`, `gateway_user_id`, and `mock_llm_url` for the hosted refresh regression scenario. |
| `extension_cleanup_server` | Starts an isolated ironclaw instance with its own temp DB/home/WASM dirs, `SECRETS_MASTER_KEY`, and hosted-style OAuth env so uninstall-cleanup scenarios can inspect the `secrets` table without interfering with the shared E2E server state. |
| `managed_gateway_server` | Function-scoped restartable gateway instance for SSE/connectivity scenarios; preserves port/DB/home across explicit stop/start calls so tests can simulate server restarts. |
| `limited_gateway_server` | Function-scoped gateway instance with `GATEWAY_MAX_CONNECTIONS=2` for connection-cap coverage. |
| `browser` | Launches a single Chromium instance (headless by default; set `HEADED=1` for headed). Shared across all tests. |

### Function-scoped fixtures

| Fixture | What it does |
|---------|-------------|
| `page` | Creates a fresh browser **context** (viewport 1280×720) and **page** per test, navigates to `/?token=e2e-test-token`, and waits for `#auth-screen` to become hidden before yielding. Closes the context after each test. |

The function-scoped `page` fixture means **each test gets a clean browser context** (cookies, storage, etc.) but reuses the same ironclaw server and browser process. Tests that need the server URL directly (e.g., `test_auth_rejection`) accept `ironclaw_server` as an additional parameter.

### Environment passed to ironclaw in tests

The `ironclaw_server` fixture injects a minimal, deterministic environment:

```
GATEWAY_ENABLED=true, GATEWAY_HOST=127.0.0.1, GATEWAY_PORT=<dynamic>
GATEWAY_AUTH_TOKEN=e2e-test-token, GATEWAY_USER_ID=e2e-tester
CLI_ENABLED=false
LLM_BACKEND=openai_compatible, LLM_BASE_URL=<mock_llm_url>, LLM_MODEL=mock-model
DATABASE_BACKEND=libsql, LIBSQL_PATH=<tmpdir>/e2e.db
SANDBOX_ENABLED=false, ROUTINES_ENABLED=false, HEARTBEAT_ENABLED=false
EMBEDDING_ENABLED=false, SKILLS_ENABLED=true
ONBOARD_COMPLETED=true   # prevents setup wizard
```

The `hosted_oauth_refresh_server` fixture uses the same baseline, but with its own DB/home tempdirs and `GOOGLE_OAUTH_CLIENT_ID=hosted-google-client-id` so hosted OAuth flows exercise proxy credential injection instead of the baked-in desktop Google app.

The binary is also started with `--no-onboard`. Coverage env vars (`CARGO_LLVM_COV*`, `LLVM_*`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_INCREMENTAL`) are forwarded from the outer environment when present.

## Mock LLM (`mock_llm.py`)

An `aiohttp`-based OpenAI-compatible server used by tests that need deterministic LLM responses without hitting a real provider.

```bash
# Start manually (port auto-selected, printed as MOCK_LLM_PORT=XXXX)
python mock_llm.py --port 0
```

It serves `POST /v1/chat/completions` (streaming + non-streaming) and `GET /v1/models`. Responses are pattern-matched from `CANNED_RESPONSES` against the last user message. Unmatched messages return `"I understand your request."`. The model name reported is always `"mock-model"`.

It also hosts OAuth test endpoints:
- `POST /oauth/exchange` for hosted auth-code exchange
- `POST /oauth/refresh` for hosted refresh-token exchange
- `GET /__mock/oauth/state` and `POST /__mock/oauth/reset` so HTTP E2E scenarios can assert exact proxy payloads and reset counters between setup and refresh assertions

To add a new canned response:
```python
# In mock_llm.py
CANNED_RESPONSES = [
    (re.compile(r"your pattern", re.IGNORECASE), "Your response"),
    ...
]
```

## Configuration

`conftest.py` handles all server startup automatically — you do not need to start ironclaw manually before running `pytest`. The conftest builds the binary (libsql feature), starts the mock LLM, and starts ironclaw with a fresh temp database on every `pytest` invocation.

If you need to test against a manually started ironclaw, you can skip conftest by running pytest with `--co` (collect-only) to understand what would run, or by calling the httpx/REST helpers directly without the `page` fixture.

## Writing New Scenarios

1. Create `scenarios/test_my_feature.py`.
2. All async functions are automatically recognized as tests — `asyncio_mode = "auto"` is set globally in `pyproject.toml`. Do **not** add `@pytest.mark.asyncio`; it is redundant and raises a warning.
3. Use the `page` fixture for browser tests (function-scoped, fresh context each test). Use `ironclaw_server` directly for pure HTTP tests.
4. Import selectors from `helpers.SEL` and `helpers.AUTH_TOKEN` — do not hardcode selectors or tokens inline.
5. Use `httpx.AsyncClient` for REST calls; `aiohttp` for SSE streaming.
6. Keep new fixtures session-scoped where possible; server startup is expensive. Function-scoped fixtures (like `page`) are fine for browser state that must be clean per test.

```python
import httpx
from helpers import AUTH_TOKEN

async def test_my_endpoint(ironclaw_server):
    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    async with httpx.AsyncClient() as client:
        r = await client.get(f"{ironclaw_server}/api/health", headers=headers)
        assert r.status_code == 200
```

For browser tests:
```python
from helpers import SEL

async def test_my_ui_feature(page):
    # page is already navigated and authenticated
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    # ... interact with the page ...
```

### Gotchas

- **`asyncio_default_fixture_loop_scope = "session"`** — all async fixtures share one event loop. Do not use `asyncio.run()` inside fixtures; use `await` directly.
- **The `page` fixture navigates with `/?token=e2e-test-token` and waits for `#auth-screen` to be hidden.** Tests receive a page that is already past the auth screen and has SSE connected.
- **Raw SSE checks belong in `aiohttp`, not Playwright.** Browser `EventSource` does not expose keepalive comments, so keepalive and low-level reconnect/header scenarios should use the `sse_stream()` helper in `helpers.py`.
- **`test_skills.py` makes real network calls to ClawHub.** Tests skip (not fail) if the registry is unreachable via `pytest.skip()`.
- **`test_html_injection.py` injects state via `page.evaluate(...)`, and most of `test_tool_approval.py` does too.** The waiting-approval regression in `test_tool_approval.py` intentionally uses a real tool approval flow so it can verify backend thread-state handling.
- **Browser is Chromium only.** `conftest.py` uses `p.chromium.launch()`; there is no Firefox or WebKit variant.
- **Default timeout is 120 seconds** (pyproject.toml). Individual `wait_for` calls inside tests use shorter timeouts (5–20s) for faster failure messages.
- **The libsql database is a temp directory** created fresh per `pytest` invocation; tests do not share state across runs.

## CI Integration

E2E tests run in CI with `cargo-llvm-cov` for coverage collection. The CI workflow (`fix(ci): persist all cargo-llvm-cov env vars for E2E coverage` — PR #559) sets `LLVM_PROFILE_FILE` and related vars before spawning the ironclaw binary so coverage from E2E runs is captured.
