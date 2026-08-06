# telegram — pure channel extension

The Telegram messaging channel via the Bot API — the extension runtime's
"addition test": a pure channel extension whose entire vendor surface is one
manifest plus the adapter crate. No tools, no WASM module, no bespoke host
code. Extension id: `telegram`.

- **Surfaces:** channel (`messages`, inbound + outbound) only — no `[[tools]]`, no `[auth.*]` recipe (deployment credentials — bot token, webhook secret — arrive via `[admin_configuration]`)
- **Vendor (credential authority):** none (pairing uses the `web_generated_code` connection strategy)
- **Runtime:** `first_party`
- **Code:** crate `ironclaw_telegram_extension` (`src/`: `payload.rs` Bot API normalization, `render.rs` outbound shaping, `channel.rs` adapter, attachment/preference codecs) + `manifest.toml`
- **Depends on:** contracts tier only — dependency-set parity with Slack (`host_api`, `extension_contracts`, `product_contracts`, `attachments`), pinned by `telegram_extension_gates.rs`; linked only by the binary and tests
- **Tests:** `cargo test -p ironclaw_telegram_extension` — `tests/channel_conformance.rs`
  runs the exported channel-adapter conformance suite (+ proptest fixtures)

The adapter never sees raw token bytes: the host runs the manifest-declared
`shared_secret_header` verification and injects credentials on mediated
egress. Working rules: [`AGENTS.md`](./AGENTS.md). Family model:
`crates/extensions/AGENTS.md`.
