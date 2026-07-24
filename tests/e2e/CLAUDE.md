# IronClaw E2E Tests

Python/Playwright test suite that runs against a live ironclaw instance. Added in PR #553 ("Trajectory benchmarks and e2e trace test rig").

**The live surface is the Reborn `ironclaw serve` WebChat v2 SPA** (`/`,
`/api/webchat/v2/*`), driven via the `reborn_webui_harness` / `reborn_v2_*`
fixtures and `SEL_V2` selectors. This is the only surface the Reborn coverage
gate (`reborn_coverage_tests.txt`, run by `reborn-e2e.yml`) exercises. When
adding a browser test, use the `reborn_v2_*` fixtures — see
`test_reborn_webui_v2_smoke.py` for the canonical example.

> **Tier B note:** the v1 legacy `ironclaw` gateway binary
> (`ironclaw-legacy`, formerly built by the `ironclaw_binary` fixture) was
> **deleted** under Tier B (`docs/plans/2026-07-02-reborn-internal-module-refactor.md`
> §8). The eight browser scenarios that drove only the legacy gateway
> (`test_connection`, `test_chat`, `test_html_injection`, `test_skills`,
> `test_sse_reconnect`, `test_tool_approval`, `test_dom_resource_limits`,
> `test_reborn_gateway_smoke`) were removed. The remaining suite still carries
> legacy `conftest.py` fixtures (`ironclaw_binary`, `ironclaw_server`, the
> legacy `page`/`browser`) and a number of `test_v2_*` scenarios that depend on
> them; those are non-functional until repointed at the Reborn serve binary and
> are tracked as a dedicated migration in issue #6369. The doc below still
> describes that legacy machinery where it remains — treat any `ironclaw_binary`
> / legacy-`page` reference as pending-removal, not current guidance.

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

Dependencies: `pytest`, `pytest-asyncio`, `pytest-playwright`, `pytest-timeout`, `playwright`, `aiohttp`, `httpx`. Optional: `anthropic` (vision extras). Requires Python >= 3.11. Emulate-backed provider tests also require Node.js. CI installs Node 24, builds the `serrrfirat/emulate` commit pinned by [the Reborn E2E workflow](../../.github/workflows/reborn-e2e.yml), and passes its CLI through `IRONCLAW_EMULATE_CLI`. Without that override, unrelated local Emulate tests retain the `emulate@0.7.0` fallback.

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

The suite has grown to ~65+ scenario files. The table below is a **representative
subset grouped by surface**, not an exhaustive list — run `pytest scenarios/ --co -q`
from `tests/e2e/` for the full, current set.

**Legacy `ironclaw` gateway (browser via `page` / `SEL`):**

| File | What it tests |
|------|--------------|
| `test_connection.py` | Gateway reachability, tab navigation, auth rejection (no token shows auth screen) |
| `test_chat.py` | Send message via browser UI, verify streamed response from mock LLM, attachments, empty-message suppression |
| `test_html_injection.py` | XSS vectors are sanitized by `renderMarkdown`; user messages shown as escaped plain text |
| `test_skills.py` | Skills tab UI, ClawHub search (skipped if registry unreachable), install + remove lifecycle |
| `test_sse_reconnect.py` | SSE reconnect, keepalive comments, multi-tab fanout, restart recovery, connection-limit handling |
| `test_tool_approval.py` | Approval card appears, buttons disable on approve/deny; waiting-approval regression uses a real HTTP tool call |
| `test_dom_resource_limits.py` | DOM pruning at `MAX_DOM_MESSAGES`, no `setInterval` leaks across reconnect cycles |

**Reborn `ironclaw serve` WebChat v2 SPA (browser via `reborn_v2_*` / `SEL_V2`):**

