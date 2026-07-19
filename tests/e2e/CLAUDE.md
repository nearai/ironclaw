# Canonical IronClaw E2E Guidance

This suite tests the shipping Reborn runtime exposed as `ironclaw`. The retired
v1 gateway, root package, `ironclaw-legacy` binary, and their fixtures must not
be reintroduced.

## Orientation

Regenerate the scenario inventory before editing:

```bash
find tests/e2e/scenarios -maxdepth 1 -name 'test_*.py' -print | sort
rg -n 'pytest .*tests/e2e/scenarios' .github/workflows scripts
```

Stable anchors:

- `conftest.py`: canonical binary, mock LLM, test-tool, and Emulate fixtures.
- `reborn_webui_harness.py`: `ironclaw serve`, browser, and bearer helpers.
- `helpers.py`: current SPA selectors, SSE helpers, and provider test tokens.
- `reborn_coverage_tests.txt`: coverage-gated binary scenarios.
- `reborn_responses_e2e_tests.txt`: OpenAI-compatible API inventory.

Verify these paths with:

```bash
test -f tests/e2e/conftest.py
test -f tests/e2e/reborn_webui_harness.py
test -f tests/e2e/helpers.py
```

## Rules

- Build `-p ironclaw_reborn_cli --bin ironclaw`; never add a second product
  binary name.
- Start the product with `ironclaw serve` through the shared harness.
- Use `reborn_v2_*` fixtures and `SEL_V2`; do not create generic `page`,
  `browser`, or `ironclaw_server` fixtures that hide which surface is tested.
- Keep API tests on current `/api/webchat/v2/*` or documented compatibility
  surfaces.
- Provider-only Emulate tests prove fixture behavior, not product wiring.
- A full-path provider test must cross the canonical runtime and verify the
  provider side effect with read-back evidence.
- Do not commit live credentials, browser storage state, PII, or provider
  responses containing secrets.

## Commands

```bash
cd tests/e2e
pytest scenarios/test_reborn_blackbox_smoke.py -v --timeout=120
pytest scenarios/test_reborn_webui_v2_smoke.py -v --timeout=120
pytest scenarios/test_reborn_responses_api.py -v --timeout=120
pytest scenarios/test_emulate_reborn_provider_contracts.py -v
pytest scenarios/ --collect-only -q
```

When changing workflow inventories, also run:

```bash
bash scripts/ci/check-e2e-matrix-files.sh
python3 scripts/ci/check-reborn-responses-e2e-manifest.py
```
