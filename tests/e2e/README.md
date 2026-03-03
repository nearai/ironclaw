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

## Adding new scenarios

1. Create `tests/e2e/scenarios/test_<name>.py`
2. Use the `page` fixture for a fresh browser page
3. Use selectors from `helpers.py` (update `SEL` dict if new elements are needed)
4. Keep tests deterministic -- use the mock LLM, not real providers
