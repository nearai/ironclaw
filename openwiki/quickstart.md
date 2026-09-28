---
type: "Reference"
title: "IronClaw OpenWiki: Quick Start"
description: "Entry point for understanding the codebase structure, navigation hub through all documentation, and onboarding guide for new contributors."
tags: ["quickstart", "navigation", "onboarding", "architecture"]
verified:
  - by: openwiki/0.6.0
    at: 2026-09-28T08:16:43.565Z
sources:
  - id: openwiki-source-651d1fb6c9e49916a916ab51
    resource: repo://Cargo.toml
  - id: openwiki-source-fed88a738aecd1503ee95163
    resource: repo://crates/events/ironclaw_event_log/Cargo.toml
  - id: openwiki-source-9d8e91361abe7c88bab33dad
    resource: repo://crates/events/ironclaw_event_projections/Cargo.toml
  - id: openwiki-source-6fd632108375bdd8d105f70e
    resource: repo://crates/extensions/ironclaw_extension_host/Cargo.toml
  - id: openwiki-source-56e3e1eef5264db26e2fcfee
    resource: repo://crates/extensions/ironclaw_extension_manager/Cargo.toml
  - id: openwiki-source-a8cfc172494c5642e13c6090
    resource: repo://crates/extensions/ironclaw_extension_registry/Cargo.toml
  - id: openwiki-source-48d3512743eb6790a1abdf21
    resource: repo://crates/kernel/ironclaw_approvals/Cargo.toml
  - id: openwiki-source-04c94a2dcef5440f2a3d519b
    resource: repo://crates/kernel/ironclaw_authorization/Cargo.toml
  - id: openwiki-source-dd86b966081c0e6edfa9ab74
    resource: repo://crates/kernel/ironclaw_capabilities/Cargo.toml
  - id: openwiki-source-5364365116523441ff79facc
    resource: repo://crates/kernel/ironclaw_host_runtime/Cargo.toml
  - id: openwiki-source-a91131dae24525c5c6743ce3
    resource: repo://crates/kernel/ironclaw_processes/Cargo.toml
  - id: openwiki-source-dfdd05c1a3cde02b22859523
    resource: repo://crates/kernel/ironclaw_resources/Cargo.toml
  - id: openwiki-source-3f68f05f34c4a7b1658c6c54
    resource: repo://crates/kernel/ironclaw_runtime_policy/Cargo.toml
  - id: openwiki-source-3296dc0c286a9be8da857f86
    resource: repo://crates/kernel/ironclaw_turns/Cargo.toml
  - id: openwiki-source-8b48c59964e6201efb95adcb
    resource: repo://crates/loop/ironclaw_agent_loop/Cargo.toml
  - id: openwiki-source-23775c3de52f3ab95a13cb8b
    resource: repo://README.md
generated: { by: "openwiki/0.6.0", at: "2026-09-28T08:16:43.565Z" }
---

# IronClaw OpenWiki: Quick Start

Welcome to the IronClaw repository documentation. This is your entry point to understanding the codebase structure, how to build and test, and where to find help.

## What is IronClaw?

