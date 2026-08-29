# xquik-mcp - Xquik hosted-MCP extension

This package connects IronClaw to Xquik's hosted MCP server. It replaces
browser-cookie extraction with OAuth 2.1 and host-managed Bearer tokens.
IronClaw discovers the server's tools after authorization.

- **Surfaces:** `[mcp]` hosted server with discovered tools and `[auth.xquik]`
- **Vendor:** `xquik`
- **Runtime:** MCP loader
- **Authentication:** OAuth 2.1 with dynamic client registration and S256 PKCE
- **Credential handling:** encrypted host storage and mediated Bearer injection
- **Tests:** package inventory, manifest parsing, catalog search, and architecture gates

The extension covers tweet search, profiles, timelines, monitoring, exports,
and approved account actions. It does not expose X browser cookies to
IronClaw.

Xquik is an independent third-party service. Not affiliated with X Corp.
"Twitter" and "X" are trademarks of X Corp.

See `crates/extensions/AGENTS.md` for package rules.
