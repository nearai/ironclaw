# OOBE & First-Run Onboarding — webui_v2

**Status:** design exploration; the automation concepts are prototyped in draft **PR #6994**
(UI-only, mock data). **Interactive mockup:** [`oobe/mockup.html`](./oobe/mockup.html) — open in a
browser, pick a **Version** (Vision / Foundational) and a **Scene** (First run / Thread / Plan),
and on Vision → First run press **Play** to watch the journey. Toggle **Flags** and the theme.

> File/symbol references are a point-in-time trace of `webui_v2`. Verify against live code
> before relying on them — prefer the codebase knowledge graph / `openwiki/`.

## Goal

Design the **first five minutes** for a new IronClaw user — the moment they land in the web chat
right after their account/workspace is set up. The automation surfaces already prototyped (the
"Done for you" carousel, the inline calendar-reschedule card, the Plan card, the agent-mode pill)
all assume automations *already exist*. This exploration fills the two first-run moments that gap
leaves open — the **cold start** (nothing yet) and the **first automation appearing** — and splits
the work into two tracks so the near-term build is decoupled from the north-star.

## Two versions (the `Version` switch)

The mockup carries **two design tracks** so we can ship an incremental, v2-faithful phase now while
keeping the aspirational target visible:

### Foundational — near-term, ships against current main
Scoped to what is feasible and lightweight on top of today's v2 system, for a **multi-tenant
enterprise** deployment:

- **Tools are admin-preconfigured** — there is no user-facing connect step. The first screen goes
  straight to suggestions.
- **Suggested task cards are present at the first step** — because tools are already connected, the
  agent surfaces its first suggestions immediately (no empty cold start to earn). Single-state first
  run; approve / modify / dismiss inline, or type your own.
- **No username** unless it can be derived from a deterministic source (admin preconfig, email/Slack
  profile). Default is a nameless greeting ("Welcome to IronClaw.") and a plain account chip.
- **Agent modes scoped to three**, default **Suggest**:
  - **Suggest** — always ask for approval before performing a task or automation.
  - **Plan** — describe the activity and the steps needed, then wait for approval.
  - **Auto Approve** — auto-approve task types the user has already approved, plus any task the user
    explicitly requests.
- Uses **main's composer** and the plain **pills-collapse** drawer (task cards condense to a pill row
  above the composer when the user starts typing). No bordered/attached drawer.

### Vision — north-star
Everything in Foundational plus the aspirational first-run experience:

- **Cold-start connect flow** (connect-your-tools panel → one-tap connect → anticipatory beat →
  first card reveal), named greeting, and the four-mode set (Suggest / Plan / Auto / Bypass).
- **First-automation reveal** — the emotional peak: a brief anticipatory beat (branded **NEAR
  process indicator** + skeleton tiles) then the first "Done for you" card is **conjured** in with a
  Gemini-style *ai-spark* border sweep. Honored under `prefers-reduced-motion`.
- **Attached task drawer** — the suggested-task cards sit in a bordered **drawer frame that docks
  onto the composer** and extends up from it, cards inset within the frame. The branded progress
  indicator + agent activity string sit **above** the drawer; the drawer header carries the
  subtitle (top-left) plus **collapse/expand** (cards ↔ pills) and **dismiss** (✕, replaced by a
  "Show suggestions" restore bar). Typing still collapses to pills.

Both versions share the redesigned card language and converge on the same populated carousel once
automations exist.

## Task drawer (reusable control)

The drawer is not just an OOBE device: it's a reusable surface for **suggested tasks whenever the
user returns or opens a new thread**. Collapsed, it's a compact scrollable pill row (brand logo +
title) docked to the composer; expanded (Vision), it's the full card frame. This gives returning
users a persistent, dismissible "here's what I'd pick up" affordance without commandeering the thread.

## Card design language (both versions)

- **Real brand product logos** (Gmail, Google Calendar, Docs, Drive, Slack in full colour; GitHub +
  Notion monochrome via `currentColor` so they follow the theme) instead of placeholder glyphs.
- **Title-first** header (icon + task title), no status tag; state reads from the action row.
- Muted **"From <app> · <time>"** provenance line; filled-primary + text-secondary buttons
  (Approve / Modify / Dismiss), with icons on the link buttons in every mode.
- Larger radius, soft shadow, condensed height; bottom-aligned action rows.

## Where this lives

SPA: `crates/ironclaw_webui/frontend/src` (React 19 + TypeScript + Tailwind v4; tokens in
`styles/app.css`). The landing view is `pages/chat/components/empty-state.tsx`; the automation
surfaces are `automation-carousel.tsx`, `automation-task-card.tsx`, `task-action-bar.tsx`,
`mode-selector.tsx`, and the mock data seam `lib/automation-tasks*.ts` + `hooks/useAutomationTasks.ts`
(all landed in PR #6994). The DEV-only harness is `pages/design-preview/design-preview-page.tsx`.

Today the carousel `return null`s when there are no tasks, so a fresh account sees the unchanged
hero + composer — i.e. the cold-start state is currently *undesigned*. That is what this mockup fills.

## What already ships vs. needs backend

- **Shipped (PR #6994, presentational, mock data):** the carousel + task card + action bar,
  calendar-reschedule card, plan card, agent-mode pill, and the endpoint-shaped data seam. The
  branded `NearProcessIndicator` (#6901) and the `AuthRequired` connector path are on main.
- **Needs backend (issue #6993):** "no automations yet" / "working on the first one" projection
  states; the reveal is driven by the `AutomationTaskAutomated` projection event; the
  anticipatory/skeleton state, the drawer's suggested-task feed for returning users, and the
  three-vs-four agent-mode gating are new UI over the §§2–7 wiring in `AUTOMATION-TASKS-CONTRACT.md`.

## Open questions

- **Phasing:** Foundational ships first against current main; which Vision pieces graduate next —
  the attached drawer, the connect flow, or the reveal animation?
- **Username derivation (Foundational):** which deterministic source wins when several are present
  (admin preconfig vs email vs Slack profile), and what's the fallback when none resolve?
- **Auto Approve scope:** how are "task types the user has already approved" defined and bounded
  (per-tool, per-action, spend/impact caps)?
- **Enterprise tool config:** how does the admin-preconfigured tool set surface to the user — a
  read-only "connected by your workspace" affordance, or invisible?
- Should the cold-start (Vision) seed one *real* low-risk automation to guarantee a first card, or
  wait for organic activity?
