# OOBE & First-Run Onboarding — webui_v2

**Status:** design exploration. The automation concepts were prototyped in **PR #6994** (UI-only,
mock data), since **rolled back** — the branch is code-free; the concepts now live in the mockup +
the [integration plan](./oobe/README.md). **Interactive mockup:** [`oobe/mockup.html`](./oobe/mockup.html) — open in a
browser, pick a **Version** (Vision / Foundational) and a **Scene** (First run / Thread / Plan),
and on Vision → First run press **Play** to watch the journey. Toggle **Flags** and the theme.

> **Integration proposal & plan:** this brief is the design "what/why." The plan to phase it into
> production — **Foundational** (near-term, extends shipped code) then **Vision** (north-star),
> with the shipped-vs-net-new scope, dependencies, and code-integration map — lives in the
> [`oobe/`](./oobe/) package: [README](./oobe/README.md) · [PROPOSAL](./oobe/PROPOSAL.md) ·
> [PLAN](./oobe/PLAN.md) · [CHECKLIST](./oobe/CHECKLIST.md).

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

- **Tools are admin-whitelisted, user-authorized** — the admin decides which tools are available,
  but the user still authorizes their own account per tool. So there is no separate connect *panel*;
  instead each suggested task card carries a **"Connect <Tool>"** CTA, and authorizing runs a modal
  OAuth "browser" dialog (sign in → approve scopes). Once connected, the card becomes an actionable
  suggestion.
- **Suggested task cards are present at the first step** — the agent surfaces its first suggestions
  immediately (no empty cold start to earn); each starts in a *connect* state and becomes
  approve/modify/dismiss once its tool is authorized. Cards read as proposals ("Triage your inbox")
  and flip to results ("Triaged your inbox") once run.
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

- **Cold-start connect flow** — a connect-your-tools panel where the user selects tools, then a
  queued modal OAuth "browser" dialog walks each one (sign in once, approve scopes per tool) until
  all are authorized; then the anticipatory beat → first card reveal. Named greeting, and the
  four-mode set (Suggest / Plan / Auto / Bypass).
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

SPA: `crates/product/ironclaw_webui/frontend/src` (React 19 + TypeScript + Tailwind v4; tokens in
`styles/app.css`). The landing view `pages/chat/components/empty-state.tsx` is on `main`; the automation
surfaces (`automation-carousel.tsx`, `automation-task-card.tsx`, `task-action-bar.tsx`,
`mode-selector.tsx`), the mock data seam (`lib/automation-tasks*.ts` + `hooks/useAutomationTasks.ts`), and
the DEV harness (`pages/design-preview/design-preview-page.tsx`) were prototyped in PR #6994 **but rolled
back** — they are not on `main`; their shape is captured in the mockup + the wiring contract.

Today the carousel `return null`s when there are no tasks, so a fresh account sees the unchanged
hero + composer — i.e. the cold-start state is currently *undesigned*. That is what this mockup fills.

## What already ships vs. needs backend

- **Prototyped then rolled back (PR #6994, presentational, mock data):** the carousel + task card + action bar,
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
