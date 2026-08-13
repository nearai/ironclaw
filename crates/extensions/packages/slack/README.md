# slack — channel + user-token tools

The Slack extension: DM the assistant via the Events API channel (workspace
bot), plus tools that act as the user via their own `xoxp-` token. Extension
id: `slack`. The family's worked example — the `reborn-extension-surfaces`
skill walks this manifest section by section.

- **Surfaces:** channel (`messages`: webhook `ChannelIngress`, message
  `ChannelReply`, message `ChannelDelivery`) + 8 tools
  (`slack.search_messages`, `slack.send_message`, …) + `[auth.slack]` (oauth2)
- **Vendor (credential authority):** `slack`
- **Runtime:** `wasm` for the tools (`wasm/slack_user_tool.wasm`, source in `wasm-src/`); the channel capabilities are a first-party crate the binary links
- **Code:** crate `ironclaw_slack_extension` (`src/`: `channel.rs` capabilities, `payload.rs` parsing, `mrkdwn.rs` rendering, delivery/attachments/preference codecs) + `manifest.toml`, `prompts/`, `wasm/`, `wasm-src/`
- **Depends on:** contracts tier only (`host_api`, `extension_contracts`, `product_contracts`, `attachments`); linked only by the binary and tests
- **Tests:** `cargo test -p ironclaw_slack_extension` — `tests/channel_conformance.rs`
  runs the exported `ironclaw_extension_contracts::test_support::conformance`
  suite against a scripted Slack Web API. Rebuilding the WASM guest:
  `./scripts/build-wasm-extensions.sh --first-party`, then
  `python3 scripts/ci/check-wasm-artifact-freshness.py --update`

The package is a translator, one complete method each way: `receive` resolves
payload-derived file URLs and shared-conversation context through restricted
egress before returning; reply/delivery render and send through that same
boundary. Signature verification, credential injection, attachment/context
validation after translation, and delivery semantics are the host's
(`crates/extensions/ironclaw_extension_host`). Working rules:
[`AGENTS.md`](./AGENTS.md). Family model: `crates/extensions/AGENTS.md`.
