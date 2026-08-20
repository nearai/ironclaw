# IronClaw WebUI Design System — Storybook + Catalog (Executive Overview)

**Status:** Proposal, under review · **Authored against:** `origin/main` @ `d3791e0f8` · **Tracks:** Epics [#7038](https://github.com/nearai/ironclaw/issues/7038) (Phases 1–2) · [#7781](https://github.com/nearai/ironclaw/issues/7781) (Phase 3) · [#7782](https://github.com/nearai/ironclaw/issues/7782) (Phases 4–5)

**Documents:** [PROPOSAL.md](PROPOSAL.md) — the case, decisions & dependencies · [PLAN.md](PLAN.md) — phased execution · [CHECKLIST.md](CHECKLIST.md) — definition of done · [explorer.html](explorer.html) — self-contained interactive review aid (schematics + phase map; also published as a [claude.ai artifact](https://claude.ai/code/artifact/371a2622-054c-404a-8992-f110e1fa3d5a))

> **North star:** a *governed, catalogued* WebUI design system that carries IronClaw to an AI/agentic-first UX — realized **natively** on our React 19 + Tailwind v4 stack, reviewed and regression-tested through **Storybook**, and evolved in **five predefined phases**.

> **Epic ownership.** The original Epic #7038 was split into three so each wave has its own
> tracking issue. This package is shared by all three:
>
> | Epic | Phases | Scope |
> |---|---|---|
> | [#7038](https://github.com/nearai/ironclaw/issues/7038) | 1–2 | Storybook integration scaffolding · `DESIGN.md` governance & documentation |
> | [#7781](https://github.com/nearai/ironclaw/issues/7781) | 3 | Theme update & UI reskin — token values, dark palette, fonts, visual assets |
> | [#7782](https://github.com/nearai/ironclaw/issues/7782) | 4–5 | Agentic interactions & components · Information architecture |

## What this proposes

Formalize the design-system work already underway (Storybook integration + a catalogued primitive/component library) into a **benchmarked, phased program** to redefine the WebUI's theming, visual assets, interactions, and information architecture around an agent-first experience. Phases 1–2 are in flight — Phase 1 as PR [#7750](https://github.com/nearai/ironclaw/pull/7750) (recreated non-stacked off current `main`; supersedes the closed #7039) and Phase 2 as issue [#7042](https://github.com/nearai/ironclaw/issues/7042) (the closed #7043's changeset preserved, fresh PR to follow). This package is the north-star that frames every phase and the dependencies they carry.

It deliberately follows two internal benchmarks: the **APDD governance kit** (`../../../../apdd-kit` — docs-are-source-of-truth, a `DESIGN.md` constitution, Storybook-as-workbench/test/MCP, a design validation gate) and the **target-crate-architecture package** ([`docs/internal/reborn/target-architecture/`](../target-architecture/README.md), PR #6918 — README/PROPOSAL/PLAN/CHECKLIST + interactive explorer).

## Why this shape

1. **The work is already phased and partly shipped** — a north-star doc set makes the sequence, dependencies, and done-ness legible to reviewers instead of living only in an Epic checklist.
2. **Docs are the source of truth** (APDD Rule 1). `DESIGN.md` + the Storybook catalog govern how UI is built; this package records *why* and *in what order*.
3. **The riskiest work is ahead** (theme reskin, expressive motion, IA). Naming the dependencies now — and proposing how each is met — de-risks Phases 3–5.

## The layer map

Where the design system sits in the frontend, and what governs/catalogs it:

```mermaid
flowchart TD
  P["pages/ — feature views (chat, settings, admin…)"]
  L["layout/ — app shell (gateway-layout)"]
  C["components/ — shared composites (sidebar, command palette, page header…)"]
  DS["design-system/ — primitives (Button, Input, Modal, SelectMenu…) + composites (primitives.tsx)"]
  T["styles/app.css — @theme + --v2-* tokens · data-theme light/dark"]
  P --> L --> C --> DS --> T
  GOV["DESIGN.md + .claude/rules/design-system.md — governance"]
  SB["Storybook catalog + story tests — Design / Primitives / Composites / Components / Icons / Tokens"]
  GOV -. governs .-> DS
  GOV -. governs .-> T
  SB -. catalogs & regression-tests .-> DS
  SB -. catalogs & regression-tests .-> C
```

## The five phases

| Phase | Scope | Epic | Status | Ships as |
|---|---|---|---|---|
| **1** | Storybook integration + design-system catalog | [#7038](https://github.com/nearai/ironclaw/issues/7038) | In review | PR [#7750](https://github.com/nearai/ironclaw/pull/7750) (supersedes closed #7039) |
| **2** | `DESIGN.md` governance & guidelines | [#7038](https://github.com/nearai/ironclaw/issues/7038) | Ready, PR to follow #7750 | Issue [#7042](https://github.com/nearai/ironclaw/issues/7042) (old PR #7043 closed) |
| **3** | Theme update & UI reskin (tokens + assets) | [#7781](https://github.com/nearai/ironclaw/issues/7781) | Planned | — |
| **4** | Interaction & component updates (agentic-first) | [#7782](https://github.com/nearai/ironclaw/issues/7782) | Planned | — |
| **5** | Information architecture | [#7782](https://github.com/nearai/ironclaw/issues/7782) | Planned | — |

## Dependencies at a glance

The reskin/interaction phases carry hard prerequisites. Each has a proposed implementation in [PROPOSAL §7](PROPOSAL.md#7-dependencies-and-their-implementation-proposals):

- **Dark palette derivation** — the supplied M3 palette is light-only; the app is dark-default and dual-theme. *(gates Phase 3)*
- **WCAG AA contrast validation** for the high-chroma tokens. *(gates Phase 3)*
- **Fonts** — vendor Roboto Flex/Mono (OFL); drop/replace Google Sans (not freely redistributable). *(gates Phase 3)*
- **Animation approach** — a spring/motion mechanism gated on `prefers-reduced-motion`; none installed today. *(gates Phase 4)*
- **CI: Playwright/Chromium** for `pnpm test:storybook` if story tests run in CI. *(cross-cutting)*
- **MSW** for network-backed component stories' happy paths. *(cross-cutting)*

## How this aligns with the governance benchmarks

- **APDD kit** → this initiative *is* the kit's design-governance track: `DESIGN.md` (constitution + 5-tier taxonomy), path-scoped `.claude/rules/design-system.md`, Storybook as workbench + test-harness + agent-MCP, and a validation/REJECT gate. See [PROPOSAL §8](PROPOSAL.md#8-alignment-with-the-governance-benchmarks).
- **PR #6918** → this package copies its doc shape (README/PROPOSAL/PLAN/CHECKLIST + interactive artifact + a PR-body file table) and its conventions (provenance shas, phased waves with milestones, `⚠` ordering constraints, "Landed with #NNNN").

## What is explicitly *not* decided here

- Exact **token values**, the **animation library**, the **font substitute** for Google Sans, and the **IA/navigation** redesign — each is owned by its phase and resolved against `DESIGN.md` before it lands.
- Whether story tests become a **required CI gate** (proposed, not yet mandated — see PROPOSAL §7.1).

## How to review

1. Skim this README for the shape and the phase table.
2. Read [PROPOSAL.md](PROPOSAL.md) §1–§4 (decision + alternatives) and **§7 (dependencies)** — that's where the real risk lives.
3. Open [explorer.html](explorer.html) for the schematics and phase map — a self-contained page (no build step); render it in-browser via [html-preview](https://html-preview.github.io/?url=https://github.com/nearai/ironclaw/blob/docs/design-system-proposal/docs/internal/reborn/design-system/explorer.html) if you don't want to clone.
4. Challenge [CHECKLIST.md](CHECKLIST.md) (is this the right definition of done?) and argue [PLAN.md](PLAN.md) sequencing.
