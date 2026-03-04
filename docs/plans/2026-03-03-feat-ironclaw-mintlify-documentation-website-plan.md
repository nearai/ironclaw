---
title: "feat: IronClaw Mintlify Documentation Website"
type: feat
status: active
date: 2026-03-03
origin: docs/brainstorms/2026-03-03-ironclaw-docs-website-brainstorm.md
---

# IronClaw Mintlify Documentation Website

## Overview

Build a standalone documentation website for IronClaw — a secure, self-hosted AI assistant — targeting integrators and end-users (not developers). The site will be built with **Mintlify** (same platform as OpenClaw docs), live in a separate **`ironclaw-docs`** GitHub repository, and be hosted at **`docs.ironclaw.ai`** on Vercel or Mintlify's own CDN.

The primary differentiator from generic software docs: IronClaw's **security architecture** (safety layer, secrets encryption, sandbox isolation, prompt injection defense) is front-and-center, not buried in reference material.

**Key decisions from brainstorm** *(see brainstorm: docs/brainstorms/2026-03-03-ironclaw-docs-website-brainstorm.md)*:
- Platform: Mintlify — identical component library to OpenClaw docs
- Repo: Separate (`ironclaw-docs`)
- Domain: `docs.ironclaw.ai` (placeholder; configure post-deploy)
- Nav: Feature-first primary nav + Guides section for use-case walkthroughs
- Brand: Fetch from ironclaw.com; use placeholders in Phase 1
- Deployment: Mintlify hosting with custom CNAME (preferred over static Vercel export — see deployment section)

---

## Problem Statement

IronClaw's target audience — integrators, system operators, and end-users — have no accessible documentation. The codebase contains detailed developer-oriented specs (`src/setup/README.md`, `src/NETWORK_SECURITY.md`, `CLAUDE.md`) and scattered partial guides (`docs/TELEGRAM_SETUP.md`, `docs/LLM_PROVIDERS.md`) but no coherent, user-friendly entry point. New users must read Rust source comments and internal specs to accomplish basic tasks like connecting Telegram or switching LLM providers.

---

## Proposed Solution

A Mintlify docs site in a separate `ironclaw-docs` repo, built in two phases:

**Phase 1 (MVP):** Home, Getting Started, Install, Configuration, Channels (Telegram + Web Gateway + Webhook), LLM Providers, Security Overview, Troubleshooting — everything a new user needs to go from zero to running IronClaw.

**Phase 2:** Skills, Tools (WASM/MCP), Memory/Workspace, Routines, Guides (use-case walkthroughs), remaining channels (Signal, Discord, Slack, WhatsApp), full Reference section.

---

## Technical Approach

### Deployment Model Decision

Two Mintlify deployment options exist. **Mintlify hosting with custom CNAME is the recommended approach:**

| Model | Pros | Cons |
|---|---|---|
| **Mintlify hosting + CNAME** (recommended) | Full component support (feedback widget, analytics, search indexing, AI chat); simple GitHub push to deploy; no build step needed | Domain must be verified in Mintlify dashboard |
| `mintlify build` + Vercel static export | Runs on Vercel infra | Missing Mintlify-cloud-only features; feedback widget broken; search may not index correctly |

> **Decision:** Use Mintlify's own hosting with a CNAME DNS record pointing `docs.ironclaw.ai` to the Mintlify CDN. Vercel can still be used as a backup/preview environment if desired.

### Repository Structure

New repository: `github.com/[org]/ironclaw-docs`

