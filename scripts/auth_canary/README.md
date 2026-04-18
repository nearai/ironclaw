# Auth Canary Runner

This runner bootstraps the auth E2E environment on a fresh machine and executes
focused end-to-end auth checks against an isolated local IronClaw instance.

Use [scripts/live-canary/run.sh](../live-canary/run.sh)
as the top-level entrypoint for scheduled and manual lane dispatch. This file
documents the underlying executor for the `auth-smoke`, `auth-full`, and
`auth-channels` lanes.

It is for:
- scheduled auth canaries on fresh CI runners
- manual pre-release auth verification
- validating that a clean machine can still build, launch the gateway, open a
  browser, complete hosted OAuth flows, and make authenticated tool calls

It is not the live-provider canary yet. This runner uses the existing mock-backed
auth matrix so failures point at our own auth/runtime regressions instead of
third-party provider drift.

## What It Covers

The default `smoke` profile runs:
- WASM tool OAuth round-trip through the HTTP chat/auth APIs
- MCP OAuth round-trip through the HTTP chat/auth APIs
- MCP OAuth round-trip through the browser UI
- multi-user MCP auth isolation through the browser UI

The `full` profile adds:
- provider and exchange failure paths
- chat-first and settings-first auth flows
- refresh-on-demand and refresh-on-start coverage

The `channels` profile runs:
- WASM channel OAuth round-trip through the HTTP auth APIs

`smoke` is the scheduled canary because it is the currently stable fresh-machine
signal. `full` and `channels` are kept as manual/diagnostic profiles until the
remaining flaky or broken cases are fixed.

## Requirements

- Rust toolchain with `cargo`
- Python 3.11+
- network access for `pip install` and Playwright browser download

For local developer runs, `playwright install chromium` is usually enough.
For fresh Ubuntu CI machines, use `--playwright-install with-deps`.

## Usage

From the repo root:

```bash
python3 scripts/auth_canary/run_canary.py
```

Run the full profile:

```bash
python3 scripts/auth_canary/run_canary.py --profile full
```

Run the channel-only diagnostic profile:

```bash
python3 scripts/auth_canary/run_canary.py --profile channels
```

CI-style fresh-machine install:

```bash
python3 scripts/auth_canary/run_canary.py --playwright-install with-deps
```

Reuse an existing venv and binary:

```bash
python3 scripts/auth_canary/run_canary.py \
  --skip-python-bootstrap \
  --skip-build
```

Pass extra pytest flags through:

```bash
python3 scripts/auth_canary/run_canary.py \
  --pytest-arg=-x \
  --pytest-arg=--maxfail=1
```

List the exact tests for a profile:

```bash
python3 scripts/auth_canary/run_canary.py --profile smoke --list-tests
```

## Artifacts

By default the runner writes JUnit output to:

```text
artifacts/auth-canary/auth-canary-junit.xml
```

Override with:

```bash
python3 scripts/auth_canary/run_canary.py --output-dir /tmp/auth-canary
```

## Fresh-Machine Flow

The runner does this in order:

1. create `tests/e2e/.venv` if needed
2. `pip install -e tests/e2e`
3. install Playwright Chromium
4. `cargo build --no-default-features --features libsql`
5. run the selected auth matrix tests

That makes it suitable for a clean CI VM or a brand-new dev box.
