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
├── crates/ironclaw_reborn_cli/        # Primary CLI/WebUI entrypoint
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
- **[Data Model](architecture/data-model.md)** — Events, runs, threads, turns, capabilities, and state flows
- **[Security & Safety](architecture/security.md)** — Kernel/userland boundary, policy enforcement, threat model

### 🛠️ Building and Testing

- **[Development Setup](development/setup.md)** — Build environment, dependencies, quick-start commands
- **[Testing Guide](development/testing.md)** — Test tiers (unit/integration/e2e), patterns, standards, and CI/CD
- **[Common Workflows](development/workflows.md)** — How to fix a bug, add a feature, review code, deploy

### 📚 Domain Deep Dives

- **[Agent Loop & Execution](domains/agent-loop.md)** — How turns flow through planning, execution, and checkpointing
- **[Capabilities & Tools](domains/capabilities.md)** — How tools are registered, approved, and executed
- **[Memory & Persistence](domains/memory.md)** — Event store, snapshots, recovery, and indexing
- **[Product Workflow](domains/product-workflow.md)** — Missions, projects, skills, routines, and approvals
- **[Channels & Integrations](domains/channels.md)** — Slack, WebUI, Discord, and custom channel adapters

### 📖 Reference

- **[Glossary](reference/glossary.md)** — Key terminology and concepts
- **[API Surface](reference/api.md)** — HTTP endpoints, WebSocket events, CLI commands
- **[Configuration](reference/configuration.md)** — Startup options, environment variables, and config.toml schema
- **[Troubleshooting](reference/troubleshooting.md)** — Common errors, debugging tips, and support

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

**Core Principle:** The loop is NOT the security perimeter. Loops request effects; the kernel decides what's allowed.

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

- **Fix a bug:** Jump to [Workflows: Fix a Bug](development/workflows.md#fixing-a-bug) (test-first discipline required)
- **Add a new feature:** See [Architecture Overview](architecture/overview.md#where-to-build-new-features) and [Crate Reference](architecture/crates.md)
- **Review a pull request:** Read [Workflows: Code Review](development/workflows.md#code-review) and the [Testing Guide](development/testing.md)
- **Deploy to production:** See [Configuration](reference/configuration.md) and Dockerfile patterns in `crates/ironclaw_reborn_cli`
- **Understand a capability:** Visit [Capabilities & Tools](domains/capabilities.md)
- **Query the codebase:** Use the knowledge graph (see [AGENTS.md: Code Discovery](AGENTS.md#code-discovery)) before grep
- **Report a security issue:** See [Security & Safety](architecture/security.md) and SECURITY.md (if present)

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
│   ├── data-model.md                # Events, state, persistence
│   └── security.md                  # Kernel boundary, threats
├── development/
│   ├── setup.md                     # Build, dependencies, quick-start
│   ├── testing.md                   # Test tiers, patterns, CI/CD
│   └── workflows.md                 # Bug fixes, features, code review
├── domains/
│   ├── agent-loop.md                # Execution engine
│   ├── capabilities.md              # Tools and extensibility
│   ├── memory.md                    # Persistence and indexing
│   ├── product-workflow.md          # Missions, skills, approvals
│   └── channels.md                  # Slack, WebUI, integrations
├── reference/
│   ├── glossary.md                  # Terminology
│   ├── api.md                       # HTTP, WebSocket, CLI
│   ├── configuration.md             # Config.toml, env vars
│   └── troubleshooting.md           # Common errors, debugging
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
| "What's the security model?" | [Security & Safety](architecture/security.md) |
| "How do capabilities work?" | [Capabilities & Tools](domains/capabilities.md) |
| "Where do I add a new feature?" | [Architecture Overview](architecture/overview.md#where-to-build-new-features) + [AGENTS.md: Where to Work](/AGENTS.md#where-to-work) |
| "What's this error?" | [Troubleshooting](reference/troubleshooting.md) |
| "What's a 'turn'?" | [Glossary](reference/glossary.md) |

### Direct Code Exploration

When these docs don't answer your question:

1. **Use the knowledge graph** (faster than grep): See [AGENTS.md: Code Discovery](/AGENTS.md#code-discovery---query-the-knowledge-graph-first)
2. **Read subsystem specs** in [CLAUDE.md](/CLAUDE.md) (detailed architecture per crate/module)
3. **Check crate README/AGENTS files** (many crates have their own docs in `src/` or `Cargo.toml`)
4. **Inspect contract tests** (look for `*_contract.rs` files — they are documentation in code)

## Next Steps

- **Beginner?** Start with [Development Setup](development/setup.md) and run `cargo test`
- **Reviewer?** Jump to [Workflows: Code Review](development/workflows.md#code-review)
- **Architect?** Read [Architecture Overview](architecture/overview.md) and [CLAUDE.md](/CLAUDE.md)
- **Seeking a specific feature?** Use the navigation table above or grep the docs

---

**Last updated:** Auto-generated by [OpenWiki](https://github.com/nearai/openwiki) on each commit. For updates, file an issue or PR against the repository.