```
ironclaw-docs/
├── docs.json                    # Mintlify config (first deliverable)
├── package.json                 # Pinned mintlify CLI version
├── .github/
│   └── workflows/
│       └── build-check.yml      # CI: mintlify build on every PR
├── assets/
│   ├── logo-light.svg           # Placeholder → replace from ironclaw.com
│   ├── logo-dark.svg            # Placeholder → replace from ironclaw.com
│   └── favicon.svg
├── images/                      # Screenshots, diagrams
│
├── index.mdx                    # Home page
│
├── start/
│   ├── getting-started.mdx      # Quickstart: zero to running
│   └── wizard.mdx               # Onboarding wizard walkthrough (8 steps)
│
├── install/
│   ├── index.mdx                # Install overview + method chooser
│   ├── local.mdx                # Local: Linux / macOS / Windows tabs
│   ├── docker.mdx               # Docker deployment
│   ├── vps.mdx                  # VPS / cloud server
│   ├── nearai-cloud.mdx         # NEAR AI managed hosting
│   ├── updating.mdx             # Updating IronClaw
│   └── uninstalling.mdx         # Clean uninstall
│
├── setup/
│   ├── configuration.mdx        # Complete env var reference table
│   └── database.mdx             # PostgreSQL vs libSQL/Turso
│
├── channels/
│   ├── index.mdx                # Channels overview
│   ├── web-gateway.mdx          # Browser UI (Web Gateway)
│   ├── telegram.mdx             # Telegram setup (import from TELEGRAM_SETUP.md)
│   ├── webhook.mdx              # HTTP Webhook channel
│   ├── tui.mdx                  # Terminal UI
│   └── signal.mdx               # Signal (stub — "in progress")
│
├── providers/
│   ├── index.mdx                # Provider overview + comparison table
│   ├── nearai.mdx               # NEAR AI (OAuth + API key paths)
│   ├── anthropic.mdx            # Anthropic / Claude
│   ├── openai.mdx               # OpenAI / GPT
│   ├── ollama.mdx               # Ollama (local inference)
│   ├── openai-compatible.mdx    # OpenRouter, Together, vLLM, LM Studio
│   └── tinfoil.mdx              # Tinfoil (private TEE inference)
│
├── security/
│   ├── index.mdx                # Security overview + architecture diagram
│   ├── safety-layer.mdx         # Sanitizer, validator, policy, leak detector
│   ├── secrets.mdx              # Secrets management + zero-exposure model
│   └── sandbox.mdx              # Docker sandbox + network proxy
│
├── skills/                      # Phase 2
│   ├── index.mdx
│   ├── trust-model.mdx
│   ├── skill-format.mdx
│   ├── installing.mdx
│   └── clawhub.mdx
│
├── tools/                       # Phase 2
│   ├── index.mdx
│   ├── builtin.mdx
│   ├── wasm.mdx
│   ├── mcp.mdx
│   └── building.mdx
│
├── memory/                      # Phase 2
│   ├── index.mdx
│   ├── writing-reading.mdx
│   ├── search.mdx
│   ├── identity-files.mdx
│   └── heartbeat.mdx
│
├── routines/                    # Phase 2
│   ├── index.mdx
│   ├── cron.mdx
│   ├── event-driven.mdx
│   └── webhook-triggers.mdx
│
├── guides/                      # Phase 2
│   ├── index.mdx
│   ├── telegram-in-10-minutes.mdx
│   ├── connect-claude.mdx
│   ├── vps-deployment.mdx
│   ├── nearai-cloud-deployment.mdx
│   ├── secure-your-deployment.mdx
│   └── first-wasm-tool.mdx
│
├── reference/
│   ├── cli.mdx                  # CLI command reference
│   ├── env-vars.mdx             # Consolidated env var table (Phase 1)
│   ├── api.mdx                  # Web Gateway API endpoints (Phase 2)
│   └── changelog.mdx            # Import from CHANGELOG.md
│
└── help/
    ├── troubleshooting.mdx      # Common errors + ironclaw doctor
    └── faq.mdx                  # FAQ
```

---

## Phase 1: Implementation Plan

### Step 1: Repository Bootstrap (Day 1)

**Deliverables:**
- Create `ironclaw-docs` GitHub repo
- `docs.json` — complete Mintlify config (first and most important file)
- `package.json` — pinned `mintlify` CLI (e.g., `"mintlify": "4.x"`)
- `.github/workflows/build-check.yml` — CI on every PR

