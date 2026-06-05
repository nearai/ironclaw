# WeCom Channel Plan

## Current PR Scope

This PR keeps WeCom focused on the intelligent bot path:

- inbound messages and events arrive through the WeCom AI Bot WebSocket protocol
- outbound replies use the same bot WebSocket route
- generated media is uploaded and sent with bot upload/send commands
- pairing, `allow_from`, private chat, and group chat session isolation are handled in the channel
- self-built application callback inbound and Agent API outbound are deferred to a separate PR

The goal is to match the OpenClaw community shape at the product level, where the chat bot is the primary user-facing entry point, without mixing the self-built app transport into the first reviewable slice.

## Implemented

- Standalone `wecom` WASM channel scaffold and registry entry.
- Bundled channel wiring and setup flow for `wecom_bot_id` and `wecom_bot_secret`.
- Host WebSocket runtime protocol support for WeCom AI Bot sessions.
- WebSocket inbound handling for text, markdown-like text, image, file, video, mixed messages, quote context, and selected interactive events.
- Inbound media hydration from WeCom-provided encrypted media URLs into IronClaw attachments.
- Attachment-only merge window so a file/image followed by text becomes one agent turn.
- Bot outbound text streaming replies.
- Bot outbound media upload/send for generated image, voice, video, and file attachments, with size guards and chunked upload state.
- Pairing flow that hides pairing codes in groups and asks users to DM the bot for the code.
- Conversation scoping that separates direct chats from group chats.
- Status/error notifications back through the active WebSocket route when model/provider failures happen.

## Security Model

- Only `wecom_bot_id` and `wecom_bot_secret` are required for this PR.
- Unknown users are blocked by default through pairing unless `dm_policy = "open"` or `allow_from` permits them.
- Group chats do not reveal pairing codes. A group message from an unapproved user only gets a "please DM the bot" prompt.
- Group conversations are scoped by WeCom `chatid`; private conversations are scoped by WeCom `userid`.
- The channel does not expose a WeCom HTTP callback endpoint in this PR.
- HTTP egress is limited to WeCom OpenWS media URLs and related object storage hosts needed for bot media retrieval.

## Deferred

- Self-built application callback verification and encrypted XML callback parsing.
- Agent API proactive send and media send.
- More exhaustive real-payload E2E coverage for every WeCom event type.
- Full account-level multi-bot isolation across all channels.
- TUI/UI affordances for editing `allow_from` and advanced group authorization policy.

## Validation Targets

- `cargo fmt --all -- --check`
- `cargo check --manifest-path channels-src/wecom/Cargo.toml --target wasm32-wasip2 -q --offline`
- `cargo clippy --manifest-path channels-src/wecom/Cargo.toml --target wasm32-wasip2 -- -D warnings`
- `cargo test --manifest-path channels-src/wecom/Cargo.toml -q --offline`
- Manual local gateway test with a real WeCom intelligent bot:
  - private text reply
  - generated image reply
  - image/file inbound hydration
  - group pairing prompt without leaking the code
  - private pairing code approval path
