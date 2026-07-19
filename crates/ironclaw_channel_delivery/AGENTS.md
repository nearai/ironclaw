# Agent Map — ironclaw_channel_delivery

## Start Here

- Read `src/lib.rs`, then the focused owner for the behavior being changed.
- `services.rs` holds the dependency bundle and bounded settings.
- `observer.rs` owns immediate-ACK live-run observation and final replies.
- `actionable.rs` classifies blocked/rejected states and builds safe prompts.
- `routing.rs` records delivered gate routes and tracks posted messages.
- `hooks.rs` owns post-submit hook fan-out.
- `triggered.rs` owns triggered-run admission, polling, routing, and outcomes.
- Re-derive this map with `find crates/ironclaw_channel_delivery/src -maxdepth 1 -name '*.rs' -print`; locate the public owners with `rg -n "pub struct FinalReplyDeliveryObserver|pub struct TriggeredRunDeliveryDriver|pub trait PostSubmitDeliveryHook" crates/ironclaw_channel_delivery/src`.

## What This Crate Owns

- Product-neutral live and triggered delivery algorithms shared by channel hosts.
- Bounded task admission, status-message cleanup, actionable gate delivery, and
  authoritative triggered-delivery outcome recording.
- The post-submit delivery hook contract and composite fan-out.

## Boundaries

- Channel-specific reference decoding, DM classification, rendering, and status
  requests enter through `ChannelDeliveryProtocol` and product-adapter ports.
- Channel-owned revision caches and decorators may construct this crate's
  observer/driver, but remain in their concrete channel crate.
- Do not depend on composition, CLI/WebUI, or concrete Slack/Telegram crates.
- Keep durable policy and state in `ironclaw_outbound`; keep prompt projection
  contracts in `ironclaw_product_workflow`.
- Preserve legacy idempotency/projection keys when moving neutral behavior.

## Validation

```bash
cargo test -p ironclaw_channel_delivery
cargo clippy -p ironclaw_channel_delivery --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture --test reborn_dependency_boundaries
```
