# xmpp-bridge

`xmpp-bridge` is the local sidecar used by the installable IronClaw `xmpp` WASM channel.

It owns the real XMPP connection, OMEMO state, room membership, and outbound delivery.
The WASM channel talks to it over loopback HTTP.

## Build

```bash
cd bridges/xmpp-bridge
./build.sh
```

## Run

```bash
XMPP_BRIDGE_BIND=127.0.0.1:8787 \
XMPP_BRIDGE_TOKEN=change-me \
./target/release/xmpp-bridge
```

For an isolated LunarWing/XMPP bridge test harness, including local API smoke
tests, live configuration wrappers, and generated user-systemd units, see
[`docs/LUNARWING_XMPP_TESTING.md`](../../docs/LUNARWING_XMPP_TESTING.md).

## systemd

On Linux, `ironclaw service install` now installs a companion user unit for
`xmpp-bridge` when the bridge binary is available next to the current checkout.
`ironclaw service start` and `ironclaw service stop` manage both units together.

If you run IronClaw as a system service instead of a user service, use the
example units in:

- `deploy/ironclaw.service`
- `deploy/xmpp-bridge.service`

The main daemon unit now declares `Wants=` / `After=` on `xmpp-bridge.service`
so the bridge starts before `ironclaw run`.

## API

- `POST /v1/configure`
- `GET /v1/status`
- `POST /v1/outbound-rate-limit`
- `GET /v1/messages?cursor=<n>`
- `POST /v1/messages/send`

When `XMPP_BRIDGE_TOKEN` is set, requests must include `Authorization: Bearer <token>`.
The server also rejects non-loopback clients.

`GET /v1/status` includes the configured plain-room list in `configured_rooms`,
rooms that have produced MUC presence in `rooms_with_presence`, and the active
OMEMO `device_id` and `fingerprint` so you can verify or trust the IronClaw
device from clients like Gajim. It also reports the configured outbound XMPP
hourly cap (`configured_max_messages_per_hour`), the live active cap
(`active_max_messages_per_hour`), and how many outbound messages are still
counted in the current rolling hour (`outbound_messages_last_hour`).

`POST /v1/outbound-rate-limit` lets you change the active outbound XMPP hourly
cap without restarting the bridge or IronClaw. Example:

```bash
curl -sS -X POST "$BASE/v1/outbound-rate-limit" \
  -H "Authorization: Bearer $XMPP_BRIDGE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"max_messages_per_hour":0}' | jq
```

Set `reset_counter` to `true` if you want to clear the current rolling-hour
usage at the same time.
