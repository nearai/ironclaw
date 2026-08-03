# OOBE & First-Run Onboarding — webui_v2

**Status:** design exploration; the automation concepts are prototyped in draft **PR #6994**
(UI-only, mock data). This round explores the two first-run *moments* that prototype skips.
**Interactive mockup:** [`oobe/mockup.html`](./oobe/mockup.html) — open in a browser and press
**Play** to watch the first-run journey; toggle the cold-start **direction** (Invite / Coach),
**Flags**, and the theme.

> File/symbol references are a point-in-time trace of `webui_v2`. Verify against live code
> before relying on them — prefer the codebase knowledge graph / `openwiki/`.

## Goal

Design the **first five minutes** for a brand-new IronClaw user — the moment they land in the
web chat right after activating their account. The automation surfaces already prototyped
(the "Done for you" carousel, the inline calendar-reschedule card, the Plan card, the agent-mode
pill) all assume automations *already exist*. A real first-timer has **zero** completed
automations, no history, and hasn't connected any tools. This exploration covers the two moments
that gap leaves open:

1. **Cold-start first run** — what a user sees when there is genuinely nothing yet, and how that
   screen earns the first connect + first prompt without feeling empty or overwhelming.
2. **First automation appears** — the aha transition from an empty landing to the first
   "Done for you" card materializing, and how that reveal is paced and framed.

## Where this lives

SPA: `crates/ironclaw_webui/frontend/src` (React 19 + TypeScript + Tailwind v4; tokens in
`styles/app.css`). The landing view is `pages/chat/components/empty-state.tsx`; the automation
surfaces are `automation-carousel.tsx`, `automation-task-card.tsx`, `task-action-bar.tsx`,
`mode-selector.tsx`, and the mock data seam `lib/automation-tasks*.ts` + `hooks/useAutomationTasks.ts`
(all landed in PR #6994). The DEV-only harness is `pages/design-preview/design-preview-page.tsx`.

Today the carousel `return null`s when there are no tasks, so a fresh account sees the unchanged
hero + composer — i.e. the cold-start state is currently *undesigned*. That is exactly what this
mockup fills in.

## The two directions (from the mockup)

The cold-start screen is a real fork; the mockup lets you toggle between them:

- **Invite** — minimal. Hero + composer + a single quiet "connect your tools and I'll start
  handling the busywork" line with a compact connect row (Gmail / Calendar / Slack). Nothing
  pretends to be data. Fastest to a first prompt; risks under-selling what the agent will do.
- **Coach** — anticipatory. The carousel slot shows three **ghost/preview cards** ("here's the
  kind of thing I'll handle") behind a soft "Connect to turn these on" veil, teaching the
  automation concept *before* anything is real. Sells the value; risks looking like fake data if
  the ghost treatment isn't unmistakably not-yet-real.

Both converge on the same populated carousel once automations exist.

## First-automation reveal (from the mockup)

The journey plays: cold-start → connect Gmail/Calendar → a brief **anticipatory** state
(NEAR process indicator: "Getting to know your inbox…", skeleton tiles) → the **first card
reveals** into a newly-appearing "Done for you" strip → the strip fills with the remaining cards.
The reveal is the emotional peak of the OOBE; it should be a paced beat, honored under
`prefers-reduced-motion`.

## What already ships vs. needs backend

- **Shipped (PR #6994, presentational, mock data):** the carousel + task card + action bar,
  calendar-reschedule card, plan card, agent-mode pill, and the endpoint-shaped data seam.
- **Needs backend (issue #6993):** the empty/anticipatory states above assume the projection can
  report "no automations yet" and "working on the first one". The reveal is driven by the
  `AutomationTaskAutomated` projection event; the skeleton/anticipatory state and connect-then-work
  handoff are new UI over the §§2–7 wiring in `AUTOMATION-TASKS-CONTRACT.md`. Connect uses the
  existing `AuthRequired` connector card path.

## Open questions

- Invite vs Coach — or a hybrid that starts Invite and promotes to Coach after the first connect?
- Should the cold-start seed one *real* low-risk automation (e.g. a calendar-accept) to guarantee
  a first card, or wait for organic activity?
- Where does "connect your tools" live long-term — inline on the landing (as mocked), or a
  dedicated onboarding step upstream (the activation wizard lives in a separate product)?
