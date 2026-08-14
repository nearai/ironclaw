# telegram — channel and linked-account extension

Telegram support under one extension identity: the workspace Bot API channel
and an optional personal account linked as a Telegram device over MTProto. The
package remains first-party and native; there is no WASM module or companion
linked-account extension id. Extension id: `telegram`.

- **Surfaces:** channel (`messages`: webhook `ChannelIngress`, message
  `ChannelReply`, message `ChannelDelivery`) plus 15 tools for the linked
  account: send/edit/delete, add/remove reaction, open/list/inspect/read/search
  conversations and messages, and inspect/resolve/list Telegram identities.
- **Setup:** deployment credentials (bot token, webhook secret, MTProto
  `api_id`, and `api_hash`) arrive via `[admin_configuration]`; each linked-
  account tool carries a device-link credential requirement.
- **Vendor (credential authority):** `telegram`; `[auth.telegram]` declares the
  generic device-link method whose Telegram-specific adapter lives in this
  package.
- **Runtime:** `first_party`
- **Code:** crate `ironclaw_telegram_extension` (`src/`: Bot API channel
  mapping plus `linked/` for MTProto login, transport, session handling,
  mappings, pooling, and tool operations) + `manifest.toml`
- **Depends on:** contracts tier only — dependency-set parity with Slack (`host_api`, `extension_contracts`, `product_contracts`, `attachments`), pinned by `telegram_extension_gates.rs`; linked only by the binary and tests
- **Tests:** `cargo test -p ironclaw_telegram_extension` — `tests/channel_conformance.rs`
  runs the exported channel-capability conformance suite (+ proptest fixtures)

Channel `receive` returns a complete message: it performs Telegram's two-hop `getFile`
handle exchange and download, including provider-size and path-traversal
validation, through restricted egress. The package never sees raw token bytes:
the host runs the manifest-declared
`shared_secret_header` verification and injects credentials on mediated
egress.

Linked-account reads are live against Telegram rather than a mirrored inbox;
writes act as the linked user. The package owns Telegram-specific login and
provider behavior, while generic setup orchestration, linked-session custody,
authorization, and tool mediation remain host-owned. Working rules:
[`AGENTS.md`](./AGENTS.md). Family model:
`crates/extensions/AGENTS.md`.

## Upgrade behavior

No database migration converts Telegram's retired bot-pairing ceremony into a
linked personal account, and no compatibility bridge keeps that ceremony's
rows alive: **the cutover is deliberately breaking for previously paired
users**. A proof-code binding written before this release stops authorizing
anything — identity lookups for a device-link channel consult only the
versioned `device-link-v1` namespace, so the retired row is inert data. A
previously paired user finds the channel back in setup, receives the
connect-required notice if they DM the bot, and links their device once; from
then on the same verified link serves the bot conversation and the personal
Telegram capabilities. That is the identical first-run ceremony a fresh
install gets — missing credentials mean setup, exactly like every other
extension. Inert pre-cutover rows are not bulk-deleted; a user's removal or
disconnect revokes the personal session and deletes both identity generations
and the DM target.
