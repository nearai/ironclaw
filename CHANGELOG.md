# Changelog

All notable changes to BastionClaw will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Merging from upstream BastionClaw

BastionClaw is a fork of [IronClaw](https://github.com/nearai/ironclaw). When merging upstream changes:

1. Pull upstream into a staging branch:
   ```bash
   git fetch upstream
   git checkout staging
   git merge upstream/staging
   ```
2. Resolve any conflicts — pay particular attention to `src/bootstrap.rs` (config paths),
   `Cargo.toml` (package names), and any files in `src/channels/web/` or `src/secrets/`
   where BastionClaw diverges from upstream.
3. Record merged upstream version below under a dedicated `### Upstream sync` heading
   so it is clear which BastionClaw release the merge covers.
4. Push and open a PR against `staging` for review before merging to `main`.

---

## [Unreleased]

### Added

- Initial fork of IronClaw v0.25.0 as BastionClaw
- Terminal3 branding and README
- Trinity decentralised secret network integration (in progress)
- Product-grade local MCP setup guidance for `t3n-mcp`, including Trinity login/authorisation prompts, per-user completion tracking, and Docker docs for rebuilding and re-registering the sidecar