**`docs.json` outline:**
```json
{
  "$schema": "https://mintlify.com/docs.json",
  "name": "IronClaw",
  "description": "Secure personal AI assistant. Your data stays yours.",
  "theme": "mint",
  "icons": { "library": "lucide" },
  "logo": {
    "light": "/assets/logo-light.svg",
    "dark": "/assets/logo-dark.svg"
  },
  "favicon": "/assets/favicon.svg",
  "colors": {
    "primary": "#5B6FFF",
    "dark": "#5B6FFF",
    "light": "#8B9AFF"
  },
  "navbar": {
    "links": [
      { "label": "GitHub", "href": "https://github.com/[org]/ironclaw", "icon": "github" },
      { "label": "Releases", "href": "https://github.com/[org]/ironclaw/releases", "icon": "package" }
    ]
  },
  "navigation": {
    "tabs": [
      {
        "tab": "Docs",
        "groups": [
          { "group": "Get Started", "pages": ["index", "start/getting-started", "start/wizard"] },
          { "group": "Install", "pages": ["install/index", "install/local", "install/docker", "install/vps", "install/nearai-cloud", "install/updating", "install/uninstalling"] },
          { "group": "Setup", "pages": ["setup/configuration", "setup/database"] },
          { "group": "Channels", "pages": ["channels/index", "channels/web-gateway", "channels/telegram", "channels/webhook", "channels/tui", "channels/signal"] },
          { "group": "LLM Providers", "pages": ["providers/index", "providers/nearai", "providers/anthropic", "providers/openai", "providers/ollama", "providers/openai-compatible", "providers/tinfoil"] },
          { "group": "Security", "pages": ["security/index", "security/safety-layer", "security/secrets", "security/sandbox"] },
          { "group": "Reference", "pages": ["reference/cli", "reference/env-vars", "reference/changelog"] },
          { "group": "Help", "pages": ["help/troubleshooting", "help/faq"] }
        ]
      }
    ]
  },
  "cname": "docs.ironclaw.ai"
}
```

**GitHub Actions CI (`build-check.yml`):**
```yaml
name: Build Check
on: [push, pull_request]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: "20" }
      - run: npm ci
      - run: npx mintlify build
```

---

### Step 2: Home Page (`index.mdx`) (Day 1-2)

**Content:**
- Hero section: tagline + 3 quick-start cards (Get Started, Web Gateway, Security)
- Mermaid flowchart: how IronClaw works (channels → agent → tools → memory)
- "What is IronClaw?" — 3-paragraph overview (vs. OpenClaw: local, secure, Rust binary)
- Key capabilities grid (6 cards): Multi-channel, Security-first, Multi-provider LLM, Skills system, Parallel jobs, Persistent memory
- Quick Start steps (3 steps: install, run wizard, open web UI)
- "Start here" card grid (6 cards linking to major sections)

**Source:** IronClaw README.md architecture diagram + feature list

---

### Step 3: Getting Started (`start/getting-started.mdx`) (Day 2)

**Goal:** Zero to running in under 10 minutes, one page.

**Content:**
1. Prerequisites callout (OS, optional Docker for sandbox)
2. Three paths: local (libSQL, simplest), Docker, VPS
3. Install IronClaw (tabs: Linux/macOS, Windows, WSL2) — lead with shell script
4. Run wizard: `ironclaw onboard`
5. Start the agent: `ironclaw run`
6. Open Web Gateway (if enabled): `http://127.0.0.1:3000`
7. Success checkmark: "If you see the chat interface, you're ready"

**Critical additions from SpecFlow:**
- Add a `<Note>` block: "VPS or remote server? See [Running on a VPS](/install/vps) — browser OAuth has special requirements"
- Link to wizard walkthrough page for full details

---

### Step 4: Onboarding Wizard (`start/wizard.mdx`) (Day 2-3)

**Source:** `src/setup/README.md` — authoritative spec (8 steps, NOT 7)

**Content structure:**
- Overview: what the wizard configures, how to run it (`ironclaw onboard`, `--skip-auth`, `--channels-only`)
- `<Steps>` component with 8 numbered steps:
  1. Database Connection (PostgreSQL vs libSQL chooser)
  2. Security / Master Key (OS keychain, env var, or skip)
  3. Inference Provider (NEAR AI OAuth, API key, or alternative)
  4. Model Selection (fetched from provider)
  5. Embeddings (enable semantic search)
  6. Channel Configuration (multi-select: TUI, Web, Telegram, etc.)
  7. Extensions / Tools (install from registry)
  8. Heartbeat (proactive background execution)
- "Re-running the wizard" section: `ironclaw onboard --channels-only` for adding channels later
- Platform caveats `<Accordion>` group:
  - macOS Keychain: two system dialogs expected
  - Linux: requires `gnome-keyring`
  - Remote server: browser OAuth blocked — use API key or `IRONCLAW_OAUTH_CALLBACK_URL`

**SpecFlow fix:** Resolve 7-step vs. 8-step inconsistency — use authoritative 8-step count from `src/setup/README.md`

---

### Step 5: Install Pages (Day 3-4)

