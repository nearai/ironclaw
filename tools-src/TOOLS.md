
# WASM Tools

These tools run as sandboxed WASM components. All other integrations are now handled as **skills** (prompt templates + HTTP tool with automatic credential injection).

See `skills/` directory for skill-based integrations: Gmail, Google Calendar/Docs/Drive/Sheets/Slides, Slack, GitHub, Web Search, LLM Context, Composio.

## Active WASM Tools

- [x] Telegram - user-mode via direct MTProto over HTTPS (contacts, messages, send, search, forward, delete); no Docker needed
- [x] Portfolio - cross-chain DeFi portfolio scanner, strategy engine, NEAR Intent construction

## Planned WASM Tools

- [ ] Google Cloud - work with cloud instances, storage, spin up and configure new instances, shut them down
- [ ] WhatsApp - Cloud API for messaging via Meta Business platform (if REST, consider skill instead)
- [ ] Signal - messaging (note: no official public API exists)
- [ ] Uber - call a car, check ride status (if REST, consider skill instead)
