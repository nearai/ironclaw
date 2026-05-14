# `ironclaw_reborn_telegram_v2_host`

Standalone host for the Reborn Telegram v2 channel. Ships as its own binary
(`ironclaw-reborn-telegram-host`); **the v1 `ironclaw` agent has zero
awareness this crate exists**.

This addresses the architectural concern raised on PR #3590: Reborn
product-layer code should not be wired into the production agent binary by
default. The v1 binary has no compile-time dependency on this crate, no
runtime entry point that touches it, and no shared in-process state.

## Why "stubbed reply path"

Today the binary terminates inbound at the durable ledger / binding write
and acks 200 OK to Telegram. There is no Telegram reply. This is intentional:
there is no Reborn agent loop in `src/` yet (the loop ships across PRs
#3544 / #3550 / #3586). The tracer's purpose is to lock down the inbound
contract — webhook auth, parse, idempotency, binding persistence, ledger
settlement — so that swapping in the real loop is a one-line change in
[`boot.rs`].

When the Reborn loop lands:

- Drop [`inbound_turn::StubInboundTurnService`].
- Wire `DefaultInboundTurnService` (from `ironclaw_product_workflow`) +
  a real `TurnCoordinator` instead.
- The webhook router, composition, migrations, and storage layer all stay
  the same.

## File map

| File | Role |
|------|------|
| `src/lib.rs` | Re-exports + crate-level docs |
| `src/boot.rs` | Builds adapter + workflow + native runner + axum router |
| `src/composition.rs` | Builds the durable storage stack (ledger, binding, outbound, thread service) against libSQL or Postgres |
| `src/config.rs` | `HostConfig::from_env()` — reads `IRONCLAW_REBORN_LISTEN_ADDR`, `LIBSQL_PATH` / `DATABASE_URL`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_WEBHOOK_SECRET`, `REBORN_TELEGRAM_V2_INSTALLATION_ID` |
| `src/error.rs` | Local `HostError` enum (not derived from any v1 error type) |
| `src/inbound_turn.rs` | `StubInboundTurnService` — persists the binding, returns `Submitted`, **no reply produced** |
| `src/migrations.rs` | Own migration runner for `product_inbound_actions` + `product_bindings` |
| `src/router.rs` | axum handler for `POST /webhook/telegram-v2/{installation_id}` |
| `src/bin/ironclaw-reborn-telegram-host.rs` | Binary entry point |

## Call path

```text
Telegram webhook
  POST /webhook/telegram-v2/{install_id}                    (router.rs)
  → NativeProductAdapterRunner::process_webhook
      ├─ verify SharedSecretHeaderAuth
      ├─ TelegramV2Adapter::parse_inbound                   (crate dep)
      └─ DefaultProductWorkflow::accept_inbound
          ├─ IdempotencyLedger::begin_or_replay             (libSQL / Postgres)
          ├─ StubInboundTurnService::accept_user_message
          │     └─ ConversationBindingService::resolve_binding
          │         (creates (tenant, user, thread) row on first inbound)
          └─ IdempotencyLedger::settle
  → 200 OK to Telegram
  (no reply produced — see "Stubbed reply path" above)
```

## Operator setup

```bash
export TELEGRAM_BOT_TOKEN=...
export TELEGRAM_WEBHOOK_SECRET=...
export LIBSQL_PATH=~/.ironclaw-reborn.db
# Optional:
# export REBORN_TELEGRAM_V2_INSTALLATION_ID=default
# export IRONCLAW_REBORN_LISTEN_ADDR=127.0.0.1:8090

cargo build --bin ironclaw-reborn-telegram-host
./target/debug/ironclaw-reborn-telegram-host
```

Then expose `127.0.0.1:8090` via cloudflared/ngrok and register the webhook
with Telegram:

```bash
curl -X POST "https://api.telegram.org/bot$TELEGRAM_BOT_TOKEN/setWebhook" \
  -H "Content-Type: application/json" \
  -d "{\"url\":\"https://YOUR-TUNNEL/webhook/telegram-v2/default\",\"secret_token\":\"$TELEGRAM_WEBHOOK_SECRET\"}"
```

Send a message in Telegram. The host logs:

```text
Reborn host: inbound resolved + bound; reply path stubbed (no Reborn agent loop yet)
```

…and Telegram receives a 200 ack. No outbound `sendMessage` is dispatched.

## Refs

- Epic: **#3484** Reborn Contributor Runway
- Channel ports tracker: **#3577**
- Telegram port issue: **#3581**
- WASM runtime dependency: **#3583** (when this lands, swap
  `NativeProductAdapterRunner` for `ProductAdapterComponentRuntime` in
  `boot.rs`)
- Reborn agent loop: **#3544 / #3550 / #3586** (when this lands, drop
  `StubInboundTurnService` and wire `DefaultInboundTurnService` +
  `TurnCoordinator`)
- Ingress boundary design: **#3578**
