# Live Canary Regression Lanes

Live Canary is supplemental drift detection. It is eligible only when the
signal depends on a real external service, live model/provider, provider
credential, or browser-consent surface. Hermetic integration suites and
recorded traces remain the blocking contract path; a green live canary does not
replace them, and a deterministic scenario must not be moved into Live Canary
solely because `scripts/live-canary/run.sh` can dispatch it.

The workflow owners are:

- `.github/workflows/live-canary.yml` — eight live-only dispatch lanes;
- `.github/workflows/reborn-tests.yml` — Tests (Reborn), including mock auth
  and hermetic workflow suites;
- `.github/workflows/replay-gate.yml` — Replay Gate (workflow name `Replay
  Snapshot Gate`) for deterministic recorded replay;
- `.github/workflows/upgrade-compatibility.yml` — manual Upgrade
  Compatibility runs;
- `scripts/live-canary/run.sh` — the broader local dispatcher shared by those
  owners.

The artifact and reporting helpers are
`scripts/live-canary/scrub-artifacts.sh`,
`scripts/live-canary/notify_slack.py`, and
`scripts/live-canary/upgrade-canary.sh`. Auth executors live at
`scripts/auth_canary/run_canary.py` and
`scripts/auth_live_canary/run_live_canary.py`; their shared Python package is
`scripts/live_canary/` (underscore).

## GitHub Live Canary lanes

These are the exact Live Canary `workflow_dispatch` choices, in addition to
the convenience choice `all`:

| Lane | Live signal | GitHub trigger |
| --- | --- | --- |
| `public-smoke` | Real Anthropic model with public tool and mission journeys | Manual |
| `persona-rotating` | Real-model multi-turn persona and workspace behavior; optional third-party credentials are available but do not prove provider success | Manual |
| `private-oauth` | Transparent OAuth refresh against a dedicated account on the `ironclaw-live` self-hosted runner | Manual |
| `provider-matrix` | Equivalent live behavior through Anthropic and OpenAI-compatible adapters | Manual |
| `release-public-full` | Full public live suite for release candidates | Manual |
| `auth-live-seeded` | Google, GitHub, and Notion provider checks with seeded account material and a clean database | Manual |
| `auth-browser-consent` | Google and Notion OAuth consent through live provider login UIs and Playwright | Manual |
| `reborn-webui-v2-live-qa` | Reborn browser, model, Slack, Google, GitHub, and automation drift | Every three hours and manual |

The cron intentionally runs only `reborn-webui-v2-live-qa`. Its consolidated
matrix has 39 cases and keeps one live connection journey per integration,
removing eight redundant connection-only journeys from the schedule. The
default GitHub manual dispatch uses the same matrix. A full manual/local
non-Telegram run through `LANE=reborn-webui-v2-live-qa CASES=all` selects all
47 implemented cases.

PR-targeted Reborn live QA requires the `reborn-live-canary-pr` environment and
either an approving review for the exact head SHA from a collaborator with
write access or an authorized maintainer trigger. Forked PRs and mismatched
head SHAs are rejected before live secrets are exposed.

## Deterministic and release owners

The local dispatcher retains several names for reproduction, but they are not
Live Canary GitHub dispatch options:

- Tests (Reborn) owns the mock-backed `auth-smoke`, `auth-full`, and
  `auth-channels` profiles plus the hermetic `workflow-canary` suite. These jobs
  participate in the `Tests (Reborn)` roll-up.
- Replay Gate owns deterministic replay of the committed `e2e_recorded_trace`
  and `e2e_live` snapshots with the `libsql,replay` feature set.
- Upgrade Compatibility owns `upgrade-canary`. It is manual-only and requires
  dedicated `previous_ref` and `current_ref` inputs; `current_ref` defaults to
  `main`. It checks out the resolved current ref with tags/history and passes
  that exact checkout to `upgrade-canary.sh` as `CURRENT_REF=HEAD`. The workflow
  scrubs the output and uploads the dedicated `upgrade-compatibility` artifact
  for 30 days.

This ownership keeps hermetic failures in required deterministic CI, live
provider drift in Live Canary, and the slower previous/current database check
behind an explicit release operation.

## Report health semantics

Reborn WebUI v2 live QA reports independent contract, behavioral-quality, and
infrastructure/precondition signals. For example:

```text
Contracts: 44/44 passed
Behavioral quality: 1/3 passed, 2 warnings
Infrastructure/preconditions: 0 inconclusive
```

The renderer omits the infrastructure/preconditions line when its count is
zero; the example includes it to state the three-signal interpretation
explicitly.

