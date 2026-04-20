# LunarWing XMPP Test Environment Setup

This is the practical setup guide for the local LunarWing and XMPP bridge test
environment created by `scripts/lunarwing-xmpp-test-env.sh`.

The test environment is isolated from your normal `~/.ironclaw` state. By
default it lives at:

```bash
/tmp/lunarwing-xmpp-test
```

Use this harness when you want to test:

- the `xmpp-bridge` sidecar HTTP API
- bearer-token enforcement on the bridge
- live XMPP bridge configuration
- LunarWing running against isolated state
- user-systemd service behavior

The `customic/` tree is not part of this setup.

## 1. Pick a Test Root

Use the default `/tmp` root for throwaway tests:

```bash
cd /home/cmc/lunarwing/ic
scripts/lunarwing-xmpp-test-env.sh init
```

Use a persistent root when you want the same env, logs, database, and OMEMO
store after reboot:

```bash
cd /home/cmc/lunarwing/ic
export LUNARWING_TEST_ROOT="$HOME/.local/state/lunarwing-xmpp-test"
scripts/lunarwing-xmpp-test-env.sh init
```

Keep that `LUNARWING_TEST_ROOT` exported for later commands in the same shell.
If you open a new shell, export it again before using the harness.

## 2. Inspect the Generated Files

After `init`, the harness creates:

```text
$LUNARWING_TEST_ROOT/
  env/
    lunarwing.env
    xmpp-bridge.env
  logs/
  run/
  state/
    xmpp/
  systemd/
```

If `LUNARWING_TEST_ROOT` is not set, replace it with
`/tmp/lunarwing-xmpp-test` in the paths above.

The env files are mode `0600`. They may contain tokens and XMPP passwords.
Do not paste their contents into chat, issues, logs, or command output.

Use the example files in this directory only as references:

- `testing/lunarwing-xmpp/lunarwing.env.example`
- `testing/lunarwing-xmpp/xmpp-bridge.env.example`

Edit the generated env files, not the examples.

## 3. Build the Test Binaries

Build both the current LunarWing binary and the bridge:

```bash
scripts/lunarwing-xmpp-test-env.sh build
```

This builds:

```text
target/debug/ironclaw
bridges/xmpp-bridge/target/debug/xmpp-bridge
```

The binary is still named `ironclaw` in the current codebase, even though the
project is now called LunarWing.

For release binaries:

```bash
export LUNARWING_TEST_PROFILE=release
scripts/lunarwing-xmpp-test-env.sh build
```

Use the same `LUNARWING_TEST_PROFILE` for later `start-*`, `smoke`, and
`render-systemd` commands.

## Service Names

The harness intentionally renders test services by default:

```text
lunarwing-test.service
xmpp-bridge-test.service
```

Those names avoid clobbering a real user or system service named
`lunarwing.service`, `ironclaw.service`, or `xmpp-bridge.service`.

When you want the generated units to use the real LunarWing name, set both
service-name variables before rendering:

```bash
export LUNARWING_TEST_SERVICE_NAME=lunarwing.service
export LUNARWING_TEST_BRIDGE_SERVICE_NAME=xmpp-bridge.service
scripts/lunarwing-xmpp-test-env.sh render-systemd
```

The harness uses the same names everywhere it generates dependencies:

- the bridge unit gets `PartOf=$LUNARWING_TEST_SERVICE_NAME`
- the LunarWing unit gets `Wants=` and `After=` for
  `$LUNARWING_TEST_BRIDGE_SERVICE_NAME`
- `configure-bridge --restart` passes the bridge service name through
  `XMPP_BRIDGE_SERVICE`

For generated user services, `configure-bridge --restart` uses
`systemctl --user` by default. For a machine-level service, set:

```bash
export LUNARWING_TEST_SYSTEMCTL_SCOPE=system
```

If you use the watchdog with a renamed service, point it at the same main unit:

```bash
IRONCLAW_WATCHDOG_SERVICE=lunarwing.service scripts/ironclaw-watchdog.sh
```

## 4. Run the Local Bridge Smoke Test

The bridge does not need live XMPP credentials for API smoke testing.

```bash
scripts/lunarwing-xmpp-test-env.sh smoke
```

That command:

1. starts `xmpp-bridge` if it is not already running
2. calls `/v1/status` without a bearer token and expects rejection
3. calls `/v1/status` with the generated token and expects success
4. prints bridge status
5. stops the bridge unless `LUNARWING_TEST_KEEP_BRIDGE=1` is set

To keep the bridge running after the smoke test:

```bash
LUNARWING_TEST_KEEP_BRIDGE=1 scripts/lunarwing-xmpp-test-env.sh smoke
```

Manual bridge commands:

```bash
scripts/lunarwing-xmpp-test-env.sh start-bridge
scripts/lunarwing-xmpp-test-env.sh bridge-auth-check
scripts/lunarwing-xmpp-test-env.sh bridge-status
scripts/lunarwing-xmpp-test-env.sh stop-bridge
```

## 5. Configure Live XMPP

Only do this after the local bridge smoke test passes.

Open the generated bridge env file:

```bash
${EDITOR:-nano} "${LUNARWING_TEST_ROOT:-/tmp/lunarwing-xmpp-test}/env/xmpp-bridge.env"
```

Set:

```bash
XMPP_JID=your-account@example.org
XMPP_PASSWORD=your-xmpp-password
XMPP_ALLOW_ROOMS_JSON=["room@conference.example.org"]
```

Optional but useful:

