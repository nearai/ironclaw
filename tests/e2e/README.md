# IronClaw E2E Tests

Python and Playwright tests for the canonical `ironclaw` runtime.

## Setup

```bash
cd tests/e2e
python -m venv .venv
source .venv/bin/activate
pip install -e .
playwright install chromium
```

Emulate provider-contract tests also require Node.js with `npx`.

## Run

```bash
pytest scenarios/test_reborn_blackbox_smoke.py -v --timeout=120
pytest scenarios/test_reborn_webui_v2_smoke.py -v --timeout=120
pytest scenarios/test_emulate_reborn_provider_contracts.py -v
```

The binary fixtures build `-p ironclaw_reborn_cli --bin ironclaw`. WebUI tests
start `ironclaw serve` through `reborn_webui_harness.py`; Responses tests enable
`openai-compat-beta`. There is no v1 gateway fixture or compatibility binary.

## Coverage

- `reborn_coverage_tests.txt` selects the canonical binary tests used by the
  coverage workflow.
- `reborn_responses_e2e_tests.txt` selects the OpenAI-compatible API cases.
- `test_emulate_reborn_provider_contracts.py` validates Google, Slack, and
  GitHub provider fixtures without claiming a product-runtime full path.

The former full-path Emulate suite started the retired v1 gateway despite its
Reborn name. A replacement must use `start_reborn_webui_v2_server` and current
`/api/webchat/v2/*` contracts before it is added back.

## Adding Tests

1. Use fixtures from `reborn_webui_harness.py`.
2. Use `SEL_V2` for browser selectors.
3. Use the mock LLM or Emulate; merge-gating tests must be hermetic.
4. Add the scenario to the appropriate workflow or coverage manifest.
5. Run `bash scripts/ci/check-e2e-matrix-files.sh` after changing workflow
   scenario paths.