#### `install/index.mdx` — Overview
- Method chooser table: Local / Docker / VPS / NEAR AI Cloud
- Lead with libSQL as the default for local installs (zero-dependency)
- System requirements: OS, optional Docker (for sandbox jobs, not for running IronClaw itself)

#### `install/local.mdx` — Local Install
**Tabs: Linux | macOS | Windows (native) | Windows (WSL2)**

Each tab covers:
- Install method (shell script / Homebrew / MSI)
- Database recommendation (libSQL for local, note PostgreSQL option)
- Keychain behavior note (macOS: Keychain dialogs; Linux: gnome-keyring; Windows: DPAPI)
- Service installation: `ironclaw service install` (systemd on Linux, launchd on macOS)

**SpecFlow gap addressed:** Windows native vs. WSL2 are meaningfully different and need separate tab content.

#### `install/docker.mdx` — Docker
**Critical clarification box:** "IronClaw runs **alongside** Docker (for job sandboxing). This page covers running IronClaw itself in a container — a different use case."

- `docker-compose.yml` example with env var injection
- Volume mounts: `~/.ironclaw` → `/home/ironclaw/.ironclaw`, Docker socket for sandbox jobs
- `IRONCLAW_BASE_DIR` env var for custom data directory
- Docker-in-Docker warning for sandbox jobs inside container

#### `install/vps.mdx` — VPS / Cloud Server
- Linux shell script install
- PostgreSQL + pgvector prerequisites (explicit: `apt install postgresql-15-pgvector`)
- **NEAR AI OAuth on VPS warning** (SpecFlow Gap 1.3 — Phase 1 blocker):
  ```
  <Warning>
  Browser OAuth (the default) requires a browser on the same machine as IronClaw.
  On a VPS, use NEAR AI Cloud API key (Step 3 → "API Key" option) or set
  IRONCLAW_OAUTH_CALLBACK_URL to a publicly reachable URL.
  </Warning>
  ```
- **Firewall note** (SpecFlow Gap 4.2): orchestrator binds `0.0.0.0:50051` on Linux — add `ufw deny 50051`
- Service: `ironclaw service install` (systemd unit)
- Reverse proxy for Web Gateway TLS (nginx/Caddy example)

#### `install/nearai-cloud.mdx` — NEAR AI Cloud
- What "managed hosting" means (NEAR AI runs IronClaw for you)
- How to access (web UI URL, token injection)
- `NEARAI_SESSION_TOKEN` env var injection

#### `install/updating.mdx`
- How to update for each install method (shell script re-run, `brew upgrade ironclaw`, Docker pull, cargo update)

---

### Step 6: Configuration Reference (`setup/configuration.mdx`) (Day 4-5)

**Source:** Consolidate all `src/config/` subdirectory modules into one master table.

**Structure:**
- `<AccordionGroup>` sections, one per config category:
  - Agent settings (`AGENT_*`)
  - Database (`DATABASE_*`, `LIBSQL_*`)
  - LLM / Inference (`LLM_BACKEND`, `NEARAI_*`, `ANTHROPIC_*`, `OPENAI_*`, etc.)
  - Channels (per channel, all env vars)
  - Web Gateway (`GATEWAY_*`)
  - HTTP Webhook (`HTTP_*`)
  - Docker Sandbox (`SANDBOX_*`, `CLAUDE_CODE_*`)
  - Skills (`SKILLS_*`)
  - Embeddings (`EMBEDDING_*`)
  - Heartbeat (`HEARTBEAT_*`)
  - Routines (`ROUTINES_*`)
  - Security (`IRONCLAW_OAUTH_CALLBACK_URL`, `IRONCLAW_BASE_DIR`)

Each env var entry: `VAR_NAME` | type | default | description

**Note:** `DATABASE_BACKEND` and `DATABASE_URL` belong in `~/.ironclaw/.env` (bootstrap layer) — add callout explaining the two-layer config system.

#### `setup/database.mdx` — Database Backends

- Comparison table: PostgreSQL vs. libSQL/Turso
- PostgreSQL: version requirements (15+), pgvector installation (`apt install postgresql-15-pgvector`), SSL modes
- libSQL: zero-dependency, auto-created at `~/.ironclaw/ironclaw.db`, Turso cloud sync option
- **libSQL encryption-at-rest callout** (SpecFlow Gap 4.3):
  ```
  <Warning>
  libSQL stores conversation and workspace data in plaintext. Only secrets (API tokens,
  credentials) are encrypted with AES-256-GCM. If you handle sensitive data, use
  full-disk encryption or the PostgreSQL backend.
  </Warning>
  ```