```bash
XMPP_ALLOW_FROM_JSON=["trusted-user@example.org"]
XMPP_DM_POLICY=allowlist
XMPP_ENCRYPTED_ROOMS_JSON=[]
XMPP_RESOURCE=lunarwing-test
```

Start the bridge and apply the live config:

```bash
scripts/lunarwing-xmpp-test-env.sh start-bridge
scripts/lunarwing-xmpp-test-env.sh configure-bridge --show-status room@conference.example.org
```

If you use `XMPP_ALLOW_ROOMS_JSON` instead of command-line room args:

```bash
scripts/lunarwing-xmpp-test-env.sh configure-bridge --show-status
```

Check status:

```bash
scripts/lunarwing-xmpp-test-env.sh bridge-status
```

## 6. Set a Safe Outbound Rate Limit

Before sending live test traffic, set a conservative outbound cap:

```bash
scripts/lunarwing-xmpp-test-env.sh rate-limit status
scripts/lunarwing-xmpp-test-env.sh rate-limit set 20 --reset
```

To pause outbound sends:

```bash
scripts/lunarwing-xmpp-test-env.sh rate-limit off --reset
```

To clear the rolling counter while keeping the current cap:

```bash
scripts/lunarwing-xmpp-test-env.sh rate-limit reset
```

The live override resets when `xmpp-bridge` restarts.

## 7. Run LunarWing Against the Test State

If you only need bridge testing, skip this section.

The harness starts LunarWing with the isolated `IRONCLAW_BASE_DIR` from
`env/lunarwing.env`:

```bash
scripts/lunarwing-xmpp-test-env.sh start-lunarwing
scripts/lunarwing-xmpp-test-env.sh lunarwing-status
```

Stop it with:

```bash
scripts/lunarwing-xmpp-test-env.sh stop-lunarwing
```

Default command:

```bash
target/debug/ironclaw --no-onboard run
```

If LunarWing needs onboarding or provider credentials, run onboarding against
the isolated env:

```bash
set -a
. "${LUNARWING_TEST_ROOT:-/tmp/lunarwing-xmpp-test}/env/lunarwing.env"
set +a
target/debug/ironclaw onboard
```

Then start LunarWing again through the harness.

To pass a custom command line:

```bash
scripts/lunarwing-xmpp-test-env.sh start-lunarwing -- --no-onboard --no-db run
```

## 8. Generate User-Systemd Units

Generate test units using the current checkout, test root, and profile:

```bash
scripts/lunarwing-xmpp-test-env.sh render-systemd
```

Install them for your user:

```bash
mkdir -p ~/.config/systemd/user
cp "${LUNARWING_TEST_ROOT:-/tmp/lunarwing-xmpp-test}/systemd/"*.service ~/.config/systemd/user/
systemctl --user daemon-reload
```

Start the bridge service first:

```bash
systemctl --user start xmpp-bridge-test.service
systemctl --user status xmpp-bridge-test.service
```

Then start LunarWing:

```bash
systemctl --user start lunarwing-test.service
systemctl --user status lunarwing-test.service
```

Use read-only diagnostics first:

```bash
systemctl --user show xmpp-bridge-test.service
journalctl --user -u xmpp-bridge-test.service -n 100 --no-pager
scripts/lunarwing-xmpp-test-env.sh bridge-status
```

Stop services:

```bash
systemctl --user stop lunarwing-test.service
systemctl --user stop xmpp-bridge-test.service
```

The bridge unit has `PartOf=lunarwing-test.service`, so LunarWing service stops
can also stop the bridge.

The built-in Rust service installer is separate from this test renderer. Running
`ironclaw service install` now installs the user unit as `lunarwing.service` and
attempts to disable the legacy `ironclaw.service` user unit when it exists.

For production system services, use the committed templates instead of the
generated user-service units:

```bash
sudo install -o root -g root -m 0644 systemd/xmpp-bridge.service /etc/systemd/system/xmpp-bridge.service
sudo install -o root -g root -m 0644 systemd/lunarwing.service /etc/systemd/system/lunarwing.service
sudo systemctl daemon-reload
sudo systemctl enable --now xmpp-bridge.service
sudo systemctl enable --now lunarwing.service
```

Those templates expect `/etc/lunarwing/lunarwing.env` and
`/etc/lunarwing/xmpp-bridge.env` for production config and secrets.

## 9. Troubleshoot

Run:

```bash
scripts/lunarwing-xmpp-test-env.sh doctor
scripts/lunarwing-xmpp-test-env.sh logs 120
```

Common issues:

- `HTTP 000` from `doctor` means nothing is listening on the bridge port.
- `401` or `403` from `bridge-status` usually means the wrong token or env file.
- A system bridge may already be using `127.0.0.1:8787`; change
  `XMPP_BRIDGE_BIND` in the generated bridge env file.
- `configure-bridge` requires `jq`, `curl`, `XMPP_BRIDGE_TOKEN`, and
  `XMPP_PASSWORD`.
- If the generated systemd unit points at a missing binary, rerun `build` with
  the same `LUNARWING_TEST_PROFILE` used for `render-systemd`.

## 10. Clean Up

Stop harness-managed processes:

```bash
scripts/lunarwing-xmpp-test-env.sh stop-lunarwing
scripts/lunarwing-xmpp-test-env.sh stop-bridge
```

Stop user services if you installed them:

```bash
systemctl --user stop lunarwing-test.service
systemctl --user stop xmpp-bridge-test.service
```

Remove generated throwaway state only when you are sure you do not need the
logs, database, or OMEMO store:

```bash
rm -rf "${LUNARWING_TEST_ROOT:-/tmp/lunarwing-xmpp-test}"
```
