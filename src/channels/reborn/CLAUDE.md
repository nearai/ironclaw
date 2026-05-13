# `src/channels/reborn/` — Telegram v2 tracer

Wires the Reborn `ProductAdapter` / `ProductWorkflow` stack into the running
binary so Telegram (and, by the same template, Slack/Discord/WeChat) can
operate as a real channel without the v1 WASM channel runtime.

This module is **default-off**. Activation gates on `REBORN_TELEGRAM_V2_ENABLED`
and the existing v1/v2 exclusivity guard at `src/config/channels.rs:517`
prevents v1 Telegram and v2 Telegram from running for the same installation.

## File map

| File | Role |
|------|------|
| `boot.rs` | Entry called by `src/main.rs`. Reads secrets, fetches bot identity via `getMe`, builds adapter + workflow + native runner + synthetic channel, returns route fragments + the `Channel` to register. |
| `composition.rs` | Builds the durable storage layer (`IdempotencyLedger`, `ConversationBindingService`, `OutboundStateStore`, `SessionThreadService`) against whichever DB backend is configured. Postgres takes precedence over libSQL. |
| `v2_router.rs` | Axum handler for `POST /webhook/telegram-v2/{installation_id}`. Delegates to `NativeProductAdapterRunner::process_webhook`, maps `RunnerError` → HTTP status. |
| `v2_inbound_turn.rs` | Custom `InboundTurnService` that bypasses `TurnCoordinator` (no Reborn executor exists yet) and emits `IncomingMessage` onto the v1 `ChannelManager` stream. **Bridge seam.** |
| `product_channel.rs` | Synthetic `Channel` impl. Receives v1's `OutgoingResponse`, builds a `ProductOutboundEnvelope`, spawns a render+POST task that calls `TelegramV2Adapter::render_outbound` via the egress shim. **Bridge seam.** |
| `mod.rs` | Module index + re-exports. |

## Call paths

### Inbound (Telegram → agent)

```text
Telegram webhook
  → POST /webhook/telegram-v2/{install_id}                      (v2_router.rs)
  → NativeProductAdapterRunner::process_webhook
      ├─ verify SharedSecretHeaderAuth
      ├─ TelegramV2Adapter::parse_inbound                       (crate)
      └─ DefaultProductWorkflow::accept_inbound
          ├─ IdempotencyLedger::begin_or_replay                 (libSQL/Postgres)
          ├─ V2InboundTurnService::accept_user_message
          │     ├─ ConversationBindingService::resolve_binding  (libSQL/Postgres)
          │     └─ mpsc::send(IncomingMessage)
          └─ IdempotencyLedger::settle
  → ChannelManager merged stream picks up IncomingMessage
  → v1 Agent (legacy bridge target)
```

### Outbound (agent → Telegram)

```text
v1 Agent produces OutgoingResponse
  → ChannelManager::respond("telegram_v2", msg, response)
  → ProductChannel::respond                                     (product_channel.rs)
      └─ tokio::spawn(render_and_dispatch)
          ├─ build ProductOutboundEnvelope (FinalReply)
          ├─ TelegramV2Adapter::render_outbound
          │     ├─ TelegramHttpEgress::send                     (reqwest → api.telegram.org)
          │     └─ OutboundStateStoreDeliverySink::record
          └─ ledger/binding rows already settled durable
```

## Bridge seams — what is NOT final form

Two pieces are explicitly interim for the tracer; both are anticipated by
issue **#3577** and the porting guide at
`docs/reborn/how-to-port-channel-to-reborn.md`:

1. **`TelegramV2Adapter` is loaded as a native Rust struct** via
   `NativeProductAdapterRunner` rather than as a wasmtime component.
   Once PR **#3583** (`ProductAdapterComponentRuntime`) lands, swap the
   construction line in `boot.rs`:

   ```rust
   // before
   let adapter = Arc::new(TelegramV2Adapter::new(config));
   // after
   let adapter: Arc<dyn ProductAdapter> =
       Arc::new(ProductAdapterComponentRuntime::load(wasm_bytes, config)?);
   ```

   Everything downstream is `Arc<dyn ProductAdapter>`-typed and unaffected.

