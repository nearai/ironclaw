# Python E2E agent guide

## Surface and ownership

All product E2E work in this directory targets the shipping `ironclaw serve`
binary. Do not recreate a second gateway launcher, duplicate server lifecycle
in a scenario, or add a compatibility binary.

The main owners are:

- `reborn_webui_harness.py`: serve lifecycle and `reborn_v2_*` fixtures;
- `conftest.py`: binary build, mock LLM, Emulate, test tools, and fake Slack;
- `helpers.py`: shared WebChat v2 selectors and HTTP helpers;
- `mock_llm.py`: deterministic model and OAuth-adjacent responses;
- `scenarios/`: caller-level product contracts;
- `*_e2e_tests.txt`: reviewed CI inventories.

Read `tests/e2e/README.md`, `.claude/rules/testing.md`, and
`tests/e2e/ironclaw_serve_e2e_tests.txt` before changing the harness or retained
coverage.

## Required invariants

- Bind test services to loopback on dynamically reserved ports.
- Give every serve process isolated home, workspace, config, and database paths.
- Never inherit real user credentials unless a live test explicitly requests a
  bounded child-only secret.
- Wait for `/api/health` with a bounded timeout.
- Stop servers with SIGINT so graceful teardown and coverage flushing run.
- Keep browser contexts function-scoped unless a contract specifically tests
  persistence across contexts.
- Use deterministic local providers or route interception for blocking CI.
- Verify side effects using durable or provider-issued read-back.

## Canonical test shape

HTTP contract:

```python
import httpx

from reborn_webui_harness import (
    REBORN_V2_AUTH_TOKEN,
    reborn_v2_server,  # noqa: F401 - imported fixture
)


async def test_example(reborn_v2_server):
    async with httpx.AsyncClient(
        headers={"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    ) as client:
        response = await client.get(
            f"{reborn_v2_server}/api/webchat/v2/session",
            timeout=15,
        )
    assert response.status_code == 200
```

Browser contract:

```python
from playwright.async_api import expect

from helpers import SEL_V2
from reborn_webui_harness import reborn_v2_page  # noqa: F401


async def test_example_ui(reborn_v2_page):
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible()
```

## Inventory rules

`ironclaw_serve_e2e_tests.txt` preserves three categories:

- auth/OAuth;
- conversation/thread;
- engine/tool/extension.

Every selected test must use a `reborn_v2_*` serve fixture. The manifest checker
validates category completeness, selectors, duplicates, and fixture routing.

`reborn_coverage_tests.txt` and `reborn_responses_e2e_tests.txt` are separate
contracts. Node IDs are allowed when only part of a scenario belongs in a
particular gate.

Run:

```bash
python3 scripts/ci/check-ironclaw-serve-e2e-manifest.py
python3 scripts/ci/check-no-deleted-binary-refs.py
python3 scripts/ci/test-check-no-deleted-binary-refs.py
```

The deleted-binary guard scans executable canary, E2E, and workflow files.
Its regression suite must demonstrate a non-zero result for a temporary
violation.

## Provider fixtures

The pinned Emulate fork is selected with `IRONCLAW_EMULATE_CLI`. Provider
contracts may use the shared Google, Slack, and GitHub fixtures. Full-path
contracts must additionally cross IronClaw installation, authentication,
capability mediation, and read-back.

The retained Slack channel scenario uses `fake_slack_server`; keep that fixture
separate from direct Emulate provider-contract coverage.

## Validation

Start narrow:

```bash
pytest tests/e2e/scenarios/test_<contract>.py -v --timeout=120
```

Then run the affected manifest. For changes to shared fixtures or collection,
collect and execute all selectors in `ironclaw_serve_e2e_tests.txt`. Run the
scope classifier regression suite when CI path rules change:

```bash
scripts/ci/test-classify-test-scope.sh
```

Do not claim live-provider coverage unless a live tier actually ran. Record
skips and unavailable optional dependencies explicitly.
