# `ironclaw_reborn_telegram_v2_host`

Library crate that owns the Reborn Telegram v2 webhook serve loop. Wired
into the `ironclaw-reborn` binary (`crates/ironclaw_reborn_cli/`) behind
the `telegram-v2` Cargo feature (default-on); the `run` subcommand
env-detects Telegram v2 and calls [`serve_from_env`] when configured.

**The v1 `ironclaw` agent has zero awareness this crate exists.** This
addresses the architectural concern raised on PR #3590: Reborn
product-layer code should not be wired into the production agent binary
by default. The v1 binary has no compile-time dependency on this crate,
no runtime entry point that touches it, and no shared in-process state.

## Why "stubbed reply path"

Today the serve loop terminates inbound at the durable ledger / binding
write and acks 200 OK to Telegram. There is no Telegram reply. This is
intentional even though the Reborn agent loop has now shipped (PRs
#3544 / #3550 / #3586 merged): this PR is the **inbound tracer**, scoped
to locking down the inbound contract — webhook auth, parse, idempotency,
binding persistence, ledger settlement. The reply-path migration is a
deliberate follow-up so the inbound contract can land + soak in
production before wiring the outbound path.

Loop migration follow-up (the loop crates are now available; this is
intentionally not done here):

- Drop [`inbound_turn::StubInboundTurnService`].
- Wire `DefaultInboundTurnService` (from `ironclaw_product_workflow`) +
  a real `TurnCoordinator` instead.
- The webhook router, composition, migrations, and storage layer all stay
  the same.

## File map

| File | Role |
|------|------|
| `src/lib.rs` | Public API: `serve`, `serve_from_env`, `init_tracing`, `telegram_v2_configured_in_env`. Also owns `connect_backend` (libSQL / Postgres). |
| `src/boot.rs` | Builds adapter + workflow + native runner + axum router |
| `src/composition.rs` | Builds the durable storage stack (ledger, binding, outbound, thread service) against libSQL or Postgres |
| `src/config.rs` | `HostConfig::from_env()` — reads `IRONCLAW_REBORN_LISTEN_ADDR`, `LIBSQL_PATH` / `DATABASE_URL`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_WEBHOOK_SECRET`, `REBORN_TELEGRAM_V2_INSTALLATION_ID` |
| `src/error.rs` | Local `HostError` enum (not derived from any v1 error type) |
| `src/inbound_turn.rs` | `StubInboundTurnService` — persists the binding, returns `Submitted`, **no reply produced** |
| `src/host_egress.rs` | `HostMediatedTelegramEgress` — `ProtocolHttpEgress` impl delegating to `ironclaw_host_api::RuntimeHttpEgress` (URL-path credential injection via `RuntimeCredentialTarget::UrlPath`) |
| `src/router.rs` | axum handler for `POST /webhook/telegram-v2/{installation_id}` |

The product-workflow ledger lives on the universal-FS dispatch fabric
(`FilesystemIdempotencyLedger` over `ScopedFilesystem`); there is no
separate per-table SQL schema — the FS backend's own
`run_migrations()` creates everything we need. Conversation binding
and outbound delivery state were already on the FS fabric before this
slice.

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
export LIBSQL_PATH=~/.ironclaw-reborn.db          # required (or DATABASE_URL for Postgres)
# Optional:
# export REBORN_TELEGRAM_V2_INSTALLATION_ID=default
# export IRONCLAW_REBORN_LISTEN_ADDR=127.0.0.1:8090

cargo build --bin ironclaw-reborn
./target/debug/ironclaw-reborn run
```

When `TELEGRAM_BOT_TOKEN` is set in the environment, `ironclaw-reborn
run` enters the long-lived serve loop. When it's unset, `run` falls
through to the existing runtime-shell snapshot — the same behavior as
before this crate was wired in.

The host fails closed at startup if neither `DATABASE_URL` (Postgres) nor
`LIBSQL_PATH` (libSQL) is set. Ephemeral in-memory storage is available
for tests/dev via the explicit opt-in `IRONCLAW_REBORN_ALLOW_EPHEMERAL=1`;
do not use it in production — the ledger and bindings will not survive
a restart.

Then expose `127.0.0.1:8090` via cloudflared/ngrok and register the webhook
with Telegram:

```bash
curl -X POST "https://api.telegram.org/bot$TELEGRAM_BOT_TOKEN/setWebhook" \
  -H "Content-Type: application/json" \
  -d "{\"url\":\"https://YOUR-TUNNEL/webhook/telegram-v2/default\",\"secret_token\":\"$TELEGRAM_WEBHOOK_SECRET\"}"
```

Send a message in Telegram. The host logs:

```text
Reborn host: inbound resolved + bound; reply path stubbed pending outbound migration
```

…and Telegram receives a 200 ack. No outbound `sendMessage` is dispatched.

## Disabling Telegram v2

Build the CLI with `--no-default-features` (or `--no-default-features
--features libsql` to keep storage support enabled but exclude the
Telegram crate) to omit this host entirely. `ironclaw-reborn run` will
then always print the runtime-shell snapshot. Future channels follow the
same opt-out pattern via their own feature flags.

## Refs

- Epic: **#3484** Reborn Contributor Runway
- Channel ports tracker: **#3577**
- Telegram port issue: **#3581**
- WASM runtime dependency: **#3583** (when this lands, swap
  `NativeProductAdapterRunner` for `ProductAdapterComponentRuntime` in
  `boot.rs`)
- Reborn agent loop: **#3544 / #3550 / #3586** (now merged; this crate
  still uses `StubInboundTurnService` by design — see "Why 'stubbed
  reply path'" above. Outbound migration to `DefaultInboundTurnService`
  + `TurnCoordinator` is a follow-up PR.)
- Ingress boundary design: **#3578**