| File | What it tests |
|------|--------------|
| `test_reborn_webui_v2_smoke.py` | Canonical v2 smoke: serve boots, SPA renders authed shell, bearer auth + `?token=` shim scope, text turn persists/streams, thread list/delete, timeline pagination, composer-while-running, approval-gate send block, **new-chat-while-a-run-is-active (the #5256 `submitBusyRef` deadlock regression)** |
| `test_reborn_gateway_smoke.py` | Legacy `ironclaw` web channel (`/api/chat/*`) under `ENGINE_V2` — NOT the reborn binary |
| `test_reborn_v2_file_download.py` | Agent-produced workspace files are downloadable from the v2 UI |
| `test_v2_activity_shell.py` | v2 activity shell rendering |
| `test_v2_*_flow.py` / `test_v2_engine_*.py` | v2 auth/OAuth matrix (GitHub PAT, GSuite, Notion MCP) and v2-engine approval/auth/tool-lifecycle/error-handling |

### Reborn E2E coverage gate

The Code Coverage workflow does **not** run the entire historical Python E2E
tree. It reads `tests/e2e/reborn_coverage_tests.txt`, which is limited to
scenarios that start the standalone `ironclaw serve` binary and exercise
the Reborn WebChat v2 or OpenAI-compatible API surface. The manifest may use
pytest node IDs to include only the Reborn binary/API checks from a broader
scenario file.

Do not add legacy `ironclaw` gateway tests to that manifest, even if they run
with `ENGINE_V2=true`. Those are compatibility/runtime E2E tests. Direct
Emulate provider-contract tests are also excluded from the Reborn coverage gate
because they primarily validate the fixture provider, while current hosted
full-path Emulate tests still start the legacy gateway binary.

**Provider-contract / full-path (Emulate-backed):**

| File | What it tests |
|------|--------------|
| `test_emulate_reborn_provider_contracts.py` | Reborn Emulate fixture contracts: Google account isolation and stateful reads/writes, Slack QA 9/10 provider shapes and strict-scope failures, and GitHub identity plus positive/negative state transitions |
| `test_provider_fault_proxy.py` | Harness self-tests for reusable provider status, response, timeout, connection-reset, and lost-acknowledgement profiles plus safe request evidence and reset |
| `test_provider_capability_inventory.py` | Fast completeness gate derived from shipped first-party manifests. Every static provider capability must be tested, live-only, unsupported, or covered by an owned waiver in `fixtures/provider_capability_coverage.toml`; non-Emulate evidence names its exact Cargo target, source, and executable test. |
| `test_reborn_qa_trace_full_path.py` | Harvested and typed provider operations through standalone Reborn and Emulate, including representative read/idempotent-write/non-idempotent-write fault cases with provider readback |
| `test_reborn_emulate_full_path.py` | Install/auth a first-party extension, drive scripted Gmail/Calendar/Drive/GitHub tool calls, assert provider state and cleanup via Emulate |
| `test_oauth_refresh.py` | Hosted Gmail OAuth refresh: expire token, real tool call, refresh via mock proxy without leaking `client_secret` |
| `test_extension_uninstall_cleanup.py` | Install/remove for WASM tools/channels, shared Google tools, MCP; uninstall deletes secrets, preserves shared creds |

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
| `ironclaw_binary` | Legacy gateway binary. Checks `target/debug/ironclaw`; if absent, runs `cargo build -p ironclaw` (timeout 600s). |
| `ironclaw_reborn_binary` | Reborn v2 binary. Builds `target/debug/ironclaw` with default features when stale/missing. Used by the v2 SPA and full-path fixture scenarios. |
| `reborn_v2_server` | Starts `ironclaw serve` (v2 SPA at `/`, `local-dev` profile) against `mock_llm_server`; config written via `_write_config_toml` (selects the `openai` provider pointed at the mock). Waits for `/api/health`; SIGINT teardown. (Module-scoped, defined in `test_reborn_webui_v2_smoke.py`.) |
| `reborn_v2_browser` | Chromium instance for the v2 scenarios, independent of the legacy `browser` fixture (generous launch timeout + retry). |
| `mock_llm_server` | Starts `mock_llm.py --port 0`, reads the assigned port from stdout, waits for `/v1/models` to return 200. Yields the base URL. Serves canned responses including delayed ones (e.g. `"editable composer slow response"` → ~5s) so tests can act while a run is in flight. |
| `emulate_google_server` | Starts the Emulate CLI selected by `IRONCLAW_EMULATE_CLI`, or the `emulate@0.7.0` fallback, with `fixtures/emulate/google_gmail.yaml`; waits for the Gmail messages endpoint; and yields the base URL for HTTP rewrite maps. The pinned CI fork covers Gmail, Calendar, Drive, Docs, Sheets, and Slides. Local runs skip if neither the selected CLI nor `npx` is available; CI fails. |
| `emulate_slack_server` | Starts the selected Emulate CLI with `fixtures/emulate/slack.yaml`, waits for seeded token auth to pass `auth.test`, and yields the base URL for Slack provider-contract assertions, including `search.messages` with the pinned CI fork. |
| `emulate_github_server` | Starts the selected Emulate CLI with `fixtures/emulate/github.yaml`, waits for `/user` to return the seeded actor, and yields the base URL for GitHub provider-contract assertions. |
| `provider_fault_proxy_world` | Module-scoped. Starts one transparent aiohttp proxy per resettable Emulate provider. Reborn traffic crosses these proxies; setup and provider readback continue to use direct Emulate URLs. Faults and the safe request ledger reset independently from provider state. |
| `ironclaw_server` | Starts the ironclaw binary with a minimal env (see below), waits for `/api/health` (timeout 60s). Yields the base URL. On teardown sends **SIGINT** (not SIGTERM) so the tokio ctrl_c handler triggers a graceful shutdown and LLVM coverage data is flushed. |
| `hosted_oauth_refresh_server` | Starts a second ironclaw instance with a dedicated libSQL DB and `GOOGLE_OAUTH_CLIENT_ID=hosted-google-client-id`, while still pointing `IRONCLAW_OAUTH_EXCHANGE_URL` at `mock_llm.py`. Yields a dict with `base_url`, `db_path`, and `mock_llm_url` for hosted refresh scenarios that do not need provider API calls. |
| `hosted_google_emulate_server` | Starts the same hosted OAuth fixture shape, but sets `IRONCLAW_TEST_HTTP_REWRITE_MAP` so Google WASM HTTP calls to `gmail.googleapis.com`, `www.googleapis.com`, and `slides.googleapis.com` hit `emulate_google_server`. Yields `emulate_google_url` in addition to the hosted OAuth server fields. |
| `hosted_google_oauth_refresh_server` | Compatibility alias for `hosted_google_emulate_server`, retained for hosted Gmail OAuth refresh regression tests. |
| `extension_cleanup_server` | Starts an isolated ironclaw instance with its own temp DB/home/WASM dirs, `SECRETS_MASTER_KEY`, and hosted-style OAuth env so uninstall-cleanup scenarios can inspect the `secrets` table without interfering with the shared E2E server state. |
| `managed_gateway_server` | Function-scoped restartable gateway instance for SSE/connectivity scenarios; preserves port/DB/home across explicit stop/start calls so tests can simulate server restarts. |
| `limited_gateway_server` | Function-scoped gateway instance with `GATEWAY_MAX_CONNECTIONS=2` for connection-cap coverage. |
| `browser` | Launches a single Chromium instance (headless by default; set `HEADED=1` for headed). Shared across all tests. |

### Function-scoped fixtures

| Fixture | What it does |
|---------|-------------|
| `reborn_qa_emulate_provider_server` | Restores providers mutated by a QA journey while reusing the session-built binary and one module-scoped Reborn process. Google mutation cases restart the seeded provider on its stable port; Slack deliveries are deleted by provider-issued timestamp so the OAuth account remains valid. Read-only providers stay warm. |
| `reborn_provider_fault_server` | Clears provider fault rules and request evidence before and after each representative fault case, then restores the affected seeded provider. |
| `page` | Legacy gateway. Creates a fresh browser **context** (viewport 1280×720) and **page** per test, navigates to `/?token=e2e-test-token`, and waits for `#auth-screen` to become hidden before yielding. Closes the context after each test. |
| `reborn_v2_page` | Reborn v2 SPA. Fresh context/page navigated to `/?token=<REBORN_V2_AUTH_TOKEN>`, waits for `SEL_V2["chat_composer"]` (authed `/chat` shell). Use this (not `page`) for v2 browser tests. |

The function-scoped `page` fixture means **each test gets a clean browser context** (cookies, storage, etc.) but reuses the same ironclaw server and browser process. Tests that need the server URL directly (e.g., `test_auth_rejection`) accept `ironclaw_server` as an additional parameter.

### Emulate provider coverage

Use Emulate for provider APIs that map directly to Reborn features already in
this repo: Google Gmail/Calendar/Drive/Docs/Sheets/Slides, Slack
delivery/search/reactions/user lookup, and GitHub repository, issue, pull
request, review thread, contents, search, branch, release, fork, Git object,
and Actions workflows. The current provider contract covers
seeded reads plus stateful writes for Gmail send, Calendar event create/delete,
Drive upload/readback, Slack channel/thread/DM delivery/reactions, GitHub repo
create/list, release create/list/latest, issue create/read/comment/search, PR
create/read/list/files/review/comment/merge, branch/ref creation, Git
blob/tree/commit read/write, contents create/read/delete, fork create/list,
review-thread resolution, Actions dispatch/readback/reruns, and Slides
presentation/slide/text/shape/image mutation.

Direct provider-contract tests prove the Emulate fixture layer itself. Full-path
recorded-trace tests load harvested `LlmTrace` JSON through `mock_llm.py`'s
`/__mock/llm_trace` endpoint. `test_reborn_qa_trace_replay.py` replays every
model response from every case in the live-canary manifest. Its closed-set
assertions require new fixtures and provider operations to be classified
instead of silently losing coverage. `test_reborn_qa_trace_full_path.py`
discovers every manifest journey with an Emulate-supported provider call and
executes that provider leg through standalone `ironclaw serve`, installed and
authenticated first-party extensions, the credential/network boundaries, and
the pinned Emulate fork. Mutated provider state is reset or removed using
provider-issued evidence while read-only providers, the built binary, and the
Reborn process are reused; representative mutation journeys run a second time with
clean-baseline assertions to prevent order-dependent passes.
Cross-provider ordering is retained, fresh Docs and
Sheets IDs are bound from earlier real tool results, redacted provider IDs are
mapped to deterministic seeded resources, and assertions target capability
success plus provider readback rather than recorded final-answer wording.
`ProviderOperationCase` adds typed provider service, capability, argument,
baseline, and readback cases for operations not yet present in harvested
journeys. These cases reuse the same Reborn process and reset only their mutable
provider world.
`ProviderFaultProfile` places a transparent proxy between that Reborn process
and Emulate. Reusable profiles cover HTTP 400/401/403/404/409/429/5xx,
timeout, connection reset, malformed/truncated/missing-field responses, and a
provider commit followed by a lost acknowledgement. The full-path matrix uses
representative read, idempotent-write, and non-idempotent-write operations,
asserts one wire attempt, and reads Emulate directly to distinguish no effect
from an unacknowledged committed effect. The proxy stores credential
fingerprints and body digests, never raw credentials or request bodies.
Debug E2E binaries honor `IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP` only for
loopback IP socket targets after the original destination has passed the normal
network policy and DNS checks. Release binaries fail startup if that test-only
environment variable is set.

Legacy hosted full-path Reborn + Emulate tests use
`hosted_google_emulate_server` or a matching
provider fixture, install/auth the first-party extension through IronClaw, drive
the scripted mock model through `/api/chat/send`, auto-resolve expected approval
gates, and read provider state back from Emulate. This is the contract tier to
use when the behavior being protected is extension install/auth, model-to-tool
routing, tool execution, and provider mutation together.

The pinned `serrrfirat/emulate` fork adds the Google Calendar, Docs, Drive,
Sheets, and Slides operations; Slack `search.messages`; and GitHub Contents,
GraphQL review threads, and seeded Actions workflows used by the provider
contract catalog. All 123 shipped static provider capabilities now have
executable hermetic evidence: 119 cross the standalone Reborn + Emulate path,
while `github.handle_webhook`, `nearai.web_search`,
`web-access.get_content`, and `web-access.search` use Reborn integration tests
at their actual local-WASM or hosted-MCP seams. The inventory records the exact
Cargo target, source, and test for those non-Emulate cases and fails if that
evidence stops being executable. Manual QA rows that mention Telegram or
Twitter/X remain model-replay-only unless paired with their own provider
fixture.

### Environment passed to ironclaw in tests

The `ironclaw_server` fixture injects a minimal, deterministic environment:

```
GATEWAY_ENABLED=true, GATEWAY_HOST=127.0.0.1, GATEWAY_PORT=<dynamic>
GATEWAY_AUTH_TOKEN=e2e-test-token, GATEWAY_USER_ID=e2e-tester
CLI_ENABLED=false
LLM_BACKEND=openai_compatible, LLM_BASE_URL=<mock_llm_url>, LLM_API_KEY=mock-api-key, LLM_MODEL=mock-model
DATABASE_BACKEND=libsql, LIBSQL_PATH=<tmpdir>/e2e.db
SANDBOX_ENABLED=false, ROUTINES_ENABLED=false, HEARTBEAT_ENABLED=false
EMBEDDING_ENABLED=false, SKILLS_ENABLED=true
ONBOARD_COMPLETED=true   # prevents setup wizard
```

The hosted OAuth refresh fixtures use the same baseline, but with their own DB/home tempdirs and `GOOGLE_OAUTH_CLIENT_ID=hosted-google-client-id` so hosted OAuth flows exercise proxy credential injection instead of the baked-in desktop Google app. Use `hosted_google_emulate_server` when the test needs Google WASM HTTP calls to hit the Emulate Google server seeded from `fixtures/emulate/google_gmail.yaml`.

For isolated v2 auth/prompt fixtures, do not rely on env-vs-DB precedence to
keep the mock LLM active. Pin the provider explicitly (typically by writing
`llm_backend=openai_compatible`, `openai_compatible_base_url=<mock_llm_url>`,
and `selected_model=mock-model` through `/api/settings/...`) so browser tests
exercise auth/activation behavior instead of silently falling back to NearAI.

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

`conftest.py` handles all server startup automatically — you do not need to start ironclaw manually before running `pytest`. The conftest builds the binary, starts the mock LLM, and starts ironclaw with a fresh temp database on every `pytest` invocation.

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
