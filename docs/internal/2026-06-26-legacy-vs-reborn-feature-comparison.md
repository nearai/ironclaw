# Legacy vs Reborn Feature Comparison

**Generated:** 2026-06-26
**Scope:** Internal planning snapshot for comparing the current legacy IronClaw runtime with the Reborn runtime surface.
**Primary inputs:** `FEATURE_PARITY.md`, `docs/reborn-binary.md`, `docs/reborn/2026-04-25-current-architecture-map.md`, and `docs/reborn/2026-04-27-product-manager-architecture-guide.md`.

This document is a summary for planning and triage. It does not replace `FEATURE_PARITY.md`, which remains the detailed OpenClaw-to-IronClaw parity tracker. Here, **Legacy** means the current non-Reborn IronClaw product/runtime surface, and **Reborn** means the standalone Reborn architecture, CLI, WebUI beta, host runtime, and loop stack.

## Status Legend

| Status | Meaning |
| --- | --- |
| Available | Broadly usable in that runtime surface. |
| Partial | Implemented slice exists, but important product or production gaps remain. |
| Missing | Not implemented or intentionally not in scope for that runtime. |
| Reborn-only | New Reborn architecture or capability without a direct legacy equivalent. |
| Legacy-only | Existing legacy capability not yet carried into Reborn. |

## Executive Summary

| Area | Legacy | Reborn | Planning read |
| --- | --- | --- | --- |
| Product readiness | Available | Partial | Legacy remains the safer default for full-product operation. Reborn is an operator/testing surface with important end-to-end slices. |
| Architecture direction | Mature monolith-style runtime with extracted crates | Reborn-only host/kernel boundary with userland loops | Reborn is the target architecture for authority, runtime isolation, and product workflow separation. |
| Web gateway | Available | Partial | Legacy has the broader gateway/control-plane surface. Reborn WebChat v2 is usable behind `webui-v2-beta`, but still beta. |
| CLI | Available | Partial | Reborn has a standalone CLI with selected commands; it does not replace `ironclaw` yet. |
| Channels | Available | Partial | Legacy has broader channel parity. Reborn product-live channel/runtime wiring is still selective. |
| Agent loop | Available | Partial | Legacy handles current product workflows. Reborn has the planned loop framework, capability calls, gates, subagent slices, and stronger run-state boundaries. |
| Tools/extensions | Available | Partial | Legacy has WASM tools/channels and MCP support. Reborn adds host-mediated capability surfaces, product-auth, MCP runtime slices, and provider-safe tool projection. |
| Security model | Available | Partial/Reborn-only | Legacy has many hardened paths. Reborn adds a clearer kernel-mediated authority model, but some production wiring remains incomplete. |
| Automation | Available | Partial | Legacy cron/routines are available. Reborn trigger persistence, first-party trigger capabilities, WebUI controls, and one-shot triggers are in progress. |
| Memory/workspace | Available | Partial | Legacy memory is broad and user-facing. Reborn memory/storage contracts and scoped filesystem substrates are architecturally stronger, but not all legacy UX is carried over. |

## Feature Comparison

