# Agent Map — ironclaw_telegram_extension

## Start Here

- No crate-local CLAUDE.md exists yet; the behavior contract is
  `docs/reborn/contracts/telegram-v2.md` — read it before changing semantics.
- Read `src/lib.rs` first, then the module you need:
  - `telegram_setup.rs` — operator save pipeline (`getMe` → mint webhook secret → `setWebhook` → persist → activate, with rollback/compensation), redacted status, clear.
  - `telegram_bot_api.rs` — the host-egress Bot API client (envelope handling, sanitized error categories).
  - `telegram_host_state.rs` — durable setup/pairing/binding/DM-target records on the tenant-shared filesystem plane (CAS-guarded).
  - `telegram_pairing.rs` — WebGeneratedCode pairing: issue/rotate/consume/refuse/unpair, continuation dispatch, the lifecycle paired-status impl.
  - `telegram_dispatch.rs` — the pairing-aware DM-only pre-router wrapping the adapter runner.
  - `telegram_serve.rs` — manifest-projected webhook route fragment, dynamic per-setup-revision installation resolver, ingress error mapping.
  - `telegram_actor_identity.rs` — provider `telegram`, key `tg-bot-<bot_id>:<tg_user>`, binding-epoch re-checks.
  - `telegram_adapter.rs` — per-revision `TelegramV2Adapter` assembly + declared egress targets.
  - `telegram_egress.rs` — policy-scoped egress with `{telegram_bot_token}` path-placeholder credential substitution.
  - `telegram_channel_routes.rs` — admin setup + pairing HTTP route fragment (composition wraps it into its protected mount).
  - `telegram_connectable_channel.rs` — Settings connectable-channels + per-caller connection facades.
  - `telegram_outbound_targets.rs` — paired-DM delivery targets + `TelegramDeliveryProtocol`.
  - `telegram_manifest.rs` — the bundled manifest constant (asset lives in `ironclaw_first_party_extensions/assets/telegram/`).
- Composition's wiring layer is
  `ironclaw_reborn_composition/src/telegram/telegram_host_beta.rs`; shared
  vendor-neutral machinery is `ironclaw_channel_host`.

## What This Crate Owns

- Everything Telegram-specific on the Reborn stack except `RebornRuntime`
  wiring: the identifiers, services, routes, facades, and protocol details of
  the single `telegram` extension.

## Do Not Move In Here

- `RebornRuntime`/mount assembly, delivery observer/driver construction, or
  trigger-hook registration — that is composition's `telegram_host_beta`.
- Vendor-neutral machinery (belongs in `ironclaw_channel_host`).
- Other channels' code, retired-taxonomy identities (`telegram_bot` /
  `telegram_personal` / `telegram_channel` companions are pinned to zero by
  `ironclaw_architecture/tests/telegram_extension_gates.rs`).

## Validation

- Fast local check: `cargo test -p ironclaw_telegram_extension`
- Composition wiring: `cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,slack-v2-host-beta,telegram-v2-host-beta,libsql --lib telegram`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
