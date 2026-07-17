# Agent Map — ironclaw_channel_host

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` first, then the module you need:
  - `identity.rs` — channel-agnostic external-identity lookup port (`RebornUserIdentityLookup`).
  - `delivery_protocol.rs` — the `ChannelDeliveryProtocol` seam (+ `PostedChannelMessage`, `FinalReplyDeliveryError`) the composition-side delivery observer/driver are generic over.
  - `outbound_targets.rs` — the outbound delivery-target provider port channel hosts implement.
  - `host_ingress.rs` — manifest-projected ingress descriptors + host-API contract registry (base) and installation rate limiting + sanitized webhook error mapping (`webhook-serve` feature).
  - `host_state_records.rs` — JSON record read/write over `ScopedFilesystem` + per-key async locks for channel host states.
  - `auth_continuation.rs` — the continuation-dispatch port pairing/OAuth completions resume blocked turns through.
  - `paired_status.rs` — the generic per-user pairedness slot extension lifecycles gate channel activation on.
- The generic delivery observer/driver these ports plug into stay in `ironclaw_reborn_composition::outbound::channel_delivery`.

## What This Crate Owns

- Vendor-neutral ports and helpers shared by Reborn channel hosts (composition's Slack module, `ironclaw_telegram_extension`).
- One definition of the manifest parsing context shared by bundled-extension installation and serve-time ingress projection.

## Do Not Move In Here

- Anything that keys on a concrete channel (Slack/Telegram strings, adapters, setup services).
- The delivery observer / triggered-run driver machinery (composition-owned until the #6116 fold).
- Composition wiring (`build_*` mount assembly), route mount shapes, or server lifecycle.

## Validation

- Fast local check: `cargo test -p ironclaw_channel_host --features webhook-serve`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