[IronClaw](https://github.com/nearai/ironclaw) is a **secure personal AI assistant** that protects your data and expands its capabilities on demand. It is built on a dual-stack architecture: a modern **Reborn** runtime (the future) coexisting with a **v1 legacy** monolith (maintenance mode).

**Key Properties:**
- **Secure by design:** Secrets encrypted on host, never inline in config; data encrypted at rest
- **Sandboxed execution:** Tools run in isolated WASM, process, or external sandboxes
- **Modular architecture:** 68+ crates with clear authority boundaries and composable components
- **Multi-platform:** Runs as CLI, Web UI, or embedded service across Linux, macOS, Windows
- **Extensible:** Native tools, WASM extensions, MCP servers, and OAuth integrations

## Repository Overview

```
ironclaw/
├── src/                   # v1 legacy monolith (maintenance only)
├── crates/                # Reborn runtime — active development target (68+ crates)
├── crates/ironclaw_cli/        # Primary CLI/WebUI entrypoint
├── crates/ironclaw_agent_loop/        # Core agent execution engine
├── crates/ironclaw_product_workflow/  # Product layer (flows, approvals, skills)
├── docs/                  # User-facing and draft architecture docs
├── tests/                 # E2E and integration test suites
├── .claude/               # Agent instruction files (rules, commands, skills)
├── AGENTS.md              # Quick agent rules and discovery (read first)
├── CLAUDE.md              # Detailed architecture and subsystem specs
└── README.md              # Setup and deployment guide
```

**Architecture Rule of Thumb:**
- **New features → Reborn** (`crates/` directory), not v1 `src/`
- **Maintenance only → v1** (`src/` directory)

## Quick Navigation

### 🚀 Getting Started (For New Contributors)

1. **First time here?** Read [AGENTS.md](/AGENTS.md) (2 min) for quick rules and code discovery tips
2. **Set up locally:** Run `./scripts/dev-setup.sh` and `cargo test` (see [Development Setup](development/setup.md))
3. **Understand the architecture:** Jump to [Architecture Overview](architecture/overview.md)

### 🏗️ Understanding the Code

- **[Architecture Overview](architecture/overview.md)** — High-level system design, four-layer model, crate organization
- **[Crate Reference](architecture/crates.md)** — Detailed breakdown of 68 crates, their purpose, and key types
- **[Kernel Authority & Security Boundary](architecture/kernel.md)** — Nine-stage effect pipeline, sealed artifacts, authorization and policy enforcement (essential for authorization and security changes)
- **[Turn & Execution Data Flow](architecture/data-flow.md)** — How user turns flow through the system, from admission through execution to event emission

### 🛠️ Building and Testing

- **[Development Setup](development/setup.md)** — Build environment, dependencies, quick-start commands
- **[Testing Guide](development/testing.md)** — Test tiers (unit/integration/e2e), patterns, standards, and CI/CD
- **[Common Workflows](development/workflows.md)** — How to fix a bug, add a feature, review code, deploy

### 🧩 Integrations and Extensions

- **[Extensions & Integrations](integrations/extensions.md)** — Guide to building, installing, and maintaining extensions (tools, channels, memory providers). Document the extension manifest, registry, and package structure.
- **[Channel Adapters & User Interfaces](integrations/channels.md)** — How conversations flow through channel adapters (CLI, WebUI, Slack, Telegram) and how to build a new channel integration

### 🚀 Operations and Deployment

- **[Deployment & Configuration](operations/deployment.md)** — Build, deploy, and configure IronClaw across development, staging, and production
- **[Persistence & Storage Backends](operations/database.md)** — Event sourcing architecture, multi-backend support (libSQL, PostgreSQL), and schema migrations
- **[Observability, Debugging & Logs](operations/observability.md)** — Event sourcing audit trails, structured logging, debug bundles, and troubleshooting techniques
- **[Security, Secrets & Sandboxing](operations/security.md)** — Secret protection, sandboxing enforcement, and threat mitigation

## Key Architectural Concepts

### The Dual Stack

IronClaw runs two parallel architectures:

| Aspect | v1 (src/) | Reborn (crates/) |
|--------|-----------|------------------|
| **Status** | Legacy, maintenance only | Modern, active development |
| **Model** | Monolith (~10k LOC in `src/`) | Modular (68+ focused crates) |
| **Design** | Tightly coupled services | Clear authority boundaries |
| **New Features** | ❌ Don't add here | ✅ Build here |
| **When to Touch** | Only existing v1 bugs | New features, product workflows |

### The Four-Layer Model (Reborn)

```
Products Layer (CLI, WebUI, Slack, Telegram)
        ↓ TurnCoordinator boundary (locks, serialization)
Userland Layer (Agent loops: Planned, Text, CodeAct)
        ↓ CapabilityHost boundary (policy enforcement)
Kernel Layer (Authorization, Safety, Approval gates)
        ↓ Effects boundary (hooks, subscribers)
Substrate Layer (Events, Filesystem, Memory, Threads)
```

**Core Principle:** The loop is NOT the security perimeter. Loops request effects; the kernel decides what's allowed. See [Kernel Authority & Security Boundary](architecture/kernel.md) for the nine-stage pipeline and [Turn & Execution Data Flow](architecture/data-flow.md) for end-to-end execution flows.

### Crate Organization (68 crates in 7 groups)

| Group | Purpose | Key Crates | Count |
|-------|---------|-----------|-------|
| **Core Contracts** | Shared types and traits | `host_api`, `common`, `prompt_envelope` | 5 |
| **Authority & Gates** | Security, approvals, secrets, policy | `authorization`, `safety`, `secrets`, `filesystem` | 9 |
| **Capability Execution** | Tool dispatch, WASM, MCP, scripts | `capabilities`, `dispatcher`, `wasm`, `mcp` | 11 |
| **Durable State** | Events, threads, conversations, memory | `events`, `run_state`, `threads`, `memory` | 9 |
| **Products & Loops** | Agent, CLI, WebUI, workflows | `agent_loop`, `reborn_cli`, `product_workflow` | 27 |
| **Storage Backends** | PostgreSQL, libSQL adapters | `hooks_postgres`, `reborn_event_store` | 8 |
| **Utilities** | Logging, embeddings, observability | `observability`, `embeddings`, `llm` | 7 |

**Key Rule:** Dependencies flow upward only (no circular). Substrate ← Kernel ← Userland ← Products.

## Common Tasks

### I want to...

<!-- openwiki: broken internal link [development/workflows.md#fixing-a-bug] heading anchor "fixing-a-bug" does not exist in "development/workflows.md". Fix the href or restore the target, then delete this comment. -->
- **Fix a bug:** Jump to [Workflows: Fix a Bug](development/workflows.md#fixing-a-bug) (test-first discipline required)
- **Add a new feature:** See [Architecture Overview](architecture/overview.md) and [Crate Reference](architecture/crates.md)
<!-- openwiki: broken internal link [development/workflows.md#code-review] heading anchor "code-review" does not exist in "development/workflows.md". Fix the href or restore the target, then delete this comment. -->
- **Review a pull request:** Read [Workflows: Code Review](development/workflows.md#code-review) and the [Testing Guide](development/testing.md)
- **Deploy to production:** See [Deployment & Configuration](operations/deployment.md)
- **Build an extension or tool:** See [Extensions & Integrations](integrations/extensions.md)
- **Add a new channel adapter:** See [Channel Adapters & User Interfaces](integrations/channels.md)
- **Understand capability execution:** Visit [Crate Reference](architecture/crates.md) and [Extensions & Integrations](integrations/extensions.md)
<!-- openwiki: broken internal link [AGENTS.md#code-discovery] file "AGENTS.md" does not exist. Fix the href or restore the target, then delete this comment. -->
- **Query the codebase:** Use the knowledge graph (see [AGENTS.md: Code Discovery](AGENTS.md#code-discovery)) before grep
- **Debug agent behavior:** See [Observability, Debugging & Logs](operations/observability.md)
- **Modify authorization or approval logic:** Read [Kernel Authority & Security Boundary](architecture/kernel.md) and [Turn & Execution Data Flow](architecture/data-flow.md)
- **Report a security issue:** See [Security, Secrets & Sandboxing](operations/security.md) and SECURITY.md (if present)

## Important Rules & Practices

### Code Quality
- **Zero clippy warnings** — enforced as `-D warnings`
- **No `.unwrap()` or `.expect()`** in production code (tests are fine)
- **Test-through-caller rule** — when a helper controls a side effect (HTTP, DB, OAuth), test at the call site, not the helper alone
- **Regression test required** — every bug fix must include a test case that would have caught the bug

### Architecture Discipline
- **Build new features in Reborn** (`crates/`), not v1 (`src/`)
- **Keep module logic in modules** — don't move it to entrypoints
- **Use traits and registries** — prefer extending existing extension points over hardcoding new integrations
- **No ambient authority in loops** — loops request effects; the kernel decides what's allowed
- **Secrets only in env vars** — config files must not contain secret values, only env-var names

### Security Mindset
- **Review auth, secrets, and sandboxing changes** with a security-first lens
- **Never weaken bearer tokens, CORS, body limits, or rate limits**
- **Treat external services as untrusted** — validate all input before storage or LLM calls
- **Session/thread/turn state matters** — submission parsing happens before normal chat

### Testing Strategy
- **Unit tests** for local logic (~10k tests, <1s each)
- **Integration tests** for runtime, DB, routing behavior (~1k tests, 1-60s each)
- **E2E tests** for user-visible flows (~100 tests, called explicitly)
- **Live canaries** supplemental only — never the sole regression protection

## Documentation Structure

```
openwiki/
├── quickstart.md                    # ← You are here
├── architecture/
│   ├── overview.md                  # System design, four-layer model
│   ├── crates.md                    # All 68+ crates explained
│   ├── kernel.md                    # Kernel authority and effect pipeline
│   └── data-flow.md                 # Turn submission and execution flow
├── development/
│   ├── setup.md                     # Build, dependencies, quick-start
│   ├── testing.md                   # Test tiers, patterns, CI/CD
│   └── workflows.md                 # Bug fixes, features, code review
├── integrations/
│   ├── extensions.md                # Extension architecture and building
│   └── channels.md                  # Channel adapters and integrations
├── operations/
│   ├── deployment.md                # Building and deploying IronClaw
│   ├── database.md                  # Persistence and storage backends
│   ├── observability.md             # Logging, debugging, and observability
│   └── security.md                  # Security, secrets, and sandboxing
└── .last-update.json                # Metadata (auto-updated)
```

## External Resources

- **Repository:** [github.com/nearai/ironclaw](https://github.com/nearai/ironclaw)
- **Docs (Mintlify):** [docs.ironclaw.ai](https://docs.ironclaw.ai) (user-facing)
- **Security:** See `/SECURITY.md` for responsible disclosure
- **Issue Tracker:** [GitHub Issues](https://github.com/nearai/ironclaw/issues)
- **Slack & Community:** See README.md for community links
- **Agent Rules:** [AGENTS.md](/AGENTS.md) — Start here for quick rules
- **Architecture Specs:** [CLAUDE.md](/CLAUDE.md) — Subsystem deep-dives

## Getting Help

### For different questions, different resources:

| Question | Answer In |
|----------|-----------|
| "What does this crate do?" | [Crate Reference](architecture/crates.md) |
| "How do I run tests?" | [Testing Guide](development/testing.md) |
| "What's the security model?" | [Kernel Authority & Security Boundary](architecture/kernel.md) and [Security, Secrets & Sandboxing](operations/security.md) |
| "How do capabilities and tools work?" | [Crate Reference](architecture/crates.md) and [Extensions & Integrations](integrations/extensions.md) |
<!-- openwiki: broken internal link [/AGENTS.md#where-to-work] heading anchor "where-to-work" does not exist in "/AGENTS.md". Fix the href or restore the target, then delete this comment. -->
| "Where do I add a new feature?" | [Architecture Overview](architecture/overview.md) + [AGENTS.md: Where to Work](/AGENTS.md#where-to-work) |
| "How does a turn flow through the system?" | [Turn & Execution Data Flow](architecture/data-flow.md) |
| "How do I debug agent behavior?" | [Observability, Debugging & Logs](operations/observability.md) |
| "What's this error?" | [Observability, Debugging & Logs](operations/observability.md) |
| "What's a 'turn'?" | [Crate Reference](architecture/crates.md) (glossary section) |
| "How do I build a channel?" | [Channel Adapters & User Interfaces](integrations/channels.md) |
| "How do I build an extension?" | [Extensions & Integrations](integrations/extensions.md) |

### Direct Code Exploration

When these docs don't answer your question:

<!-- openwiki: broken internal link [/AGENTS.md#code-discovery---query-the-knowledge-graph-first] heading anchor "code-discovery---query-the-knowledge-graph-first" does not exist in "/AGENTS.md". Fix the href or restore the target, then delete this comment. -->
1. **Use the knowledge graph** (faster than grep): See [AGENTS.md: Code Discovery](/AGENTS.md#code-discovery---query-the-knowledge-graph-first)
2. **Read subsystem specs** in [CLAUDE.md](/CLAUDE.md) (detailed architecture per crate/module)
3. **Check crate README/AGENTS files** (many crates have their own docs in `src/` or `Cargo.toml`)
4. **Inspect contract tests** (look for `*_contract.rs` files — they are documentation in code)

## Next Steps

- **Beginner?** Start with [Development Setup](development/setup.md) and run `cargo test`
<!-- openwiki: broken internal link [development/workflows.md#code-review] heading anchor "code-review" does not exist in "development/workflows.md". Fix the href or restore the target, then delete this comment. -->
- **Reviewer?** Jump to [Workflows: Code Review](development/workflows.md#code-review)
- **Architect?** Read [Architecture Overview](architecture/overview.md), [Kernel Authority & Security Boundary](architecture/kernel.md), and [CLAUDE.md](/CLAUDE.md)
- **Extension developer?** See [Extensions & Integrations](integrations/extensions.md)
- **Operations?** Start with [Deployment & Configuration](operations/deployment.md) and [Observability, Debugging & Logs](operations/observability.md)
- **Seeking a specific feature?** Use the navigation table above or grep the docs

---

**Last updated:** Auto-generated by [OpenWiki](https://github.com/nearai/openwiki) on each commit. For updates, file an issue or PR against the repository.