- Feature comparison: libSQL FTS-only search vs. PostgreSQL hybrid (FTS + vector)

---

### Step 7: Channel Pages (Day 5-7)

#### `channels/index.mdx` — Overview
- Channel types diagram (built-in vs. WASM)
- Available channels table with status and setup complexity

#### `channels/web-gateway.mdx` — Web Gateway (Browser UI)
- What it is, how to enable (`GATEWAY_ENABLED=true`)
- All tabs in the Web UI: Chat, Memory, Jobs, Routines, Extensions, Skills
- **Auth token setup** (SpecFlow Gap 3.4):
  - Default: random token generated at startup (printed in logs)
  - Persistent: set `GATEWAY_AUTH_TOKEN` env var
  - Where to find it: `RUST_LOG=ironclaw=info ironclaw run | grep token`
- Access options: localhost-only vs. LAN (`GATEWAY_HOST` setting)
- **TLS/reverse proxy callout**: "For external access, front with nginx/Caddy for HTTPS"
- WebSocket connection indicator in UI

#### `channels/telegram.mdx` — Telegram
**Source:** Import from `docs/TELEGRAM_SETUP.md` and restructure for Mintlify components.

Key sections:
- Prerequisites (Telegram bot token from @BotFather)
- Configuration env vars
- Polling vs. Webhook mode (with tunnel setup for webhooks)
- DM pairing flow (`ironclaw pairing approve telegram`)
- **Tunnel caveats** (SpecFlow Gap 3.1): ngrok free tier changes URL on restart → webhook re-registration required
- **Owner binding wait callout** (SpecFlow Gap 3.3): 120-second wait in wizard for first message
- Troubleshooting section

#### `channels/webhook.mdx` — HTTP Webhook
- Default bind: `0.0.0.0:8080`
- **Security callout** (SpecFlow Gap 3.7):
  ```
  <Warning>
  The HTTP webhook server binds to all network interfaces by default (0.0.0.0:8080).
  If you don't need external webhook delivery, set HTTP_HOST=127.0.0.1.
  </Warning>
  ```
- Shared secret validation
- Rate limiting (64 KB body, 60 req/min)
- Payload format and response format

#### `channels/tui.mdx` — Terminal UI
- How to use the TUI (keyboard shortcuts, message composer, approval overlays)
- Enabling/disabling (`CLI_ENABLED`)

#### `channels/signal.mdx` — Signal (Stub)
```
<Info>
Signal channel documentation is coming soon. The channel is fully implemented —
documentation is being written. See the configuration reference for SIGNAL_* env vars.
</Info>
```

---

### Step 8: LLM Provider Pages (Day 7-8)

**Source:** Import from `docs/LLM_PROVIDERS.md` and split into per-provider pages.

#### `providers/index.mdx` — Overview
- Provider comparison table (all 7 providers)
- Selection guide: "Which provider should I use?"
- How to switch providers (env var change + restart)

#### `providers/nearai.mdx` — NEAR AI (Default)
- Two auth modes (Session Token OAuth, API Key)
- OAuth flow: how it works, where session is saved (`~/.ironclaw/session.json`)
- **VPS callout** (SpecFlow Gap 1.3): browser OAuth blocked on VPS — use API key or `IRONCLAW_OAUTH_CALLBACK_URL`
- Session renewal (auto-renewal, manual with `ironclaw config set nearai.session_token ...`)
- Model selection: `NEARAI_MODEL` env var, available models
- Advanced: `NEARAI_CHEAP_MODEL`, `NEARAI_FALLBACK_MODEL`, circuit breaker settings

#### `providers/anthropic.mdx` — Anthropic
- API key setup (`ANTHROPIC_API_KEY`)
- Default model (`claude-sonnet-4-20250514`)
- Custom base URL for enterprise

#### `providers/openai.mdx` — OpenAI
#### `providers/ollama.mdx` — Ollama (local)
#### `providers/openai-compatible.mdx` — OpenAI-Compatible
- OpenRouter, Together AI, Fireworks AI, vLLM, LiteLLM, LM Studio examples
- `LLM_EXTRA_HEADERS` for OpenRouter attribution

