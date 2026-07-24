# IronClaw E2E Tests

Browser-level end-to-end tests for the IronClaw web gateway using Python + Playwright.

## Prerequisites

- Python 3.11+
- Rust toolchain (for building ironclaw)
- Chromium (installed via Playwright)
- Node.js with `npx` (for Emulate-backed provider fixtures)

## Setup

```bash
cd tests/e2e
pip install -e .
playwright install chromium
```

## Build ironclaw

The tests need the ironclaw binary built with libsql support:

```bash
cargo build -p ironclaw
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
| `test_sse_reconnect.py` | SSE reconnection handling, keepalive comments, restart recovery, stale reconnect IDs, and connection-limit coverage |
| `test_html_injection.py` | HTML injection security |
| `test_extensions.py` | Extensions tab: install, remove, configure, OAuth, auth card, activate |
| `test_oauth_refresh.py` | Hosted Gmail/MCP OAuth refresh; the Gmail path refreshes through the proxy and reads seeded Gmail data from Emulate |
| `test_emulate_reborn_provider_contracts.py` | Emulate provider contracts for Reborn-backed Google Gmail/Calendar/Drive reads, writes, missing resources, and account isolation; Slack QA 9/10 channel/thread/DM routing, strict-scope failures, profiles, mentions, and identity shapes; and GitHub identity, negative-result, repo/issue/PR/search/branch/git-object/release/fork/action-route surfaces |
| `test_provider_fault_proxy.py` | Self-tests the transparent provider fault proxy, reusable status/transport/response profiles, safe request ledger, reset behavior, and commit-then-disconnect semantics |
| `test_reborn_emulate_full_path.py` | Full-path IronClaw + Emulate coverage: install/auth extensions, drive scripted Gmail/Calendar/Drive/GitHub/Slack calls, assert provider-side state, and exercise GitHub→Slack, Calendar+Drive→Slack, Gmail→Slack, and Slack→Drive→Slack dispatch |

## Reborn coverage gate

The GitHub Actions Code Coverage workflow uses
`tests/e2e/reborn_coverage_tests.txt` instead of running every scenario in this
directory. That manifest is intentionally limited to tests that boot
`ironclaw serve` and cover the Reborn WebChat v2 or OpenAI-compatible
API surface. Legacy gateway tests and `ENGINE_V2=true` compatibility tests stay
in the E2E suite, but they are not part of the Reborn coverage gate. Manifest
entries may be pytest node IDs when only part of a broader scenario file belongs
in this gate.

## Adding new scenarios

1. Create `tests/e2e/scenarios/test_<name>.py`
2. Use the `page` fixture for a fresh browser page
3. Use selectors from `helpers.py` (update `SEL` dict if new elements are needed)
4. Keep tests deterministic -- use the mock LLM, not real providers

## Emulate-backed provider fixtures

Emulate coverage is intentionally limited to provider APIs that match Reborn
features already present in the codebase:

- Google: Gmail, Calendar, Drive, Docs, Sheets, and Slides seeded reads plus
  stateful message, event, file, document, spreadsheet, presentation, slide,
  text, shape, and image mutations with isolated accounts where applicable.
- Slack: auth, conversations, channel/thread/DM delivery, reactions, user
  lookup, membership, self-authored/last-sent identity, missing email/scope,
  mention encoding, two isolated DM targets, and exact-count readback.
- GitHub: authenticated user, repo create/list/metadata, fork list/create,
  release create/latest/list, issue create/read/comment/list/search, PR
  create/read/list/files/review/comment/merge, review-thread resolution,
  contents create/read/delete, search, branch/ref mutation, Git
  blob/tree/commit read/write, Actions workflow dispatch/readback/reruns, two
  repositories with distinct latest releases, and private-account isolation.

These expanded APIs come from the immutable `serrrfirat/emulate` fork pinned by
the Reborn E2E workflow; the unpinned local `emulate@0.7.0` fallback does not
provide all of them.

The direct provider-contract tests prove the emulator fixture layer. Full-path
Reborn + Emulate tests should use `hosted_google_emulate_server` or a matching
provider fixture, install/auth the extension through IronClaw, drive
`/api/chat/send` with the scripted mock LLM, and assert provider state through
Emulate readback.

The full-path QA runtime routes provider traffic through the transparent fault
proxy while keeping baseline and readback traffic direct to Emulate. Add common
failure shapes to `PROVIDER_FAULT_PROFILES`, then apply them to a representative
operation class in `PROVIDER_FAULT_CASES`; do not create the full Cartesian
product. For ambiguous writes, assert the proxy's forwarded/responded evidence
and direct provider mutation count so a lost acknowledgement cannot become a
false success or a duplicate mutation.

Do not duplicate account binding, refresh/reconnect, duplicate-inbound, or
repeated-delivery contracts as provider-only fixtures. Those are
caller/runtime properties and remain covered at their existing hermetic seams
(`runtime_credentials`, `gsuite_core`, `github_wasm_runtime_contract`,
`idempotent_replay`, trigger/outbound integration tests, and
`test_v2_auth_oauth_matrix.py`). Malformed and transport failures cross the
real caller path only for the representative operation-class matrix. Emulate
supplies provider state for full-path flows; it is neither an OAuth authority
nor the fault injector.

### Manual QA mapping

The Emulate provider contracts map to the manual QA sheet only where Emulate
can represent the backing provider API. Google Docs, Sheets, and Slides
operations now execute through their native extension routes instead of
Drive-style substitutes. Rows that also depend on model-authored routines or
live news remain only partially hermetic; 8B-8C still need a separate fake
HN/search endpoint. Telegram and Twitter/X rows 1A-1C are not covered by
Emulate.

## Live Persona Failure Notes

For the live 20+ turn persona workflows and recurring tool-misuse patterns seen
there, see [`LIVE_TOOL_FAILURES.md`](./LIVE_TOOL_FAILURES.md).

## Skip/xfail debt

For the current inventory of E2E skips/xfails and the policy for keeping browser
lifecycle tests deterministic, see [`E2E_DEBT.md`](./E2E_DEBT.md).

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

This is the pattern used in most of `test_tool_approval.py` and parts of
`test_extensions.py` (auth card, configure modal). The waiting-approval
regression in `test_tool_approval.py` uses a real tool call instead so it can
exercise backend approval state.
