# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.1](https://github.com/nearai/ironclaw/compare/v0.11.0...v0.11.1) - 2026-02-23

### Other

- Ignore out-of-date generated CI so custom release.yml jobs are allowed

## [0.11.0](https://github.com/nearai/ironclaw/compare/v0.10.0...v0.11.0) - 2026-02-23

### Fixed

- auto-compact and retry on ContextLengthExceeded ([#315](https://github.com/nearai/ironclaw/pull/315))

### Other

- *(README)* Adding badges to readme ([#316](https://github.com/nearai/ironclaw/pull/316))
- Feat/completion ([#240](https://github.com/nearai/ironclaw/pull/240))

## [0.10.0](https://github.com/nearai/ironclaw/compare/v0.9.0...v0.10.0) - 2026-02-22

### Added

- update dashboard favicon ([#309](https://github.com/nearai/ironclaw/pull/309))
- add web UI test skill for Chrome extension ([#302](https://github.com/nearai/ironclaw/pull/302))
- implement FullJob routine mode with scheduler dispatch ([#288](https://github.com/nearai/ironclaw/pull/288))
- hot-activate WASM channels, channel-first prompts, unified artifact resolution ([#297](https://github.com/nearai/ironclaw/pull/297))
- add pairing/permission system to all WASM channels and fix extension registry ([#286](https://github.com/nearai/ironclaw/pull/286))
- group chat privacy, channel-aware prompts, and safety hardening ([#285](https://github.com/nearai/ironclaw/pull/285))
- embedded registry catalog and WASM bundle install pipeline ([#283](https://github.com/nearai/ironclaw/pull/283))
- show token usage and cost tracker in gateway status popover ([#284](https://github.com/nearai/ironclaw/pull/284))
- support custom HTTP headers for OpenAI-compatible provider ([#269](https://github.com/nearai/ironclaw/pull/269))
- add smart routing provider for cost-optimized model selection ([#281](https://github.com/nearai/ironclaw/pull/281))

### Fixed

- persist user message at turn start before agentic loop ([#305](https://github.com/nearai/ironclaw/pull/305))
- block send until thread is selected ([#306](https://github.com/nearai/ironclaw/pull/306))
- reload chat history on SSE reconnect ([#307](https://github.com/nearai/ironclaw/pull/307))
- map Esc to interrupt and Ctrl+C to graceful quit ([#267](https://github.com/nearai/ironclaw/pull/267))

### Other

- Fix tool schema OpenAI compatibility ([#301](https://github.com/nearai/ironclaw/pull/301))
- simplify config resolution and consolidate main.rs init ([#287](https://github.com/nearai/ironclaw/pull/287))
- Update image source in README.md
- Add files via upload
- remove ExtensionSource::Bundled, use download-only install for WASM channels ([#293](https://github.com/nearai/ironclaw/pull/293))
- allow OAuth callback to work on remote servers (fixes #186) ([#212](https://github.com/nearai/ironclaw/pull/212))
- add rate limiting for built-in tools (closes #171) ([#276](https://github.com/nearai/ironclaw/pull/276))
- add LLM providers guide (OpenRouter, Together AI, Fireworks, Ollama, vLLM) ([#193](https://github.com/nearai/ironclaw/pull/193))
- Feat/html to markdown #106  ([#115](https://github.com/nearai/ironclaw/pull/115))
- adopt agent-market design language for web UI ([#282](https://github.com/nearai/ironclaw/pull/282))
- speed up startup from ~15s to ~2s ([#280](https://github.com/nearai/ironclaw/pull/280))
- consolidate tool approval into single param-aware method ([#274](https://github.com/nearai/ironclaw/pull/274))

## [0.9.0](https://github.com/nearai/ironclaw/compare/v0.8.0...v0.9.0) - 2026-02-21

### Added

- add TEE attestation shield to web gateway UI ([#275](https://github.com/nearai/ironclaw/pull/275))
- configurable tool iterations, auto-approve, and policy fix ([#251](https://github.com/nearai/ironclaw/pull/251))

### Fixed

- add X-Accel-Buffering header to SSE endpoints ([#277](https://github.com/nearai/ironclaw/pull/277))

## [0.8.0](https://github.com/nearai/ironclaw/compare/ironclaw-v0.7.0...ironclaw-v0.8.0) - 2026-02-20

### Added

- extension registry with metadata catalog and onboarding integration ([#238](https://github.com/nearai/ironclaw/pull/238))
- *(models)* add GPT-5.3 Codex, full GPT-5.x family, Claude 4.x series, o4-mini ([#197](https://github.com/nearai/ironclaw/pull/197))
- wire memory hygiene into the heartbeat loop ([#195](https://github.com/nearai/ironclaw/pull/195))

### Fixed

- persist WASM channel workspace writes across callbacks ([#264](https://github.com/nearai/ironclaw/pull/264))
- consolidate per-module ENV_MUTEX into crate-wide test lock ([#246](https://github.com/nearai/ironclaw/pull/246))
- remove auto-proceed fake user message injection from agent loop ([#255](https://github.com/nearai/ironclaw/pull/255))
- onboarding errors reset flow and remote server auth (#185, #186) ([#248](https://github.com/nearai/ironclaw/pull/248))
- parallelize tool call execution via JoinSet ([#219](https://github.com/nearai/ironclaw/pull/219)) ([#252](https://github.com/nearai/ironclaw/pull/252))
- prevent pipe deadlock in shell command execution ([#140](https://github.com/nearai/ironclaw/pull/140))
- persist turns after approval and add agent-level tests ([#250](https://github.com/nearai/ironclaw/pull/250))

### Other

- add automated PR labeling system ([#253](https://github.com/nearai/ironclaw/pull/253))
- update CLAUDE.md for recently merged features ([#183](https://github.com/nearai/ironclaw/pull/183))

## [0.7.0](https://github.com/nearai/ironclaw/compare/ironclaw-v0.6.0...ironclaw-v0.7.0) - 2026-02-19

### Added

- extend lifecycle hooks with declarative bundles ([#176](https://github.com/nearai/ironclaw/pull/176))
- support per-request model override in /v1/chat/completions ([#103](https://github.com/nearai/ironclaw/pull/103))

### Fixed

- harden openai-compatible provider, approval replay, and embeddings defaults ([#237](https://github.com/nearai/ironclaw/pull/237))
- Network Security Findings ([#201](https://github.com/nearai/ironclaw/pull/201))

### Added

- Refactored OpenAI-compatible chat completion routing to use the rig adapter and `RetryProvider` composition for custom base URL usage.
- Added Ollama embeddings provider support (`EMBEDDING_PROVIDER=ollama`, `OLLAMA_BASE_URL`) in workspace embeddings.
- Added migration `V9__flexible_embedding_dimension.sql` for flexible embedding vector dimensions.

### Changed

- Changed default sandbox image to `ironclaw-worker:latest` in config/settings/sandbox defaults.
- Improved tool-message sanitization and provider compatibility handling across NEAR AI, rig adapter, and shared LLM provider code.

### Fixed

- Fixed approval-input aliases (`a`, `/approve`, `/always`, `/deny`, etc.) in submission parsing.
- Fixed multi-tool approval resume flow by preserving and replaying deferred tool calls so all prior `tool_use` IDs receive matching `tool_result` messages.
- Fixed REPL quit/exit handling to route shutdown through the agent loop for graceful termination.

## [0.6.0](https://github.com/nearai/ironclaw/compare/ironclaw-v0.5.0...ironclaw-v0.6.0) - 2026-02-19

### Added

- add issue triage skill ([#200](https://github.com/nearai/ironclaw/pull/200))
- add PR triage dashboard skill ([#196](https://github.com/nearai/ironclaw/pull/196))
- add OpenRouter usage examples ([#189](https://github.com/nearai/ironclaw/pull/189))
- add Tinfoil private inference provider ([#62](https://github.com/nearai/ironclaw/pull/62))
- shell env scrubbing and command injection detection ([#164](https://github.com/nearai/ironclaw/pull/164))
- Add PR review tools, job monitor, and channel injection for E2E sandbox workflows ([#57](https://github.com/nearai/ironclaw/pull/57))
- Secure prompt-based skills system (Phases 1-4) ([#51](https://github.com/nearai/ironclaw/pull/51))
- Add benchmarking harness with spot suite ([#10](https://github.com/nearai/ironclaw/pull/10))
- 10 infrastructure improvements from zeroclaw ([#126](https://github.com/nearai/ironclaw/pull/126))

### Fixed

- *(rig)* prevent OpenAI Responses API panic on tool call IDs ([#182](https://github.com/nearai/ironclaw/pull/182))
- *(docs)* correct settings storage path in README ([#194](https://github.com/nearai/ironclaw/pull/194))
- OpenAI tool calling — schema normalization, missing types, and Responses API panic ([#132](https://github.com/nearai/ironclaw/pull/132))
- *(security)* prevent path traversal bypass in WASM HTTP allowlist ([#137](https://github.com/nearai/ironclaw/pull/137))
- persist OpenAI-compatible provider and respect embeddings disable ([#177](https://github.com/nearai/ironclaw/pull/177))
- remove .expect() calls in FailoverProvider::try_providers ([#156](https://github.com/nearai/ironclaw/pull/156))
- sentinel value collision in FailoverProvider cooldown ([#125](https://github.com/nearai/ironclaw/pull/125)) ([#154](https://github.com/nearai/ironclaw/pull/154))
- skills module audit cleanup ([#173](https://github.com/nearai/ironclaw/pull/173))

### Other

- Fix division by zero panic in ValueEstimator::is_profitable ([#139](https://github.com/nearai/ironclaw/pull/139))
- audit feature parity matrix against codebase and recent commits ([#202](https://github.com/nearai/ironclaw/pull/202))
- architecture improvements for contributor velocity ([#198](https://github.com/nearai/ironclaw/pull/198))
- fix rustfmt formatting from PR #137
- add .env.example examples for Ollama and OpenAI-compatible ([#110](https://github.com/nearai/ironclaw/pull/110))

## [0.5.0](https://github.com/nearai/ironclaw/compare/v0.4.0...v0.5.0) - 2026-02-17

### Added

- add cooldown management to FailoverProvider ([#114](https://github.com/nearai/ironclaw/pull/114))

## [0.4.0](https://github.com/nearai/ironclaw/compare/v0.3.0...v0.4.0) - 2026-02-17

### Added

- move per-invocation approval check into Tool trait ([#119](https://github.com/nearai/ironclaw/pull/119))
- add polished boot screen on CLI startup ([#118](https://github.com/nearai/ironclaw/pull/118))
- Add lifecycle hooks system with 6 interception points ([#18](https://github.com/nearai/ironclaw/pull/18))

### Other

- remove accidentally committed .sidecar and .todos directories ([#123](https://github.com/nearai/ironclaw/pull/123))

## [0.3.0](https://github.com/nearai/ironclaw/compare/v0.2.0...v0.3.0) - 2026-02-17

### Added

- direct api key and cheap model ([#116](https://github.com/nearai/ironclaw/pull/116))

## [0.2.0](https://github.com/nearai/ironclaw/compare/v0.1.3...v0.2.0) - 2026-02-16

### Added

- mark Ollama + OpenAI-compatible as implemented ([#102](https://github.com/nearai/ironclaw/pull/102))
- multi-provider inference + libSQL onboarding selection ([#92](https://github.com/nearai/ironclaw/pull/92))
- add multi-provider LLM failover with retry backoff ([#28](https://github.com/nearai/ironclaw/pull/28))
- add libSQL/Turso embedded database backend ([#47](https://github.com/nearai/ironclaw/pull/47))
- Move debug log truncation from agent loop to REPL channel ([#65](https://github.com/nearai/ironclaw/pull/65))

### Fixed

- shell destructive-command check bypassed by Value::Object arguments ([#72](https://github.com/nearai/ironclaw/pull/72))
- propagate real tool_call_id instead of hardcoded placeholder ([#73](https://github.com/nearai/ironclaw/pull/73))
- Fix wasm tool schemas and runtime ([#42](https://github.com/nearai/ironclaw/pull/42))
- flatten tool messages for NEAR AI cloud-api compatibility ([#41](https://github.com/nearai/ironclaw/pull/41))
- security hardening across all layers ([#35](https://github.com/nearai/ironclaw/pull/35))

### Other

- Explicitly enable cargo-dist caching for binary artifacts building
- Skip building binary artifacts on every PR
- add module specification rules to CLAUDE.md
- add setup/onboarding specification (src/setup/README.md)
- deduplicate tool code and remove dead stubs ([#98](https://github.com/nearai/ironclaw/pull/98))
- Reformat architecture diagram in README ([#64](https://github.com/nearai/ironclaw/pull/64))
- Add review discipline guidelines to CLAUDE.md ([#68](https://github.com/nearai/ironclaw/pull/68))
- Bump MSRV to 1.92, add GCP deployment files ([#40](https://github.com/nearai/ironclaw/pull/40))
- Add OpenAI-compatible HTTP API (/v1/chat/completions, /v1/models)   ([#31](https://github.com/nearai/ironclaw/pull/31))


## [0.1.3](https://github.com/nearai/ironclaw/compare/v0.1.2...v0.1.3) - 2026-02-12

### Other

- Enabled builds caching during CI/CD
- Disabled npm publishing as the name is already taken

## [0.1.2](https://github.com/nearai/ironclaw/compare/v0.1.1...v0.1.2) - 2026-02-12

### Other

- Added Installation instructions for the pre-built binaries
- Disabled Windows ARM64 builds as auto-updater [provided by cargo-dist] does not support this platform yet and it is not a common platform for us to support

## [0.1.1](https://github.com/nearai/ironclaw/compare/v0.1.0...v0.1.1) - 2026-02-12

### Other

- Renamed the secrets in release-plz.yml to match the configuration
- Make sure that the binaries release CD it kicking in after release-plz

## [0.1.0](https://github.com/nearai/ironclaw/releases/tag/v0.1.0) - 2026-02-12

### Added

- Add multi-provider LLM support via rig-core adapter ([#36](https://github.com/nearai/ironclaw/pull/36))
- Sandbox jobs ([#4](https://github.com/nearai/ironclaw/pull/4))
- Add Google Suite & Telegram WASM tools ([#9](https://github.com/nearai/ironclaw/pull/9))
- Improve CLI ([#5](https://github.com/nearai/ironclaw/pull/5))

### Fixed

- resolve runtime panic in Linux keychain integration ([#32](https://github.com/nearai/ironclaw/pull/32))

### Other

- Skip release-plz on forks
- Upgraded release-plz CD pipeline
- Added CI/CD and release pipelines ([#45](https://github.com/nearai/ironclaw/pull/45))
- DM pairing + Telegram channel improvements ([#17](https://github.com/nearai/ironclaw/pull/17))
- Fixes build, adds missing sse event and correct command ([#11](https://github.com/nearai/ironclaw/pull/11))
- Codex/feature parity pr hook ([#6](https://github.com/nearai/ironclaw/pull/6))
- Add WebSocket gateway and control plane ([#8](https://github.com/nearai/ironclaw/pull/8))
- select bundled Telegram channel and auto-install ([#3](https://github.com/nearai/ironclaw/pull/3))
- Adding skills for reusable work
- Fix MCP tool calls, approval loop, shutdown, and improve web UI
- Add auth mode, fix MCP token handling, and parallelize startup loading
- Merge remote-tracking branch 'origin/main' into ui
- Adding web UI
- Rename `setup` CLI command to `onboard` for compatibility
- Add in-chat extension discovery, auth, and activation system
- Add Telegram typing indicator via WIT on-status callback
- Add proactivity features: memory CLI, session pruning, self-repair notifications, slash commands, status diagnostics, context warnings
- Add hosted MCP server support with OAuth 2.1 and token refresh
- Add interactive setup wizard and persistent settings
- Rebrand to IronClaw with security-first mission
- Fix build_software tool stuck in planning mode loop
- Enable sandbox by default
- Fix Telegram Markdown formatting and clarify tool/memory distinctions
- Simplify Telegram channel config with host-injected tunnel/webhook settings
- Apply Telegram channel learnings to WhatsApp implementation
- Merge remote-tracking branch 'origin/main'
- Docker file for sandbox
- Replace hardcoded intent patterns with job tools
- Fix router test to match intentional job creation patterns
- Add Docker execution sandbox for secure shell command isolation
- Move setup wizard credentials to database storage
- Add interactive setup wizard for first-run configuration
- Add Telegram Bot API channel as WASM module
- Add OpenClaw feature parity tracking matrix
- Add Chat Completions API support and expand REPL debugging
- Implementing channels to be handled in wasm
- Support non interactive mode and model selection
- Implement tool approval, fix tool definition refresh, and wire embeddings
- Tool use
- Wiring more
- Add heartbeat integration, planning phase, and auto-repair
- Login flow
- Extend support for session management
- Adding builder capability
- Load tools at launch
- Fix multiline message rendering in TUI
- Parse NEAR AI alternative response format with output field
- Handle NEAR AI plain text responses
- Disable mouse capture to allow text selection in TUI
- Add verbose logging to debug empty NEAR AI responses
- Improve NEAR AI response parsing for varying response formats
- Show status/thinking messages in chat window, debug empty responses
- Add timeout and logging to NEAR AI provider
- Add status updates to show agent thinking/processing state
- Add CLI subcommands for WASM tool management
- Fix TUI shutdown: send /shutdown message and handle in agent loop
- Remove SimpleCliChannel, add Ctrl+D twice quit, redirect logs to TUI
- Fix TuiChannel integration and enable in main.rs
- Integrate Codex patterns: task scheduler, TUI, sessions, compaction
- Adding LICENSE
- Add README with IronClaw branding
- Add WASM sandbox secure API extension
- Wire database Store into agent loop
- Implementing WASM runtime
- Add workspace integration tests
- Compact memory_tree output format
- Replace memory_list with memory_tree tool
- Simplify workspace to path-based storage, remove legacy code
- Add NEAR AI chat-api as default LLM provider
- Add CLAUDE.md project documentation
- Add workspace and memory system (OpenClaw-inspired)
- Initial implementation of the agent framework
