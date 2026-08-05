# Agentic Activity & Streaming UX — webui_v2

**Status:** design approved; implementation in progress (this is the foundation PR).
**Interactive mockup:** [`mockup.html`](./agent-activity-streaming/mockup.html) — open in a
browser to see the full behavior (press **Play**; toggle **Compare / Proposed · focus**,
**Flags**, and the theme).

> File/symbol references are a point-in-time trace of `webui_v2`. Verify against live code
> before relying on them — prefer the codebase knowledge graph / `openwiki/`.

## Goal

Bring the chat's agent-activity + streaming experience up to (and past) Claude Desktop
**Cowork** and **OpenWorker**: replace the raw `Activity — N tools` + JSON disclosures with
**narrated activity**, a **branded live process indicator**, a persistent **Plan / Context**
rail, and IronClaw-native surfaces (safety-approval gate, connector cards, background-job,
multi-channel handoff) — all on the real `--v2-*` tokens.

## Where this lives

SPA: `crates/ironclaw_webui/frontend/src` (React 19 + TypeScript + Tailwind v4; tokens in
`styles/app.css`). Backend edge: `crates/ironclaw_webui/src/webui_v2/{schema.rs,handlers.rs}`.
Producer/mapping: `crates/ironclaw_composition/src/projection/live_progress.rs`.

Chat components (`pages/chat/`): `chat.tsx`, `components/message-list.tsx`,
`message-bubble.tsx` (incl. `ThinkingDisclosure`), `tool-activity.tsx`, `activity-run.tsx`,
`typing-indicator.tsx`, `plan-card.tsx`, `approval-card.tsx`, `auth-*-card.tsx`,
`onboarding-pairing-card.tsx`, `chat-input.tsx`.

## Design decisions (from the mockup)

- **Process indicator** — the NEAR "N" mark (`provider-logos.tsx` canonical path) rendered
  filled, with a glowing brand-blue (`#0091FD`) light that **chases through the interior along
  the glyph's spine**, plus a quick spin + heartbeat pulse. It settles to a **solid `#0091FD`**
  mark when the turn completes. It sits pinned at the **bottom** of the thread (Claude-Desktop
  style) as the live status ("Thinking… 0:12" → "Worked for 12s").
- **Activity trail** — thinking narration + tool activities stream as a trail. A **"N steps ⌄"
  toggle** sits at the **top** (below the prompt) and collapses/expands the trail; it's collapsed
  by default once the turn is done. Grouped runs ("Explored your setup", "Pulled …") collapse to
  their summary once the agent moves past them; **Connect/Approve cards stay expanded**.
- **Composer** — input over an action row: agent-**mode selector** (Discuss / Ask for approval /
  Full access + "Send approvals to Inbox"), mic, attach, and a **Run ⌘↵** button that is
  disabled until there's input.
- **Cards** — connector, approval, and artifact cards share one neutral card family (no
  warning-yellow on approval). The final response surfaces the produced **artifact as a compact
  reference card**.
- **Rail** — a floating (borderless) Plan / Context panel that appears only once the run has
  telemetry; its column collapses while hidden.
- **Theme + motion** — both light and dark off `--v2-*`; honor the near-static motion policy
  (only whitelisted keyframes animate; all suppressed under `prefers-reduced-motion`).

## What already ships in v2 (recompose, no backend change)

Thinking narration (`ProductProjectionItem::Thinking`); grouped tool run with expandable params
(`WebChatV2Event::CapabilityActivity` + `activity-run.tsx`); the **safety-approval gate**
(`WebChatV2Event::Gate` + `approval-card.tsx` — a surface Cowork has no equivalent for); the
**connector / auth card** (`WebChatV2Event::AuthRequired`); text streaming / final
(`ModelTextDelta` → `Text` / `final_reply`); run status; skill activation; and a durable
completed-run duration derived from the run-scoped user/final transcript timestamps.

## Needs a new payload (follow-up Rust work + tests)

1. **Per-tool duration** — the UI renders `{ms}` but `toolCardFromActivity/Preview` hardcode
   `null`; add `started_at`/`completed_at` (or `duration_ms`) to `CapabilityActivityView` and the
   `CapabilityCompleted` milestone.
2. **Live Plan / todo tracker** — no plan event exists (`plan-card.tsx` is fed by a client-side
   mock); add a `PlanUpdate { steps: [{id,title,status}] }` projection item + producing milestone.
3. **Live elapsed timer** — the completed duration can use durable transcript timestamps, but
   a precise ticking live timer still needs a run/step start timestamp on the stream.
4. **Background-job on the turn stream** — jobs are REST-polled (`ProcessStatus` Running→Completed);
   bridge job status into the projection for the inline job chip.
5. **Multi-channel handoff** — new UI + action (no event today).

Each of the above follows `.claude/rules/gateway-events.md` (durable event → projection → stream
→ frontend reconcile) with the required tests.

## Rollout

- **This activity cohort (foundation):** the design reference above + the `NearProcessIndicator` component
  (the branded live indicator), wired into `TypingIndicator` and retained as a static
  “Worked for Ns” line after completion. This cohort is presentation-only, with no backend change.
- **Next:** activity-trail consolidation (top toggle + bottom status), composer mode selector,
  card-family unification + artifact card, floating rail.
- **Then:** the five new SSE payloads, one focused PR each, each with tests through the caller.