#### `providers/tinfoil.mdx` — Tinfoil (Private TEE Inference)
- What Tinfoil is (hardware-attested TEE — neither Tinfoil nor cloud can see prompts)
- Setup: `TINFOIL_API_KEY`, `TINFOIL_MODEL` (default: `kimi-k2-5`)

---

### Step 9: Security Pages (Day 8-9)

#### `security/index.mdx` — Security Overview
**This is IronClaw's primary differentiator. Lead with the architecture diagram.**

Sections:
- Security-first design philosophy (multi-layer defense)
- Architecture diagram: data flow through safety layers
- The 4 defense layers (Safety Layer → WASM Sandbox → Docker Sandbox → Secrets Encryption)
- Quick-reference security defaults table
- Links to detailed pages

#### `security/safety-layer.mdx` — Safety Layer
- What it is (sanitizer, validator, policy engine, leak detector)
- How tool output is wrapped: `<tool_output name="..." sanitized="true">` format
- Leak detection: 15+ secret patterns, three actions (Block, Redact, Warn)
- Shell environment scrubbing before command execution
- Policy severity levels (Critical/High/Medium/Low)

#### `security/secrets.mdx` — Secrets Management
- **Zero-exposure credential model** (SpecFlow Gap 4.1):
  - Secrets stored AES-256-GCM encrypted in database
  - Injected into HTTP requests at host boundary via proxy
  - Container processes never see raw credential values
  - Architecture diagram: secret never leaves host boundary
- Where secrets are stored (`secrets` DB table)
- Master key options (OS keychain, env var, skip)

#### `security/sandbox.mdx` — Docker Sandbox
- Two Docker concepts clarified:
  1. IronClaw itself (not containerized by default)
  2. IronClaw launches containers for job execution — this is the sandbox
- Sandbox policies: ReadOnly, WorkspaceWrite, FullAccess
- Network proxy: domain allowlist, CONNECT tunnel validation
- Zero-exposure credential injection
- Container hardening: non-root (UID 1000), read-only rootfs, dropped capabilities

---

### Step 10: Reference & Help Pages (Day 9-10)

#### `reference/cli.mdx` — CLI Reference
- All `ironclaw` subcommands with descriptions:
  - `run`, `onboard`, `config`, `tool`, `registry`, `mcp`, `memory`, `pairing`, `service`, `doctor`, `status`, `completion`
- Usage examples for most common operations

#### `reference/changelog.mdx`
**Source:** Import from `CHANGELOG.md` (Keep-a-Changelog format).

#### `help/troubleshooting.mdx` — Troubleshooting
**Source:** Extract from `src/NETWORK_SECURITY.md` known findings + write new content.

Key sections:
- **`ironclaw doctor`** (SpecFlow Gap 1.5): show example output, map each check to a fix
  - Database connectivity
  - NEAR AI session validity
  - Docker availability
  - cloudflared / ngrok / tailscale detection
- Common setup errors:
  - "Browser did not open" (VPS OAuth) → API key workaround
  - pgvector extension not found → `apt install postgresql-15-pgvector`
  - Telegram bot not responding → polling vs. webhook mode check
  - Web Gateway token not found → log grep command
  - Wizard interrupted mid-run → "Settings saved after each step; re-run to continue"
- Platform-specific issues (macOS Keychain dialogs, Linux gnome-keyring)

#### `help/faq.mdx` — FAQ
- Do I need Docker? (No for basic use, yes for job sandboxing)
- Can I run without PostgreSQL? (Yes — libSQL is zero-dependency)
- Is my data sent to NEAR AI? (Only prompts/responses via chosen provider)
- How do I add a second channel? (`ironclaw onboard --channels-only`)
- What's the difference between skills and tools?

---

## Alternative Approaches Considered

| Approach | Verdict |
|---|---|
| Docusaurus | Rejected — traditional sidebar nav, no rich component library matching OpenClaw |
| Mintlify static export to Vercel | Rejected — loses feedback widget, analytics, search indexing (cloud-only features) |
| In-repo docs (`ironclaw-src/docs/`) | Rejected — docs team independence, own CI, cleaner separation |
| Use-case-first nav only | Rejected — harder to maintain, features scattered; added as Guides section instead |

---

## System-Wide Impact

### Interaction Graph
Docs repo is a standalone content repository with no direct code dependencies. The chain of concern is:
- IronClaw source code changes → env vars/behavior changes → docs become stale
- Docs pages reference specific `ironclaw` CLI commands → CLI changes break doc steps
- `CHANGELOG.md` is imported → must be kept up to date in source repo

