# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/nearai/ironclaw/compare/ironclaw_common-v0.4.2...ironclaw_common-v0.5.0) - 2026-07-24

### Added

- *(reborn)* WebChat v2 model selection + per-run usage/cost (incl. default-model pricing) ([#6111](https://github.com/nearai/ironclaw/pull/6111))
- *(reborn)* downloadable project files in WebChat v2 ([#4933](https://github.com/nearai/ironclaw/pull/4933))
- *(reborn)* attachment web UX on the WebChat v2 SPA ([#4644](https://github.com/nearai/ironclaw/pull/4644)) ([#4738](https://github.com/nearai/ironclaw/pull/4738))
- *(attachments)* bridge inbound bytes into transcript AttachmentRefs ([#4644](https://github.com/nearai/ironclaw/pull/4644)) ([#4670](https://github.com/nearai/ironclaw/pull/4670))
- *(threads)* carry attachment refs through the Reborn transcript contract ([#4644](https://github.com/nearai/ironclaw/pull/4644)) ([#4655](https://github.com/nearai/ironclaw/pull/4655))
- *(common)* extensible attachment format registry ([#4644](https://github.com/nearai/ironclaw/pull/4644)) ([#4654](https://github.com/nearai/ironclaw/pull/4654))

### Fixed

- *(reborn)* allow renaming automations ([#5765](https://github.com/nearai/ironclaw/pull/5765))
- *(reborn)* admit final replies deterministically ([#4207](https://github.com/nearai/ironclaw/pull/4207))

### Other

- Mask ambient NEARAI env in tests ([#6540](https://github.com/nearai/ironclaw/pull/6540))
- *(errors)* surface best-effort failures instead of silent let _ drops (90 sites) ([#5662](https://github.com/nearai/ironclaw/pull/5662))
- Add Reborn crate layer allowlist gate ([#5852](https://github.com/nearai/ironclaw/pull/5852))
- upgrade Rust version to 1.96 ([#5405](https://github.com/nearai/ironclaw/pull/5405))
- *(deps)* bump the everything-else group across 1 directory with 47 updates ([#5271](https://github.com/nearai/ironclaw/pull/5271))
- *(agents)* reconcile crate AGENTS.md maps with current Reborn code ([#4302](https://github.com/nearai/ironclaw/pull/4302))
- *(reborn)* consolidate PKCE math into ironclaw_common::pkce (Track A, #4215) ([#4216](https://github.com/nearai/ironclaw/pull/4216))
- [codex] Use shared LLM config resolution for Reborn ([#4079](https://github.com/nearai/ironclaw/pull/4079))
- Merge main into reborn integration ([#4078](https://github.com/nearai/ironclaw/pull/4078))
- Reborn cost-based budgets: foundation (resources, accountant, gate, period) ([#3841](https://github.com/nearai/ironclaw/pull/3841))
- *(ironclaw_common)* seal internal modules
- *(crates)* enable unreachable_pub and prune dead re-exports
- *(reborn)* establish trust-boundary hardening baseline ([#3494](https://github.com/nearai/ironclaw/pull/3494))
- Merge origin/main into reborn-integration

## [0.4.2](https://github.com/nearai/ironclaw/compare/ironclaw_common-v0.4.1...ironclaw_common-v0.4.2) - 2026-05-11

### Added

- *(common)* describe paths and platform helpers in crate description ([#3498](https://github.com/nearai/ironclaw/pull/3498))

### Other

- *(llm)* extract multi-provider integration into ironclaw_llm crate ([#3387](https://github.com/nearai/ironclaw/pull/3387))

## [0.4.1](https://github.com/nearai/ironclaw/compare/ironclaw_common-v0.4.0...ironclaw_common-v0.4.1) - 2026-05-07

### Added

- *(common)* align crate description with lib.rs doc wording ([#3372](https://github.com/nearai/ironclaw/pull/3372))

### Fixed

- *(common)* clarify crate-level doc wording ([#3370](https://github.com/nearai/ironclaw/pull/3370))

## [0.4.0](https://github.com/nearai/ironclaw/compare/ironclaw_common-v0.3.0...ironclaw_common-v0.4.0) - 2026-04-29

### Added

- *(debug-panel)* expand Activity tab coverage with CodeAct + warnings ([#2850](https://github.com/nearai/ironclaw/pull/2850))
- *(bridge)* project 7 more engine events to AppEvents ([#2844](https://github.com/nearai/ironclaw/pull/2844))
- *(bridge)* project 3 dropped engine events to AppEvents ([#2797](https://github.com/nearai/ironclaw/pull/2797))

### Fixed

- bug bash 4/16 triage — error boundary, TEE secrets, pairing, rehydration ([#2753](https://github.com/nearai/ironclaw/pull/2753))

### Other

- Merge pull request #3002 from nearai/main

## [0.3.0](https://github.com/nearai/ironclaw/compare/ironclaw_common-v0.2.0...ironclaw_common-v0.3.0) - 2026-04-21

### Added

- add debug inspector panel for web gateway ([#1873](https://github.com/nearai/ironclaw/pull/1873))
- *(skills)* activation feedback pipeline + install idempotence ([#2530](https://github.com/nearai/ironclaw/pull/2530))
- *(common)* apply ExtensionName newtype to fan-out sites (PR 2/2) ([#2617](https://github.com/nearai/ironclaw/pull/2617))
- *(common)* CredentialName + ExtensionName newtypes (PR 1/2) ([#2611](https://github.com/nearai/ironclaw/pull/2611))

### Fixed

- *(gateway)* align historical/live tool call cards and preserve tool call correlation ([#2182](https://github.com/nearai/ironclaw/pull/2182))
- image generation with nearai models ([#1819](https://github.com/nearai/ironclaw/pull/1819))

### Other

- *(types)* adopt MissionId in router + introduce McpServerName ([#2681](https://github.com/nearai/ironclaw/pull/2681))
- *(channels)* introduce ExternalThreadId newtype at channel boundary ([#2685](https://github.com/nearai/ironclaw/pull/2685))
- *(events)* replace JobResult.status String with JobResultStatus enum ([#2678](https://github.com/nearai/ironclaw/pull/2678))
- Fix gateway tool output visibility and timing ([#2555](https://github.com/nearai/ironclaw/pull/2555))
- *(events)* add OnboardingStateDto::pairing_required constructor ([#2607](https://github.com/nearai/ironclaw/pull/2607))
- Unify gateway onboarding, auth gates, and pairing flows ([#2515](https://github.com/nearai/ironclaw/pull/2515))

## [0.2.0](https://github.com/nearai/ironclaw/compare/ironclaw_common-v0.1.0...ironclaw_common-v0.2.0) - 2026-04-11

### Added

- *(tui)* port full-featured Ratatui terminal UI onto staging ([#1973](https://github.com/nearai/ironclaw/pull/1973))
- *(engine)* Unified Thread-Capability-CodeAct execution engine (v2 architecture) ([#1557](https://github.com/nearai/ironclaw/pull/1557))
- *(jobs)* per-job MCP server filtering and max_iterations cap ([#1243](https://github.com/nearai/ironclaw/pull/1243))

### Fixed

- *(engine)* mission cron scheduling + timezone propagation ([#1944](https://github.com/nearai/ironclaw/pull/1944)) ([#1957](https://github.com/nearai/ironclaw/pull/1957))

### Other

- Improve channel onboarding and Telegram pairing flow ([#2103](https://github.com/nearai/ironclaw/pull/2103))