- **Runner-emitted case policy:** current contract cases are blocking; current
  behavioral cases explicitly carry `blocking=false`, so a failure is a warning
  rather than a contract regression.
- **Notifier normalization:** for a valid `case_tier`, a boolean `blocking`
  value is preserved and a missing or nonboolean value becomes `true`. A
  missing or invalid tier fails closed atomically as `case_tier=contract` and
  `blocking=true`, even when the entry supplies `blocking=false` or
  inconclusive-looking metadata.
- **Infrastructure/preconditions:** an unsuccessful result with a valid tier is
  inconclusive only when it explicitly carries `failure_class=infrastructure`,
  `failure_class=precondition`, `failure_status=inconclusive`, or
  `inconclusive=true`. Examples are a stale Slack search index, a terminal
  model-provider incident, and a durable-evidence read error. These results do
  not increment contract or behavioral totals. Successful entries ignore stale
  failure metadata.
- Case-owned credential, setup, and fixture checks emit explicit precondition
  inconclusives with
  `failure_status=inconclusive`, `inconclusive=true`, and `blocking=false`.
  They remain unsuccessful for diagnostics but do not enter contract or
  behavioral totals. Generic errors outside those case-owned paths remain
  ordinary lane failures.

Slack and GitHub summaries may also show an aggregate execution count such as
`succeeded of total`. That combined number is secondary execution detail; it
must not be used in place of the three tier-specific health lines.

Older JUnit-only lanes are treated as contract results, but skipped tests are
non-executed and excluded from the contract denominator; an all-skipped lane is
reported as `skip`. For structured results, the notifier applies the
inconclusive rule only after validating the product tier. A valid tier retains
its independently normalized blocking value; an invalid tier uses the atomic
fail-closed contract policy above.

## Reborn WebUI v2 operating guarantees

### Slack global-search freshness

`qa_10g_slack_last_message_sent_global` tests workspace-global recall, where
Slack search indexing can lag. Before asking the model, the probe:

1. posts a unique marker to the dedicated personal-token DM fixture;
2. polls Slack workspace search until that marker is visible or the bounded
   deadline expires (`REBORN_WEBUI_V2_LIVE_QA_SLACK_INDEX_TIMEOUT_SECONDS`, 90
   seconds by default);
3. invokes the model only after the fresh marker is observable.

A stale index, timeout, exception, or malformed search observation returns an
infrastructure/precondition result with `inconclusive=true` and
`blocking=false`; no model call is made. Artifact metadata is intentionally
bounded to `indexed`, `attempts`, `latency_ms`, and an optional sanitized
`last_error`. Errors are whitespace-normalized, capped at 240 characters, and
redact query secrets plus Slack user/conversation identifiers.

### Agent workspace isolation

Only a selected Reborn QA case that passes preparation plus its pre-server
credential and delivery-target checks reaches server execution and gets a newly
created ephemeral working directory. Those early preflight incidents start no
server and allocate no workspace. The Slack setup API and fixture checks that
require the live server happen after startup and before the model call or
product side effect they guard. Routine delivery probes also validate their
created fixture before judging delivery. Incidents at these typed checkpoints
emit nonblocking precondition inconclusives and enter normal server/workspace
cleanup. After a terminal provider incident, the remaining selected cases are
recorded as inconclusive without being run or allocated workspaces.

For a case that starts, the harness rejects a working directory inside either
the repository checkout or the artifact output tree and passes the isolated
path as the server process `cwd`. Its `finally` path stops the server, exports
the case trace while the temporary context still exists, and then leaves the
context so the workspace is removed. The working directory therefore cannot
consume the repository's files as ambient agent state or contaminate uploaded
artifacts.

### Durable routine and capability evidence

Routine creation does not pass on plausible response text. The harness first
requires a structurally final assistant reply, then polls the Reborn database
for a new durable `trigger_record` after the pre-turn baseline (up to 120
seconds). A final reply without the new durable record is a failure.

Slack correctness probes capture the submitted message/thread/turn/run
identity and read terminal capability activity plus run-state records from the
Reborn database. Expected capabilities, terminal statuses, order, and selected
arguments are bound to the current turn. A reply without the expected durable
capability evidence fails; an evidence read error is infrastructure
inconclusive. Persisted result details redact Slack entity IDs and omit the
full response body.

### Persona credential configuration

The rotating persona lane verifies live-model multi-turn persona and workspace
behavior. GitHub, Google, Slack, Telegram, and Composio credentials can be made
available to its harness. Its `env-summary.txt` contains only name-level
configuration:

```text
persona_credentials_configured=github,slack
persona_credentials_fallback=google,telegram,composio
```