### Content Freshness Strategy (SpecFlow Gap 6.3)
- **Env var reference** (`setup/configuration.mdx`): hand-maintained table sourced from `src/config/` modules; update procedure documented in docs repo `CONTRIBUTING.md`
- **Version callout**: each page template includes a frontmatter field `verified_version: "0.13.x"` so readers know recency
- **Stale tracking**: docs repo CONTRIBUTING.md documents: "When a feature changes in `ironclaw-src`, open a docs PR linking the code PR"

### Mintlify Deployment Sequence (SpecFlow Gap 5.3)
Step-by-step to avoid chicken-and-egg:
1. Create `ironclaw-docs` GitHub repo
2. Add Mintlify GitHub App to the repo (from mintlify.com/dashboard)
3. Mintlify auto-deploys from `main` branch → initial URL: `ironclaw.mintlify.app`
4. Verify site builds correctly at Mintlify subdomain
5. Add CNAME record in DNS: `docs` → `ironclaw.mintlify.app`
6. Configure custom domain in Mintlify dashboard: `docs.ironclaw.ai`
7. Verify HTTPS cert provisioning (Mintlify handles automatically)

---

## Acceptance Criteria

### Repository & Infrastructure
- [ ] `ironclaw-docs` repo created with Mintlify GitHub App installed
- [ ] `docs.json` complete with all Phase 1 nav groups and pages
- [ ] `package.json` with pinned `mintlify` CLI version
- [ ] CI workflow runs `mintlify build` on every PR; fails on broken links or build errors
- [ ] Mintlify deploys from `main` branch automatically
- [ ] `docs.ironclaw.ai` CNAME resolves with valid HTTPS certificate

### Home Page
- [ ] Hero section loads with IronClaw logo (placeholder or real from ironclaw.com)
- [ ] Mermaid architecture diagram renders correctly in both light/dark mode
- [ ] All 6 quick-start card links resolve to valid pages

### Getting Started
- [ ] A user following the quickstart from scratch on a clean Linux machine reaches a running `ironclaw` process
- [ ] VPS warning callout present linking to `/install/vps`
- [ ] Success state is clearly defined (terminal output snippet or screenshot)

### Install
- [ ] Local install page has 4 tabs: Linux, macOS, Windows (native), Windows (WSL2)
- [ ] libSQL is presented as the default for local installs (not PostgreSQL)
- [ ] VPS page includes PostgreSQL + pgvector prerequisites and OAuth workaround
- [ ] Docker page clarifies IronClaw-in-Docker vs. sandbox-Docker distinction
- [ ] All install methods include an "Updating" path

### Configuration
- [ ] Complete env var reference table covers all variables from all `src/config/` modules
- [ ] Two-layer config system (`.env` bootstrap vs. DB settings) is explained
- [ ] libSQL encryption-at-rest limitation clearly stated with `<Warning>` callout

### Channels
- [ ] Web Gateway page covers: auth token setup, localhost vs. LAN access, TLS recommendation
- [ ] Telegram page includes: polling vs. webhook, tunnel caveats, owner binding wait callout
- [ ] Webhook page includes: `0.0.0.0` binding security warning
- [ ] Signal page exists as a stub with "coming soon" callout

### LLM Providers
- [ ] All 7 providers have pages: NEAR AI, Anthropic, OpenAI, Ollama, OpenAI-compatible, Tinfoil
- [ ] NEAR AI page includes VPS/remote OAuth workaround
- [ ] Tinfoil page explains TEE privacy model

### Security
- [ ] Security overview features the zero-exposure credential model prominently
- [ ] Safety layer page explains tool output wrapping format
- [ ] Sandbox page clarifies job-sandbox Docker vs. IronClaw-in-Docker
- [ ] Orchestrator Linux `0.0.0.0:50051` firewall note present in VPS install page

### Reference & Help
- [ ] CLI reference covers all `ironclaw` subcommands
- [ ] Troubleshooting page covers `ironclaw doctor` with example output
- [ ] Wizard step count is consistently 8 (not 7) throughout all docs

### Style & Quality
- [ ] All code blocks use correct Prism.js language identifiers (`bash` not `sh`, `toml` not `TOML`)
- [ ] No broken internal links (enforced by CI)
- [ ] Pages render correctly in both light and dark mode
- [ ] Mobile-responsive layout verified on 375px viewport

---