2. **`V2InboundTurnService` + `ProductChannel` bridge v2 inbound through
   the v1 `ChannelManager`.** This violates #3577's shared AC forbidding
   "v1 `Channel` dependencies in adapter core" — but the adapter *core*
   stays clean; the v1 dependency is contained to these two files. The
   reason: there is no Reborn agent loop in `src/` yet (workstream PRs
   #3544 / #3550 / #3586 still open). When the Reborn loop lands:

   - Drop `V2InboundTurnService` — use `DefaultInboundTurnService` from
     `ironclaw_product_workflow` directly.
   - Drop `ProductChannel` — outbound dispatch flows from the Reborn
     executor straight to `TelegramV2Adapter::render_outbound`, no v1
     `Channel::respond` involved.
   - `composition.rs` survives unchanged.
   - Storage layer (ledger, binding, outbound, thread service) survives
     unchanged.

## Where things live (cross-crate)

| Concern | Location |
|---|---|
| `ProductAdapter` trait + DTOs | `crates/ironclaw_product_adapters/` |
| `TelegramV2Adapter` (parse/render) | `crates/ironclaw_telegram_v2_adapter/` |
| `NativeProductAdapterRunner` | `crates/ironclaw_wasm_product_adapters/src/runner.rs` |
| `DefaultProductWorkflow`, `IdempotencyLedger`, `ConversationBindingService` traits | `crates/ironclaw_product_workflow/` |
| Durable ledger + binding + egress shim + delivery sink impls | `crates/ironclaw_product_workflow_storage/` |
| `OutboundStateStore` libSQL + Postgres | `crates/ironclaw_outbound/` |
| `SessionThreadService` libSQL + Postgres | `crates/ironclaw_threads/` |
| Schema migrations | `src/db/libsql_migrations.rs` (V26) + `migrations/V28_*.sql` |

## Operator setup

1. Store the two secrets via the web UI (or `ironclaw secret set` when
   the libSQL CLI bug is fixed):
   - `telegram_bot_token` — the BotFather token.
   - `telegram_webhook_secret` — any non-empty string; will be sent in
     the `X-Telegram-Bot-Api-Secret-Token` header on every webhook.
2. Set `REBORN_TELEGRAM_V2_ENABLED=true` in env (or via TOML).
3. Optionally set `REBORN_TELEGRAM_V2_INSTALLATION_ID` (default `"default"`).
4. Make sure v1 Telegram is **not** in `WASM_CHANNELS` and not persisted
   active — the exclusivity guard will reject startup otherwise.
5. Start `ironclaw`. Boot logs should include `Reborn Telegram v2 wired`.
6. Expose `127.0.0.1:8080` via cloudflared / ngrok / similar.
7. Register the webhook with Telegram:

   ```bash
   curl -X POST "https://api.telegram.org/bot$BOT_TOKEN/setWebhook" \
     -H "Content-Type: application/json" \
     -d "{\"url\":\"https://YOUR-TUNNEL/webhook/telegram-v2/default\",\"secret_token\":\"$WEBHOOK_SECRET\"}"
   ```

8. Send a message to the bot. The reply should arrive within a few
   seconds.

## Known gaps (P1/P2 follow-ups)

- Slash commands (`/start`, `/help`) are rejected by the workflow's
  `CommandRoutingUnavailable` path. Plain-text messages work.
- Multi-installation support is single-bot only (one entry in the
  runners map). Real multi-tenant onboarding is deferred.
- Built-in `setWebhook` helper (no manual curl) is deferred.
- Projection-based features (typing indicators, group fanout) wait on
  `EventStreamManager` from Epic Child 11 (#3281).
- Hooks framework (PR #3573) operates one layer down (Reborn loop
  ports); the tracer doesn't currently fire hooks.

## Refs

- Epic: **#3484** Reborn Contributor Runway
- Channel ports tracker: **#3577**
- Telegram port issue: **#3581**
- WASM runtime dependency: **#3583**
- Porting guide: `docs/reborn/how-to-port-channel-to-reborn.md`
- Ingress boundary design: **#3578**