| Feature area | Legacy status | Reborn status | Notes |
| --- | --- | --- | --- |
| Default runtime | Available | Missing | `ironclaw-reborn` is explicitly not the default runtime and does not replace `ironclaw` behavior yet. |
| Standalone binary | Missing | Partial | `ironclaw-reborn` supports `run`, `repl`, `doctor`, `onboard`, `models`, `skills`, `hooks`, `logs`, `extension`, `profile`, and selected `channels` commands. |
| Configuration and migration | Available | Partial | Reborn has its own home/config path. It intentionally does not yet support v1 config, DB, settings, secrets, or history migration. |
| WebChat/UI | Available | Partial | Legacy has Web gateway chat and dashboard views. Reborn WebChat v2 is supported through `serve` only with `webui-v2-beta`; it is an early beta operator surface. |
| Web gateway APIs | Available | Partial | Legacy includes health/status, chat, memory, jobs, logs, extensions, SSE/WebSocket, and OpenAI-compatible APIs. Reborn WebUI exposes browser-facing `/api/webchat/v2` flows and event projections, but production durable/live fanout remains follow-up work. |
| Channel registry and channels | Available | Partial | Legacy supports CLI/TUI, HTTP webhook, REPL, WebChat, WASM channels, Telegram, Slack, Signal, and partial Discord/Feishu/WeCom/WeChat. Reborn channel registry is still not product-complete; `channels list` currently reports a deliberate empty/configured surface in the standalone CLI docs. |
| Agent/session model | Available | Partial | Legacy supports per-sender sessions, compaction, custom prompts, skills, plugin tools, subagent framework, and approvals. Reborn has typed turn/run/thread state, loop family/executor architecture, capability calls, gates, and subagent slices, with product-complete replacement still pending. |
| Capability/tool execution | Available | Partial/Reborn-only | Legacy uses built-in/WASM/MCP tools through the existing registry. Reborn models tools as host-mediated capabilities with provider-safe tool definitions, explicit capability IDs, authorization, approvals, leases, and runtime adapters. |
| Tool-name model surface | Available | Reborn-only | Reborn separates internal dotted `CapabilityId` values from provider-safe tool names because provider APIs reject dots. The snapshot maps provider names back to canonical capability IDs. |
| File/coding tools | Available | Partial | Legacy has file/shell tools and approvals. Reborn has first-party scoped read/write/list/glob/grep/apply_patch capabilities through HostRuntime; OpenClaw-compatible `agents.files.*` aliases and full hardening are still pending. |
| MCP and hosted extensions | Available | Partial | Legacy has MCP clients and registries. Reborn composes host-mediated MCP runtime slices, hosted MCP activation, product-auth credential staging, and bundled Notion/NEAR AI slices, with live-recorded parity still follow-up. |
| Google/GSuite tools | Available/Partial | Partial | Reborn has operation-level Google Drive/Docs/Sheets/Slides WASM packages with host-mediated HTTP egress and OAuth setup metadata; full parity remains follow-up. |
| Model providers | Available | Partial | Legacy has broad provider coverage and failover. Reborn standalone CLI supports model provider listing/status and `set-provider`; the WebUI quick start supports NEAR AI, OpenAI, Anthropic, Ollama, and catalog-backed provider selection, but live fetching/OAuth/API-key login flows are not complete across all surfaces. |
| Model features | Available | Partial | Legacy has failover, cooldowns, model selector, OpenAI-compatible support, and many provider adapters. Reborn is focused on host-managed model requests and tool-capable model gateway behavior; advanced catalog/pricing/thinking/replay parity is incomplete. |
| Skills | Available | Partial | Legacy has prompt-based skills, grouped directories, CLI commands, and activation criteria. Reborn local-dev uses catalog/list-first model-selected activation before loading full skill context. |
| Memory/workspace | Available | Partial | Legacy has vector memory, hybrid search, identity files, daily logs, heartbeat docs, flexible paths, and CLI commands. Reborn strengthens scoped filesystem/storage placement and project/tenant/user scoping, but not every legacy memory UX or backend is carried over. |
| Automation/routines | Available | Partial | Legacy cron, heartbeat, hooks, and routines are available. Reborn scheduled triggers have persistence, backend parity work, atomic fire claim/update APIs, first-party `trigger_*` capabilities, WebUI controls, and first-class one-shot triggers; external delivery and production readiness remain follow-up. |
| Background/process execution | Available | Partial/Reborn-only | Legacy has orchestrator/worker and jobs. Reborn adds a capability-backed process path through `CapabilityHost::spawn_json`, `ProcessManager`, `ProcessHost`, resource ownership, events, and runtime dispatch, but product completeness is still pending. |
| Approval/auth gates | Available | Partial/Reborn-only | Legacy has TUI approvals and extension auth paths. Reborn models auth/approval as typed gate/resume paths with exact invocation identity, capability replay metadata, product auth, and WebUI SSO slices. |
| WebUI SSO | Missing | Partial/Reborn-only | Reborn `serve` includes Google/GitHub browser SSO behind `webui-v2-beta`, with verified-email-domain admission and tenant-bound HMAC sessions. |
| Security perimeter | Available | Partial/Reborn-only | Legacy has bearer auth, pairing, allowlists, SSRF guards, sandboxing, path traversal protections, prompt-injection defense, and leak detection. Reborn makes the kernel/host runtime the authority boundary for capability dispatch, secrets, network, mounts, resources, redaction, process state, and audit/events; production wiring is still incomplete in several areas. |
| Sandboxing | Available | Partial/Reborn-only | Legacy has Docker sandbox and WASM sandbox support. Reborn process sandbox MVP adds typed process plans, backend-neutral sandbox backends, hardened Docker command construction, fail-closed network-host validation, timeout/cancel cleanup, and approval/lease spawn paths, with production MITM/product wiring still partial. |
| Hooks/plugins | Available | Partial | Legacy has lifecycle hooks, declarative hooks, webhook hooks, WASM tools/channels, and extension lifecycle. Reborn supports selected hooks and host-mediated extension/runtime slices, but plugin route registration, provider plugins, registry repair, and many OpenClaw hook families remain missing. |
| OpenAI-compatible API | Available | Missing/Partial | Legacy exposes OpenAI-compatible chat completions and request-level model override. Reborn currently focuses on WebChat v2 and host-managed model routing, not full legacy gateway API replacement. |
| Media/TTS/STT | Partial | Missing/Partial | Legacy has some media handling and channel attachment paths, but broad OpenClaw media parity is still missing. Reborn PDF/read-file slices exist, but broad image/audio/video/TTS/STT parity is not product-complete. |
| Mobile/macOS apps | Mostly out of scope | Missing | Legacy tracks these as out of scope or missing relative to OpenClaw. Reborn does not change that near-term. |
| Trace Commons | Partial | Partial/Reborn-aligned | IronClaw retains local-first trace capture, queue/status/credit helpers, and client wrappers. Standalone server-side production infra moved out to `tracedao-server`; Reborn uses product facade patterns for local integration. |

