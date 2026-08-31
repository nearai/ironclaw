# slack — channel + user-token tools

The Slack extension: DM the assistant via the Events API channel (workspace
bot), plus tools that act as the user via their own `xoxp-` token. Extension
id: `slack`. The family's worked example — the `reborn-extension-surfaces`
skill walks this manifest section by section.

- **Surfaces:** channel (`messages`: webhook `ChannelIngress`, stream
  `ReplySink` on Slack's native Agent surface, message `ChannelDelivery`)
  + 16 tools
  (`slack.search_messages`, `slack.send_message`, …) + `[auth.slack]`
  (oauth2). Every tool is a `standard_op` binding: Slack covers all 16 core
  standard messaging operations, so the host owns their schemas and the
  manifest `description` is only the Slack addendum
- **Vendor (credential authority):** `slack`
- **Runtime:** `wasm` for the tools (`wasm/slack_user_tool.wasm`, source in `wasm-src/`); the channel capabilities are a first-party crate the binary links
- **Code:** crate `ironclaw_slack_extension` (`src/`: `channel.rs` ingress + delivery, `reply_sink/` the streamed reply, `reply_context.rs`, `api.rs` endpoint inventory, `payload.rs` parsing, `mrkdwn.rs` rendering, delivery/attachments/preference codecs) + `manifest.toml`, `prompts/`, `wasm/`, `wasm-src/`
- **Depends on:** contracts tier only (`host_api`, `extension_contracts`, `product_contracts`, `attachments`); linked only by the binary and tests
- **Tests:** `cargo test -p ironclaw_slack_extension` — `tests/channel_conformance.rs`
  runs the exported `ironclaw_extension_contracts::test_support::conformance`
  suite against a scripted Slack Web API (stream reply cadence included);
  `tests/reply_sink_agent_api.rs` drives the reply sink against an in-crate
  fake of the native Agent API with fault injection;
  `tests/agent_app_manifest_lockstep.rs` pins the documented app manifest,
  the egress allowlist, and the endpoint inventory together. Rebuilding the WASM guest:
  `./scripts/build-wasm-extensions.sh --first-party`, then
  `python3 scripts/ci/check-wasm-artifact-freshness.py --update`

The package is a translator, one complete method each way: `receive` resolves
payload-derived file URLs and shared-conversation context through restricted
egress before returning; `deliver` renders and sends notices through that same
boundary; the reply sink reconciles Slack's native Agent session and streaming
message toward each reply revision, behind a checkpoint the host persists and
never reads. Signature verification, credential injection, attachment/context
validation after translation, publication and delivery semantics are the
host's (`crates/extensions/ironclaw_extension_host`, `ironclaw_assistant`).
A workspace whose Slack app lacks the Agents feature fails the reply clearly
(`feature_disabled`); there is no conventional-message fallback. Working rules:
[`AGENTS.md`](./AGENTS.md). Family model: `crates/extensions/AGENTS.md`.
