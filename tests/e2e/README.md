# IronClaw Python E2E tests

The Python E2E suite exercises the shipping `ironclaw serve` process through
its WebChat v2, OpenAI-compatible, admin, extension, channel, and provider
surfaces. The retired gateway harness and its browser fixtures are not part of
this tree.

## Setup

From the repository root:

```bash
python3 -m venv tests/e2e/.venv
tests/e2e/.venv/bin/pip install -e tests/e2e
tests/e2e/.venv/bin/playwright install chromium
cargo build -p ironclaw --bin ironclaw
```

Provider-contract tests also require Node.js. CI uses a pinned Emulate checkout;
local runs can set `IRONCLAW_EMULATE_CLI` to an equivalent CLI.

## Run tests

```bash
# One scenario
tests/e2e/.venv/bin/pytest \
  tests/e2e/scenarios/test_reborn_webui_v2_smoke.py -v --timeout=120

# Retained product contracts migrated to the serve harness
grep -v '^#' tests/e2e/ironclaw_serve_e2e_tests.txt \
  | sed '/^[[:space:]]*$/d' \
  | xargs tests/e2e/.venv/bin/pytest -v --timeout=120

# Full current Python suite
tests/e2e/.venv/bin/pytest tests/e2e/scenarios -v
```

Set `HEADED=1` when debugging Playwright locally.

## Harness and fixtures

`reborn_webui_harness.py` owns the shared `ironclaw serve` lifecycle. It starts
the shipping binary on a reserved loopback address with isolated home,
workspace, configuration, and persistence paths, waits for `/api/health`, and
uses SIGINT for graceful teardown.

Use the `reborn_v2_*` fixtures exported by that module:

- `reborn_v2_server` for authenticated HTTP contract tests;
- `reborn_v2_page` and `reborn_v2_browser` for WebChat v2 browser tests;
- `reborn_v2_yolo_server` or `reborn_v2_yolo_page` only where the test
  explicitly covers unprompted capability execution;
- specialized restartable, private-install, or OpenAI-compatible fixtures only
  for the contract named by the scenario.

`mock_llm.py` provides deterministic OpenAI-compatible model responses.
`conftest.py` owns the shared binary build, mock LLM, Emulate services,
test-tool archives, and the fake Slack provider used by the retained channel
E2E.

## CI inventories

The suite has three explicit inventories:

- `reborn_coverage_tests.txt` selects the serve-backed coverage gate;
- `reborn_responses_e2e_tests.txt` selects the OpenAI-compatible API contract;
- `ironclaw_serve_e2e_tests.txt` preserves the migrated auth/OAuth,
  conversation/thread, and engine/tool/extension product contracts.

Validate the retained inventory and the deleted-binary boundary with:

```bash
python3 scripts/ci/check-ironclaw-serve-e2e-manifest.py
python3 scripts/ci/check-no-deleted-binary-refs.py
python3 scripts/ci/test-check-no-deleted-binary-refs.py
```

The final command deliberately creates a temporary bad workflow and verifies
that the static guard rejects it.

## Adding a scenario

1. Add `tests/e2e/scenarios/test_<contract>.py`.
2. Import the smallest suitable `reborn_v2_*` fixture from
   `reborn_webui_harness`.
3. Use `SEL_V2` for browser selectors and local route/provider doubles for
   deterministic external data.
4. Test through the caller when a classifier or helper gates a side effect.
5. Add the test to an inventory only when it belongs to that inventory's
   documented contract.
6. Run the scenario, the relevant inventory checker, and the deleted-binary
   guard.

## Emulate-backed provider coverage

Emulate fixtures cover provider behavior that maps to shipped features:

- Google Gmail, Calendar, and Drive stateful reads and writes;
- Slack channel, thread, DM, reaction, identity, and scope behavior;
- GitHub repository, issue, pull request, search, branch, release, fork, Git
  object, and Actions route behavior.

Provider-only contracts prove the fixture layer. Full-path tests must still
install and authenticate through IronClaw, drive the mediated product surface,
and verify provider-issued state with read-back.

## Test debt

See [E2E_DEBT.md](E2E_DEBT.md). Runtime skips are reserved for genuinely
optional external prerequisites; required serve-backed CI contracts should fail
instead of silently skipping.