The exact names vary with configured secrets. Credential values are never
written. An absent or empty credential uses the harness's dummy fallback so
persona behavior can still run. A configured credential is only available if
the model selects that integration, and a passing persona case does not prove
an external provider call or success. Provider coverage requires
provider-issued evidence and readback in a dedicated probe.

## Required repository configuration

### Public model lanes

The public Anthropic lanes use secret `ANTHROPIC_API_KEY` and variable
`LIVE_ANTHROPIC_MODEL`. The OpenAI-compatible provider-matrix arm uses secret
`LIVE_OPENAI_COMPATIBLE_API_KEY` and variables
`LIVE_OPENAI_COMPATIBLE_BASE_URL` and `LIVE_OPENAI_COMPATIBLE_MODEL`.

### Live auth lanes

Seeded and browser-consent account material is documented in
[`scripts/live-canary/ACCOUNTS.md`](../../scripts/live-canary/ACCOUNTS.md).
Seeded coverage includes Google access/refresh material, a GitHub PAT and
stable issue fixture, and Notion token/query material. Browser consent uses
Google and Notion storage state. GitHub is PAT-only in the released tool and
therefore belongs to `auth-live-seeded`, not `auth-browser-consent`.

### Reborn WebUI v2 Slack lane

The generated Reborn config enables Slack without writing retired legacy setup
fields:

```toml
[slack]
enabled = true
```

After `ironclaw-reborn serve` starts, the harness calls
`PUT /api/webchat/v2/channels/slack/setup` with the WebUI operator bearer token.
Required repository variables are:

- `REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID`
- `REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID`
- `REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID`
- `REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID`

Required secrets are:

- `IRONCLAW_REBORN_SLACK_SIGNING_SECRET`
- `IRONCLAW_REBORN_SLACK_BOT_TOKEN`
- `AUTH_LIVE_SLACK_ACCESS_TOKEN`

`AUTH_LIVE_SLACK_ACCESS_TOKEN` must be a real Slack user token for the live QA
identity. Before writing any account state, the harness calls Slack
`auth.test`; only a successful response with the user/team identity proceeds to
encrypt the token and seed a configured `slack_personal` product-auth account.
A failed validation leaves the account unseeded and blocks dependent cases.

The personal OAuth connect probes additionally require variable
`REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID` and secret
`REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET`. An optional
`AUTH_LIVE_SLACK_SECOND_USER_TOKEN` arms probes that need a second human Slack
identity.

## Local reproduction

Run public live smoke:

```bash
IRONCLAW_LIVE_TEST=1 \
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=... \
LANE=public-smoke \
scripts/live-canary/run.sh
```

Run mock auth or hermetic workflow owners locally through the shared
dispatcher:

```bash
LANE=auth-smoke scripts/live-canary/run.sh
LANE=workflow-canary scripts/live-canary/run.sh
LANE=deterministic-replay scripts/live-canary/run.sh
```

These commands are local reproduction support; they do not make the lanes Live
Canary GitHub dispatch options.

The local `deterministic-replay` convenience is narrower than Replay Gate: it
runs only ignored `e2e_live` tests through `cargo test --features libsql`. For
an exact Replay Gate reproduction, install `cargo-insta` and `cargo-nextest`,
then run:

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

Run private OAuth refresh on the dedicated `ironclaw-live` runner or an
equivalently configured host:

```bash
LANE=private-oauth scripts/live-canary/run.sh
```

Run the two live auth lanes:

```bash
LANE=auth-live-seeded CASES=gmail,github scripts/live-canary/run.sh
LANE=auth-browser-consent CASES=google,notion scripts/live-canary/run.sh
```

Run Reborn QA locally:

```bash
LANE=reborn-webui-v2-live-qa \
REBORN_WEBUI_V2_LIVE_QA_HOME=/tmp/ironclaw-reborn-real-slack \
scripts/live-canary/run.sh

LANE=reborn-webui-v2-live-qa CASES=all scripts/live-canary/run.sh
```

Run Upgrade Compatibility locally through the retained dispatcher lane:

```bash
LANE=upgrade-canary \
PREVIOUS_REF=v0.1.2 \
CURRENT_REF=HEAD \
scripts/live-canary/run.sh
```

## Artifact policy

Local artifacts are written under
`artifacts/live-canary/<lane>/<provider>/<timestamp>/`. GitHub jobs run
`scripts/live-canary/scrub-artifacts.sh` before upload. Browser and seeded-auth
artifacts may include screenshots and bounded JSON evidence, but must not
contain long-lived credentials or raw provider payloads. Private OAuth uploads
only scrubbed summaries and structured outcomes.
