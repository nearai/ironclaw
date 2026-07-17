# Live Canary Local and GitHub Setup

This directory contains the unified local dispatcher and artifact helpers used
by several CI owners:

- `run.sh` dispatches named lanes and writes artifacts
- `scrub-artifacts.sh` scans artifacts before upload
- `upgrade-canary.sh` checks previous-release DB compatibility

The auth-focused Python runners remain the executors behind the auth lanes:

- `scripts/auth_canary/run_canary.py` — mock-backed pytest matrix (fresh-machine)
- `scripts/auth_live_canary/run_live_canary.py` — live-provider runner with two
  modes: `--mode seeded` (token persistence and refresh) and `--mode browser`
  (OAuth consent in Playwright)

Their shared auth canary setup, provider registry, and runtime helpers live in:

- `scripts/live_canary/common.py`
- `scripts/live_canary/auth_registry.py`
- `scripts/live_canary/auth_runtime.py`

Note on naming: `live-canary/` (this directory, hyphen) is the shell dispatcher
and operator-facing entrypoint; `live_canary/` (sibling, underscore) is the
Python package. The hyphen/underscore split follows Python's package-naming
convention — Python imports cannot contain hyphens.

Future auth providers should be added through the shared registry and account
guide, not by creating a new standalone runner shape.

Run commands from the repository root.

## What belongs in Live Canary

Live Canary is supplemental coverage for drift that requires a real external
service, live model, provider credential, or browser-consent surface. It does
not replace hermetic Reborn integration tests, mock auth/workflow coverage, or
recorded replay contracts. A scenario that can produce the same signal without
live infrastructure belongs in the deterministic owner instead.

## Workflow ownership

The local `run.sh` lane vocabulary is broader than any single GitHub workflow.
CI ownership is intentionally split by signal type:

| Owner | Signal | Lanes or tests |
| --- | --- | --- |
| `.github/workflows/live-canary.yml` (Live Canary) | Live external-provider drift only | `public-smoke`, `persona-rotating`, `private-oauth`, `provider-matrix`, `release-public-full`, `auth-live-seeded`, `auth-browser-consent`, `reborn-webui-v2-live-qa` |
| `.github/workflows/reborn-tests.yml` (Tests (Reborn)) | Hermetic PR/merge CI | Mock auth profiles `auth-smoke`, `auth-full`, `auth-channels`, plus `workflow-canary` |
| `.github/workflows/replay-gate.yml` (Replay Gate; workflow name `Replay Snapshot Gate`) | Deterministic recorded replay | Replay snapshot tests; not a Live Canary job |
| `.github/workflows/upgrade-compatibility.yml` (Upgrade Compatibility) | Manual previous/current compatibility | `upgrade-canary` through `run.sh` and `upgrade-canary.sh` |

This separation keeps mock/replay failures in required deterministic CI, live
provider failures in the drift workflow, and expensive compatibility checks
behind an explicit manual dispatch. The local dispatcher retains all lane names
for operator reproduction.

Upgrade Compatibility requires `previous_ref` and `current_ref`; the latter
defaults to `main`. GitHub checks out the resolved `current_ref`, then invokes
the retained local lane with `CURRENT_REF=HEAD` so the script tests that exact
checkout. Its scrubbed `upgrade-compatibility` artifact is retained for 30 days.

### Live auth lanes

- `auth-live-seeded`
- `auth-browser-consent`

### Reborn WebUI v2 QA lane

- `reborn-webui-v2-live-qa`

PR-targeted runs execute the reviewed PR binary with live integration secrets.
They must pass the `reborn-live-canary-pr` GitHub environment gate and have an
approving review for the exact PR head commit from a collaborator with write
access. Scheduled and manual default-branch runs do not require this PR gate.

## Local Commands

Run the public live smoke lane:

```bash
LANE=public-smoke scripts/live-canary/run.sh
```

Run the provider matrix lane:

```bash
LANE=provider-matrix \
PROVIDER=openai-compatible \
PROVIDER_TEST_TARGET=e2e_live_mission \
SCENARIO=mission_daily_news_digest_with_followup \
scripts/live-canary/run.sh
```

Run the private OAuth refresh lane on the dedicated `ironclaw-live` runner (or
an equivalently configured local host):

```bash
LANE=private-oauth scripts/live-canary/run.sh
```

Run the auth smoke lane:

```bash
LANE=auth-smoke scripts/live-canary/run.sh
```

Run the local deterministic convenience lane:

```bash
LANE=deterministic-replay scripts/live-canary/run.sh
```

That command is intentionally narrower than Replay Gate: it runs only the
ignored `e2e_live` target through `cargo test --features libsql`. To reproduce
Replay Gate exactly (with `cargo-insta` and `cargo-nextest` installed), run:

```bash
NEXTEST_PROFILE=ci cargo insta test \
  --check \
  --test-runner nextest \
  --no-default-features \
  --features "libsql,replay" \
  --test e2e_recorded_trace \
  --test e2e_live

if git ls-files 'tests/snapshots/*.snap.new' | grep .; then
  echo "Committed .snap.new files found — run 'cargo insta review' and commit the accepted .snap."
  exit 1
fi
```

Run the seeded auth live lane:

```bash
LANE=auth-live-seeded scripts/live-canary/run.sh
```

Run the browser-consent auth lane:

```bash
LANE=auth-browser-consent scripts/live-canary/run.sh
```

Run selected auth provider cases:

```bash
LANE=auth-live-seeded CASES=gmail,github scripts/live-canary/run.sh
LANE=auth-browser-consent CASES=google,notion scripts/live-canary/run.sh
# Browser cases: google, notion only. github is PAT-only (not OAuth) so
# it lives in auth-live-seeded instead — see scripts/live_canary/auth_registry.py.
```

