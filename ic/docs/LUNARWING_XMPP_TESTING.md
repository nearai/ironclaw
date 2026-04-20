# LunarWing and XMPP Bridge Testing

This guide sets up an isolated local environment for the project currently
branded as LunarWing. Some binaries, environment variables, and service names
still use `ironclaw`; keep those names until the codebase is renamed.

The goal is to test in layers:

1. Build the LunarWing and bridge binaries.
2. Smoke-test the local bridge HTTP API and bearer-token enforcement.
3. Configure the bridge against a live XMPP account when needed.
4. Run LunarWing and the bridge under either the script harness or user systemd.

The `customic/` tree is intentionally not part of this workflow.

## Quick Start

From the repo's `ic/` directory:

```bash
scripts/lunarwing-xmpp-test-env.sh init
scripts/lunarwing-xmpp-test-env.sh build
scripts/lunarwing-xmpp-test-env.sh smoke
```

For a command-by-command setup walkthrough, see
[`testing/lunarwing-xmpp/README.md`](../testing/lunarwing-xmpp/README.md).

The default test root is `/tmp/lunarwing-xmpp-test`. Export an override when
you want the state to survive reboots:

```bash
export LUNARWING_TEST_ROOT="$HOME/.local/state/lunarwing-xmpp-test"
scripts/lunarwing-xmpp-test-env.sh init
```

Generated files:

- `env/lunarwing.env` for the LunarWing daemon.
- `env/xmpp-bridge.env` for the XMPP bridge.
- `state/` for the isolated `IRONCLAW_BASE_DIR`, libSQL database, and OMEMO
  store.
- `logs/` and `run/` for harness-managed processes.
- `systemd/` for generated user-service unit files.

The env files are created with mode `0600`. They may contain live XMPP passwords
and bridge tokens, so do not paste their contents into chat, issues, or logs.

## Bridge API Smoke Tests

The bridge can start without live XMPP credentials. This is useful for testing
the local HTTP sidecar boundary before touching an external XMPP account:

```bash
scripts/lunarwing-xmpp-test-env.sh start-bridge
scripts/lunarwing-xmpp-test-env.sh bridge-auth-check
scripts/lunarwing-xmpp-test-env.sh bridge-status
scripts/lunarwing-xmpp-test-env.sh stop-bridge
```

`bridge-auth-check` verifies two things:

- `GET /v1/status` without the bearer token is rejected.
- `GET /v1/status` with the generated bearer token succeeds.

The smoke test does the same sequence and stops the bridge unless
`LUNARWING_TEST_KEEP_BRIDGE=1` is set.

## Live XMPP Configuration

Edit the generated bridge env file:

```bash
${LUNARWING_TEST_ROOT:-/tmp/lunarwing-xmpp-test}/env/xmpp-bridge.env
```

Set at least:

- `XMPP_JID`
- `XMPP_PASSWORD`
- `XMPP_ALLOW_ROOMS_JSON`, or pass room JIDs as command arguments

Then configure the running bridge through the existing wrapper:

```bash
scripts/lunarwing-xmpp-test-env.sh start-bridge
scripts/lunarwing-xmpp-test-env.sh configure-bridge --show-status room@conference.example.org
```

For live outbound safety, use the rate-limit wrapper:

```bash
scripts/lunarwing-xmpp-test-env.sh rate-limit status
scripts/lunarwing-xmpp-test-env.sh rate-limit set 20 --reset
scripts/lunarwing-xmpp-test-env.sh rate-limit off
```

The rate-limit command changes the live bridge override only. Restarting the
bridge resets the override back to bridge configuration defaults.

## Running LunarWing Locally

The harness can start the current `ironclaw` binary with isolated state:

```bash
scripts/lunarwing-xmpp-test-env.sh start-lunarwing
scripts/lunarwing-xmpp-test-env.sh lunarwing-status
scripts/lunarwing-xmpp-test-env.sh stop-lunarwing
```

By default this runs:

```bash
target/debug/ironclaw --no-onboard run
```

Use your isolated `IRONCLAW_BASE_DIR` for onboarding or live credentials instead
of sharing `~/.ironclaw`:

```bash
set -a
. /tmp/lunarwing-xmpp-test/env/lunarwing.env
set +a
target/debug/ironclaw onboard
```

For alternate runtime flags, pass the full argument list after `--`:

```bash
scripts/lunarwing-xmpp-test-env.sh start-lunarwing -- --no-onboard --no-db run
```

## User Systemd Method

Generate user-service units from your current checkout and test root:

```bash
scripts/lunarwing-xmpp-test-env.sh render-systemd
mkdir -p ~/.config/systemd/user
cp /tmp/lunarwing-xmpp-test/systemd/*.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user start xmpp-bridge-test.service
systemctl --user start lunarwing-test.service
```

Use read-only diagnostics before restarting anything:

```bash
systemctl --user status xmpp-bridge-test.service
systemctl --user show xmpp-bridge-test.service
journalctl --user -u xmpp-bridge-test.service -n 100 --no-pager
scripts/lunarwing-xmpp-test-env.sh bridge-status
```

The generated bridge unit has `PartOf=lunarwing-test.service`, so LunarWing
service stops can also stop the bridge. Do not assume the bridge caused a
LunarWing stop just because both units changed state together.

## System Service Method

For a machine-level service, keep secrets in root-readable env files such as:

- `/etc/lunarwing/lunarwing.env`
- `/etc/lunarwing/xmpp-bridge.env`

Use `EnvironmentFile=` in the unit files instead of inline `Environment=`
entries for tokens or passwords. The bridge should remain loopback-bound:

```ini
XMPP_BRIDGE_BIND=127.0.0.1:8787
```

When adapting the generated user units for system services:

- Add a dedicated `User=` and `Group=`.
- Move writable state out of `/tmp`, for example to `/var/lib/lunarwing-test`.
- Keep the bridge ordered before LunarWing with `Wants=` and `After=`.
- Keep token auth enabled with `XMPP_BRIDGE_TOKEN`.
- Inspect `systemctl status`, `systemctl show`, `journalctl`, and
  `/v1/status` before restarting services.

## Troubleshooting

Run:

```bash
scripts/lunarwing-xmpp-test-env.sh doctor
scripts/lunarwing-xmpp-test-env.sh logs 120
```

Common checks:

- `curl` is required for bridge API calls.
- `jq` is required by the existing configure and rate-limit wrappers.
- `cargo` is required for `build`.
- `bridge-status` returning `401` or `403` usually means the wrong token or env
  file is being used.
- A running system bridge on port `8787` can conflict with the harness bridge.
  Change `XMPP_BRIDGE_BIND` in the test env file when needed.
