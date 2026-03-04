# IronClaw Documentation Website — Brainstorm

**Date:** 2026-03-03
**Status:** Ready for planning

---

## What We're Building

A standalone documentation website for IronClaw — a secure, self-hosted AI assistant. The audience is **integrators and end-users** (not developers): people deploying, configuring, connecting channels, and using IronClaw day-to-day.

The docs will live in a **separate repository** (`ironclaw-docs` or similar), be built with **Mintlify** (same platform as OpenClaw docs), and be hosted on **Vercel** at `docs.ironclaw.ai`.

---

## Why This Approach

### Platform: Mintlify

OpenClaw uses Mintlify and the user wants "similar if not identical." Mintlify provides:
- Rich component library out of the box: `<Card>`, `<Steps>`, `<Tabs>`, `<Accordion>`, `<Callout>`
- Built-in full-text search
- Light/dark mode
- GitHub-based content workflow
- Static output that deploys to Vercel via `mintlify build`

### Structure: IronClaw-first + Use-Case Guides

Primary nav follows IronClaw's feature pillars (not forced-mapped from OpenClaw). A separate **Guides** section covers use-case-oriented walkthroughs for common setup scenarios.

### Repo: Separate

Clean separation between code and docs. Docs team/contributors can work independently. Own CI (Mintlify CLI build check on PR).

---

## Key Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Platform | Mintlify | Identical to OpenClaw; rich components; Vercel-compatible |
| Hosting | Vercel | User requirement; Mintlify builds static output |
| Domain | `docs.ironclaw.ai` | Placeholder; configure post-deploy |
| Repo | Inside `ironclaw-src/docs/` | Docs and code versioned together; simpler sync |
| Nav approach | Feature-first + Guides | Covers both power users and beginners |
| Brand assets | Fetch from ironclaw.com | Logo/colors TBD; placeholders in initial build |

---

## Proposed Navigation Structure

```
/ (Home)
  - What is IronClaw
  - Quick links: Install, Web UI, Channels, Security

/start/
  - Getting Started (quickstart: running in 5 minutes)
  - Onboarding Wizard (7-step setup)
  - What's Next

/install/
  - Overview
  - Local (Linux / macOS / Windows WSL2)
  - Docker
  - VPS / Cloud Server (generic Linux)
  - NEAR AI Cloud (managed)
  - Updating
  - Uninstalling

/setup/
  - Configuration Reference (all env vars)
  - Database Backends (PostgreSQL vs libSQL/Turso)
  - First-Run Wizard

/channels/
  - Overview
  - TUI (terminal interface)
  - Web Gateway (browser UI)
  - Telegram
  - Slack
  - HTTP Webhooks
  - Adding Custom Channels

/providers/
  - Overview
  - NEAR AI (default, OAuth)
  - Anthropic (Claude)
  - OpenAI (GPT)
  - Ollama (local inference)
  - OpenAI-Compatible (OpenRouter, Together, vLLM, LM Studio)
  - Tinfoil (private TEE inference)

/security/
  - Security Overview
  - Safety Layer (sanitizer, validator, policy, leak detection)
  - Secrets Management (encryption, zero-exposure model)
  - Docker Sandbox (isolation, network proxy, credential injection)
  - Prompt Injection Defense
  - Shell Environment Scrubbing
  - Allowlists & Policies

/skills/
  - Overview
  - Trust Model (Trusted vs Installed)
  - SKILL.md Format
  - Installing Skills
  - ClawHub Registry
  - Writing Your Own Skill

/tools/
  - Overview
  - Built-in Tools
  - WASM Tools (sandboxed)
  - MCP Servers
  - Building a Tool
  - Tool Auth & Credentials

/memory/
  - Overview
  - Writing & Reading Memory
  - Hybrid Search (FTS + semantic)
  - Identity Files (AGENTS.md, SOUL.md, USER.md)
  - Heartbeat System

/routines/
  - Overview
  - Cron (scheduled)
  - Event-driven (reactive)
  - Webhook Triggers

/guides/          ← Use-case oriented walkthroughs
  - Set Up Telegram in 10 Minutes
  - Connect IronClaw to Claude (Anthropic)
  - Run IronClaw on a VPS
  - Deploy to NEAR AI Cloud
  - Secure Your Deployment
  - Build Your First WASM Tool
  - Schedule a Daily Routine

/reference/
  - Environment Variables (complete table)
  - CLI Commands
  - API Endpoints (web gateway)
  - Database Schema Overview
  - Changelog / Release Notes
  - Credits & License

/help/
  - Troubleshooting
  - FAQ
  - Common Errors
  - Getting Support
```

---

## IronClaw vs OpenClaw — Key Differences to Highlight

| Topic | IronClaw | OpenClaw |
|---|---|---|
| Language | Rust binary | Node.js package |
| Install | `cargo build` or pre-built binary | `npm install -g openclaw` |
| Security | Safety layer, sandbox, secrets encryption, leak detection | Standard |
| LLM default | NEAR AI (session token OAuth) | Configurable |
| Sandbox | Docker-based with network proxy + credential injection | No |
| Memory | Hybrid FTS + vector search workspace | Session-based |
| Skills | SKILL.md trust model with ClawHub registry | Plugins |
| Channels | TUI, webhook, WASM, web gateway | WhatsApp, Telegram, Discord |
| Routines | Cron + event-driven | Basic scheduling |

IronClaw's **security architecture** is a primary differentiator and should be prominently featured — not buried in a reference section.

---

## Homepage Design (Mintlify)

Follows OpenClaw homepage pattern:

- Hero: tagline + 3 quick-start cards
- "What is IronClaw?" section with Mermaid flowchart
- Key capabilities grid (6 cards)
- Quick start steps
- "Start here" card grid linking to major sections

---

## Content Priorities (Phase 1 MVP)

For the initial launch, prioritize:

1. **Home** — hero, overview, quick links
2. **Getting Started** — zero to running in one page
3. **Install** — local + Docker (most common paths)
4. **Configuration** — env var reference
5. **Channels** — Telegram, Web Gateway, Webhook
6. **LLM Providers** — NEAR AI, Anthropic, OpenAI
7. **Security overview** — the differentiator
8. **Troubleshooting / FAQ**

Defer to Phase 2: full Skills docs, WASM tool building guide, advanced routines, all provider pages, Guides section.

---

## Brand & Design Notes

- Logo: fetch from ironclaw.com (placeholder until available)
- Color palette: TBD from ironclaw.com; use neutral dark theme as default
- Icons: Lucide (same as OpenClaw)
- Font: similar to OpenClaw (`DM Sans` headings, monospace body) unless brand specifies otherwise
- CNAME: `docs.ironclaw.ai` — configure in Vercel + Mintlify

---

## Open Questions

*None remaining — all resolved in brainstorm session.*

---

## Resolved Questions

| Question | Decision |
|---|---|
| Mintlify vs Docusaurus? | Mintlify — closest match to OpenClaw |
| Separate repo or in-repo? | In-repo (`ironclaw-src/docs/`) |
| Domain? | `docs.ironclaw.ai` (placeholder) |
| Brand assets? | Fetch from ironclaw.com; use placeholders initially |
| Target users? | All: local, VPS, NEAR AI cloud, integrators |
| Feature-first or use-case nav? | Both — feature-first primary nav + Guides section |