## Content Migration Map

| Source File | Destination | Work Required |
|---|---|---|
| `docs/LLM_PROVIDERS.md` | `providers/*.mdx` (split per provider) | Split into files, add Mintlify components |
| `docs/TELEGRAM_SETUP.md` | `channels/telegram.mdx` | Restructure with `<Steps>`, `<Tabs>`, add caveats |
| `docs/BUILDING_CHANNELS.md` | Phase 2 only (integrator guide) | Defer |
| `src/NETWORK_SECURITY.md` | `security/*.mdx` + `help/troubleshooting.mdx` | Simplify for end-users, extract findings |
| `src/setup/README.md` | `start/wizard.mdx` | Full rewrite for end-users, keep 8-step structure |
| `src/workspace/README.md` | Phase 2: `memory/*.mdx` | Defer |
| `src/tools/README.md` | Phase 2: `tools/*.mdx` | Defer |
| `channels-src/discord/README.md` | Phase 2: `channels/discord.mdx` | Defer |
| `CHANGELOG.md` | `reference/changelog.mdx` | Direct import, Mintlify format |
| `README.md` install section | `install/local.mdx`, `install/docker.mdx` | Expand with OS tabs |

**Content to write from scratch (Phase 1):**
- `index.mdx` — home page
- `start/getting-started.mdx` — quickstart
- `channels/web-gateway.mdx` — Web UI guide
- `channels/webhook.mdx` — webhook channel
- `channels/tui.mdx` — TUI guide
- `channels/signal.mdx` — stub
- `setup/configuration.mdx` — consolidated env var table
- `setup/database.mdx` — database backends guide
- `install/vps.mdx`, `install/docker.mdx`, `install/nearai-cloud.mdx`
- `security/index.mdx`, `security/secrets.mdx`, `security/sandbox.mdx`
- `help/troubleshooting.mdx`, `help/faq.mdx`
- `reference/cli.mdx`

---

## Phase 2 Outline (Deferred)

- `/skills/` — Trust model, SKILL.md format, ClawHub registry, writing skills
- `/tools/` — Built-in tools, WASM tools, MCP servers, building a WASM tool
- `/memory/` — Writing/reading memory, hybrid search, identity files, heartbeat
- `/routines/` — Cron, event-driven, webhook triggers
- `/guides/` — Use-case walkthroughs (Telegram in 10 min, connect Claude, VPS deployment, secure your setup)
- `/channels/discord.mdx`, `/channels/slack.mdx`, `/channels/whatsapp.mdx`
- `/reference/api.mdx` — Web Gateway REST + WebSocket API reference

---

## Documentation Plan

- `CONTRIBUTING.md` in `ironclaw-docs` repo: contribution guide, content freshness process, Mintlify component cheat sheet, code block style guide (Prism language identifiers)
- `verified_version` frontmatter convention: each page documents which IronClaw version it was verified against
- Content update workflow: when `ironclaw-src` adds/changes a feature, PR description must link to a docs PR

---

## Sources & References

### Origin
- **Brainstorm document:** [docs/brainstorms/2026-03-03-ironclaw-docs-website-brainstorm.md](../brainstorms/2026-03-03-ironclaw-docs-website-brainstorm.md)
  - Key decisions carried forward: Mintlify platform, separate `ironclaw-docs` repo, `docs.ironclaw.ai` domain, feature-first nav + Guides, security as primary differentiator

### Internal Content Sources
- `docs/LLM_PROVIDERS.md` — provider configuration guide
- `docs/TELEGRAM_SETUP.md` — Telegram setup walkthrough
- `src/setup/README.md` — authoritative 8-step wizard spec
- `src/workspace/README.md` — memory/workspace system spec
- `src/tools/README.md` — tool architecture spec
- `src/NETWORK_SECURITY.md` — network surface inventory and known findings
- `src/config/` — all environment variable definitions
- `CHANGELOG.md` — release history
- `README.md` — install methods, architecture diagram

### External References
- [Mintlify docs.json schema](https://mintlify.com/docs.json)
- [Mintlify component library](https://mintlify.com/docs/components)
- [OpenClaw docs](https://docs.openclaw.ai) — structural reference at `/home/opselite/ai_projects/openclaw/docs/`
- [Mintlify GitHub App](https://github.com/apps/mintlify)
- [Vercel Mintlify deployment guide](https://mintlify.com/docs/settings/deployment)