Run the Reborn WebUI v2 live QA lane against the local copied Reborn home:

```bash
LANE=reborn-webui-v2-live-qa \
REBORN_WEBUI_V2_LIVE_QA_HOME=/tmp/ironclaw-reborn-real-slack \
scripts/live-canary/run.sh
```

Run the full manual/local non-Telegram Reborn target (47 implemented cases):

```bash
LANE=reborn-webui-v2-live-qa CASES=all scripts/live-canary/run.sh
```

The GitHub 3-hour schedule is consolidated to 39 cases: it retains one live
connection journey per integration and omits eight redundant connection-only
journeys. The default GitHub manual dispatch uses that same 39-case matrix;
use the local command above when a full manual 47-case sweep is required.

Use CI-style browser installation for auth browser lanes:

```bash
LANE=auth-browser-consent PLAYWRIGHT_INSTALL=with-deps scripts/live-canary/run.sh
```

Reuse an existing build and Python environment:

```bash
LANE=auth-smoke SKIP_BUILD=1 SKIP_PYTHON_BOOTSTRAP=1 scripts/live-canary/run.sh
```

Run an upgrade canary:

```bash
LANE=upgrade-canary \
PREVIOUS_REF=v0.1.2 \
CURRENT_REF=HEAD \
scripts/live-canary/run.sh
```

Artifacts are written under:

```text
artifacts/live-canary/<lane>/<provider>/<timestamp>/
```

## Reborn QA signal semantics

The Reborn WebUI v2 report separates three health signals. For example:

```text
Contracts: 44/44 passed
Behavioral quality: 1/3 passed, 2 warnings
Infrastructure/preconditions: 0 inconclusive
```

The reporter omits the infrastructure/preconditions line when the value is
zero; it is shown here to make the three-signal interpretation explicit.

- Current runner-emitted contract cases are blocking; current behavioral cases
  are nonblocking warnings, so model variance stays visible without
  masquerading as a product-contract regression.
- For a valid `case_tier`, the notifier normalizes `blocking` independently: a
  boolean value is preserved and a missing or nonboolean value becomes `true`.
  A missing or invalid tier fails closed atomically as `case_tier=contract` and
  `blocking=true`, even when the entry supplies `blocking=false` or
  inconclusive-looking metadata.
- An unsuccessful result with a valid tier is inconclusive only when it is
  explicitly typed with
  `failure_class=infrastructure`, `failure_class=precondition`,
  `failure_status=inconclusive`, or `inconclusive=true`. Examples include the
  stale Slack search index, a terminal model-provider incident, and a
  durable-evidence read error. Successful entries ignore stale failure
  metadata.
- Case-owned credential, setup, and fixture checks emit explicit precondition
  inconclusives with
  `failure_status=inconclusive`, `inconclusive=true`, and `blocking=false`.
  They remain unsuccessful for diagnostics but do not enter contract or
  behavioral totals. Generic errors outside those case-owned paths remain
  ordinary lane failures.

JUnit-only lanes treat skipped tests as non-executed: skips remain visible in
the execution counts but are excluded from the contract denominator. An
all-skipped lane is reported as `skip`.

Any combined `succeeded of total` line is execution detail only. Use the tiered
lines above as the primary health signal.

For `qa_10g_slack_last_message_sent_global`, the harness first seeds a unique
Slack marker and polls workspace search for bounded index freshness (90 seconds
by default). A stale index, exception, or malformed observation returns an
inconclusive result before any model call. Persisted preflight metadata is
bounded to `indexed`, `attempts`, `latency_ms`, and an optional sanitized,
240-character `last_error`.

Each Reborn QA case that passes preparation plus the pre-server credential and
delivery-target checks starts `ironclaw-reborn` with a new ephemeral agent
working directory outside both the checkout and artifact tree. Those early
preflight incidents do not start a server or allocate a workspace. The Slack
setup API and fixture checks that require the live server run after startup and
before the model call or product side effect they guard. Routine delivery
probes also validate their created fixture before judging delivery. Incidents
at these typed checkpoints emit nonblocking precondition inconclusives and
enter normal server/workspace cleanup. Cases short-circuited after a terminal
provider incident do not run. For a started case, the harness stops the server,
exports its trace while the context is still live, and only then removes the
workspace.
Routine creation passes only after a structurally
final assistant reply and a new durable `trigger_record`; Slack correctness
probes bind expected terminal capability evidence to the current turn/run
rather than trusting response prose alone.

The `persona-rotating` environment summary records only integration names in
`persona_credentials_configured` and `persona_credentials_fallback` (`github`,
`google`, `slack`, `telegram`, `composio`). A configured credential is merely
available to the harness if the model selects that integration; the summary
does not prove an external call or provider success. Provider coverage requires
provider-issued evidence and readback in a dedicated probe. The summary never
writes credential values.

## Secrets And Account Material

Public live LLM lane secrets and variables are documented in
[docs/internal/live-canary.md](../../docs/internal/live-canary.md).

Seeded auth live-provider credentials:

- [scripts/live-canary/ACCOUNTS.md](ACCOUNTS.md)

For Reborn Slack personal-auth coverage, `AUTH_LIVE_SLACK_ACCESS_TOKEN` must be
a real Slack user token. The harness validates it with Slack `auth.test` before
encrypting the token and seeding the configured `slack_personal` product-auth
account; a failed validation blocks the dependent cases.

## GitHub Workflow

GitHub Actions uses `.github/workflows/live-canary.yml` only for scheduled or
manual live drift checks. Mock auth and workflow scenarios run in Tests
(Reborn), deterministic replay runs in Replay Gate, and upgrade
compatibility has its own manual workflow. Do not add hermetic or compatibility
jobs back to Live Canary merely because they use this directory's dispatcher.
