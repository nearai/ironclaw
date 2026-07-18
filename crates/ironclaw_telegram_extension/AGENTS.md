# Agent Map — ironclaw_telegram_extension

## Start Here

- Read `docs/reborn/contracts/telegram-v2.md` before changing semantics.
- Regenerate the source map with:

```bash
find crates/ironclaw_telegram_extension/src -maxdepth 2 -name '*.rs' -print | sort
rg -n "pub (struct|enum|trait) Telegram|pub async fn build_telegram_host" crates/ironclaw_telegram_extension/src
```

- Stable responsibility anchors:
  - `src/setup/` — operator save/clear, redacted status, compensation.
  - `src/state/` — the single concrete filesystem-backed setup/pairing/binding/DM-target state.
  - `src/pairing/` — issue/rotate/consume/refuse/unpair and continuation dispatch.
  - `src/ingress/` — manifest-projected route, setup-revision resolver, DM pre-router.
  - `src/delivery/` — Telegram protocol, target authority, revision-aware trigger hook.
  - `src/host/` — facade-shaped Telegram builder and per-revision workflow construction.
  - `src/channel_routes.rs` — thin admin/pairing HTTP adapter.

## What This Crate Owns

- Every Telegram-specific Reborn behavior for the single `telegram` extension: concrete
  services/state/client, identity, routes/facades, revision workflow assembly, outbound
  targets/protocol, and the Telegram trigger-hook decorator.
- `build_telegram_host(TelegramHostInput) -> TelegramHostParts`, which consumes explicit
  neutral ports and returns only mountable/registerable facades and hooks.
- Telegram's declarative account-setup descriptor; generic lifecycle consumes it through
  the `ExtensionId`-keyed registry.

## Boundaries

- `ironclaw_channel_host` owns neutral host contracts; `ironclaw_channel_delivery` owns
  generic live/triggered delivery algorithms. Verify the dependency rule with
  `cargo test -p ironclaw_architecture --test reborn_dependency_boundaries`.
- Composition may construct the scoped filesystem and durable neutral stores, then mount
  routes and register returned providers/hooks. It must not regain Telegram revision,
  protocol, cache, fallback, or delivery behavior; verify with
  `cargo test -p ironclaw_architecture --test telegram_extension_gates telegram_composition_is_assembly_only`.
- Do not introduce same-crate store/client/resolver traits solely for test fakes. Exercise
  concrete filesystem state, Bot API client, and setup-revision resolver through their
  genuine lower seams.
- Do not add `RebornRuntime`, global route-mount types, listener lifecycle, other channels,
  or retired companion identities (`telegram_bot`, `telegram_personal`,
  `telegram_channel`).

## Validation

```bash
cargo test -p ironclaw_telegram_extension
cargo clippy -p ironclaw_telegram_extension --all-targets --all-features -- -D warnings
cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,slack-v2-host-beta,telegram-v2-host-beta,libsql --lib telegram
cargo test -p ironclaw_architecture --test telegram_extension_gates
```
