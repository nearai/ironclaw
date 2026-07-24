# Live Auth Canary

This runner starts the shipping Reborn `ironclaw` binary in a clean home and
verifies provider-backed auth through:

Use [scripts/live-canary/run.sh](../live-canary/run.sh)
as the top-level entrypoint for scheduled and manual lane dispatch. This file
documents the underlying executor for the `auth-live-seeded` lane.

- `/v1/responses`
- the browser gateway UI via Playwright

It uses the existing mock LLM from `tests/e2e/mock_llm.py` only for deterministic
tool selection. The thing under test is the real provider auth/runtime path,
not model behavior.

## What It Proves

- a brand-new machine can build and start the shipping Reborn binary
- manifest-declared tenant configuration is saved through the operator API
- install derives `setup_needed` when personal auth is missing
- completing the declared setup recipe derives `active` and publishes tools
- the Responses API can execute provider-backed tools
- the browser UI can execute provider-backed tools
- no public activation endpoint or legacy auth booleans are required

## Current Provider Cases

- `gmail`
  Configures `vendor.google`, then completes the declared Google OAuth recipe
  Runs through Responses API and browser
- `google_calendar`
  Installs package `google-calendar` and completes the declared Google OAuth recipe
  Runs through Responses API
- `github`
  Submits the PAT under the opaque manual-token requirement named by the manifest
  Runs through Responses API only (PAT-only — not browser-OAuth; the
  GitHub manifest declares `manual_token`)
- `notion`
  Runs only in browser mode because its manifest declares OAuth/DCR. The
  generic setup API intentionally does not accept a pre-seeded OAuth token.

## Setup

See the canonical live-canary account and credential guide in
[scripts/live-canary/ACCOUNTS.md](../live-canary/ACCOUNTS.md).

Copy the example config and fill in the real test credentials:

```bash
cd scripts/auth_live_canary
cp config.example.env config.env
set -a && source config.env && set +a
```

For seeded Google verification provide:

- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN`

along with:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`

The runner saves the client values through the generic `vendor.google` admin
configuration group. It then installs each package, starts the descriptor's
OAuth flow, and completes the Reborn callback against the deterministic token
endpoint. If `AUTH_LIVE_GOOGLE_REFRESH_TOKEN` is also set, the resulting account
includes it; no canary reaches into persistence or mutates an internal state
field.

### Browser-consent Google challenge bypass

When running `--mode browser` against Google, Google's risk engine will often
interrupt the Playwright login with a "Verify it's you" challenge that
`handle_google_popup` cannot solve. Bootstrap a `storage_state.json` once, and
the canary will skip the login (and the challenge) on subsequent runs:

```bash
python3 scripts/auth_live_canary/bootstrap_google_storage_state.py
# log into the dedicated test Google account in the window that opens,
# solve any challenges, then press Enter

export AUTH_BROWSER_GOOGLE_STORAGE_STATE_PATH=~/.ironclaw/auth-canary/google_storage_state.json
unset AUTH_BROWSER_GOOGLE_USERNAME AUTH_BROWSER_GOOGLE_PASSWORD
```

Re-run the bootstrap if browser-mode failures suggest the session has decayed.

## Usage

From the repo root:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --mode seeded
```

Run only selected providers:

```bash
python3 scripts/auth_live_canary/run_live_canary.py \
  --mode seeded \
  --case gmail \
  --case github
```

CI-style fresh-machine install:

```bash
python3 scripts/auth_live_canary/run_live_canary.py \
  --mode seeded \
  --playwright-install with-deps
```

Reuse an existing venv and binary:

```bash
python3 scripts/auth_live_canary/run_live_canary.py \
  --mode seeded \
  --skip-python-bootstrap \
  --skip-build
```

List the currently configured cases:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --mode seeded --list-cases
```

## Artifacts

The runner writes JSON results to:

```text
artifacts/auth-live-canary/<mode>/results.json
```

Browser failures also write screenshots into the same output directory.

## Important Boundary

This is the practical high-frequency live canary.

Seeded mode does **not** automate the provider login UI. It still crosses the
same generic Reborn install/setup/callback contracts as browser mode; its mock
token exchange returns the configured live Google test tokens. This catches:

- broken manifest/admin/setup plumbing
- regressions in derived lifecycle state or tool publication
- bad redirect/client config shipped with the runtime
- provider-side token validation changes
- tool execution regressions

Browser mode is the lower-frequency full provider-consent pass.