## Reborn Strengths

| Strength | Why it matters |
| --- | --- |
| Kernel-mediated capability boundary | Side effects go through explicit authorization, approvals, leases, mounts, secrets, network policy, resources, redaction, and audit/event paths. |
| Product workflow separation | Product behavior sits above the kernel instead of being mixed into low-level runtime dispatch. |
| Typed run/thread/turn state | Reborn avoids stringly paths for pending auth, approvals, provider tool replay, capability activity, and result writing. |
| Provider-safe tool projection | Dotted internal capability IDs stay stable while provider-facing names satisfy model API constraints. |
| Product-auth and hosted runtime direction | OAuth/product credentials are staged by host-owned paths rather than exposed as raw tool inputs. |
| Multi-runtime capability model | WASM, Script, MCP, first-party/system, and sandboxed process work converge behind common host contracts. |
| Better future multi-user posture | Reborn docs and slices consistently model tenant/user/project/agent/resource scopes. |

## Legacy Strengths

| Strength | Why it matters |
| --- | --- |
| Broader day-to-day product coverage | Legacy remains better for existing user-facing gateway, channel, CLI, memory, model, and routine workflows. |
| More complete channel surface | Telegram, Slack, Signal, WebChat, HTTP webhook, TUI/REPL, and WASM channels are already available or partially available. |
| More mature operational commands | Existing `ironclaw` CLI covers run/onboard/config/status/memory/skills/pairing/sandbox/doctor/logs and more. |
| Existing integration surface | Current tools, extensions, hooks, secrets, DB settings, and workspace memory have live usage paths. |
| Lower migration risk today | Existing configs, settings, secrets, and data stores are wired for the current runtime. |

## Notable Gaps Before Reborn Can Replace Legacy

| Gap | Impact |
| --- | --- |
| v1 migration | Reborn does not yet migrate legacy config, DB, settings, secrets, or history. |
| Production runtime services | Reborn `serve` is beta and not yet a production gateway replacement. |
| Channel product parity | Reborn channel registry/product adapters are not complete enough to cover legacy channel behavior. |
| Gateway API parity | OpenAI-compatible API, broader control-plane endpoints, diagnostics, and durable fanout need explicit replacement decisions. |
| Extension/plugin parity | Provider plugins, auth plugins, memory/context plugins, route registration, registry management, and many hook families are missing. |
| Model/provider feature parity | Catalog, OAuth/login flows, pricing, advanced thinking controls, replay normalization, and media generation support are incomplete. |
| Memory UX parity | Reborn has stronger storage contracts, but not all legacy memory/search/identity UX is product-complete. |
| Automation production readiness | Trigger loop follow-ups remain around external result delivery, readiness policy, active-run retention, and jitter. |
| Docs and operator guidance | Reborn needs a migration guide, replacement readiness checklist, and explicit compatibility policy before defaulting users to it. |

## Suggested Tracking Categories

Use these categories when turning this comparison into work items:

| Category | Suggested owner boundary |
| --- | --- |
| Replacement blockers | Runtime composition, config migration, gateway/API parity, channel parity. |
| Product-visible deltas | WebUI behavior, CLI commands, logs/status, memory UX, automation UX. |
| Security deltas | Auth, approvals, secrets, outbound network policy, sandboxing, audit/events. |
| Extension deltas | WASM, MCP, product-auth, hosted extensions, plugin registry, hook families. |
| Model deltas | Provider catalog, credential setup, tool-call replay, thinking controls, image/audio/video. |
| Operational deltas | Doctor checks, logs, diagnostics export, service install, readiness, rollback. |

## Source Notes

- `FEATURE_PARITY.md` is OpenClaw vs IronClaw, not Legacy vs Reborn. This document reuses its legacy feature coverage and Reborn-specific notes where present.
- `docs/reborn-binary.md` is the best current operator guide for the standalone Reborn binary and WebUI beta.
- `docs/reborn/2026-04-25-current-architecture-map.md` describes implemented Reborn slices and explicit gaps; it cautions that most rows are slice-level, not product-complete claims.
- `docs/reborn/2026-04-27-product-manager-architecture-guide.md` is PM-facing architecture guidance and should be used to place new product features in the correct Reborn layer.
