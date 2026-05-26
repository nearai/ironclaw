# ironclaw_oauth guardrails

- Owns the loopback OAuth callback listener (port 9876), branded landing pages, and `OAUTH_CALLBACK_HOST` binding rules. That is the entire scope.
- Do **not** add provider-specific OAuth (Anthropic, Gemini, GitHub Copilot, OpenAI Codex, NEAR AI, MCP) — those flows live with the consumer that owns the credential and depend on this crate for the transport.
- Do **not** add token storage, refresh logic, PKCE/device-code orchestration, secrets handling, or HTTP client work — keep those concerns in the calling crate. This crate is a callback transport.
- Do **not** depend on `ironclaw_llm`, `ironclaw_secrets`, `ironclaw_authorization`, or any upper substrate. Only `ironclaw_common` for env helpers.
- Wildcard host binds (`0.0.0.0`, `::`) must remain rejected — the listener carries session tokens over plain HTTP.
