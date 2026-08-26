# OOBE Vision — reconciliation with the durable backend suggestions contract (PR #7694)

**Status:** governs. Where this document differs from [`PROPOSAL.md`](PROPOSAL.md),
[`PLAN.md`](PLAN.md), [`IMPLEMENTATION.md`](IMPLEMENTATION.md), or
[`AUTOMATION-TASKS-CONTRACT.md`](AUTOMATION-TASKS-CONTRACT.md), **this governs** — those
predate the backend contract landing.

**Scope change this records.** The Foundational (Phase 1) UX is **cut**. The program
retargets to the **Vision** UX, consuming the durable backend suggestions contract from
[PR #7694](https://github.com/nearai/ironclaw/pull/7694) (`feat: add durable backend
suggestions`, branch `backend-suggestions`, stacked on `native-structured-output`).

---

## 1. What #7694 is, and why it changes the phase decision

#7694 ships the **suggestion feed**: durable, agent-generated suggestion cards with a
full lifecycle — generation, listing, start, dismiss. It is backend-only and explicit
that *"no frontend files change"* and *"frontend consumption is explicitly out of scope."*
That is precisely the seam PR #6994 occupies, so the two are complementary: **#7694 is the
producer, #6994 becomes the consumer.**

It also settles the phase question. [`PROPOSAL.md` §5.2](PROPOSAL.md) offered three
producer designs and recommended shipping the *deterministic starter set* for Foundational
while treating the *agent-driven suggester* as "likely Vision-tier." #7694 built the
agent-driven one — a canonical unbounded run with a read-only capability allowlist
(memory search/read/tree, extension search, tool search/describe) finalized by a zero-tool
inference against a native structured-output schema.

> **The backend already shipped the Vision-tier producer.** Retargeting to Vision is not a
> scope expansion; it is alignment with what exists.

### 1.1 The contract (frozen in `ironclaw_product_contracts`)

| Method | WebUI route | Product operation |
|---|---|---|
| `GET` | `/api/webchat/v2/suggestions` | `suggestions.list` |
| `POST` | `/api/webchat/v2/suggestions/generate` | `suggestions.generate` |
| `POST` | `/api/webchat/v2/suggestions/{id}/start` | `suggestion.start` |
| `DELETE` | `/api/webchat/v2/suggestions/{id}` | `suggestion.dismiss` |

```
RebornSuggestionsResponse {
  status: "empty" | "generating" | "ready" | "failed",
  generation_id?: string,
  retry_after_seconds?: number,
  suggestions: RebornSuggestion[],
}

RebornSuggestion { id, title, description, suggested_prompt, thread_id?, run_id? }

RebornSuggestionStartResponse   { suggestion_id, thread_id, run_id }
RebornSuggestionDismissResponse { suggestion_id, dismissed }
```

Generation is asynchronous: `POST generate` (with a `client_action_id`) returns **202** and
`status: generating` plus a `retry_after_seconds` hint; the client polls `GET suggestions`.
One card set exists per `(tenant_id, user_id)`; a new generation **clears the previous set**.
Cards are bounded to **1–5** items (`schemas/suggestions.output.v1.json`), with
`title` ≤ 80, `description` ≤ 240, `suggested_prompt` ≤ 2000 characters.

The routes and the frontend suggestion surface are **always on**. The surface remains
lazy-loaded so its card and icon code does not add to the eager `/chat` bundle.

---

## 2. What Vision gains

| POR element | Before | With #7694 |
|---|---|---|
| Durable card store (N3), routes (N4), producer (N5) | Net-new, unbuilt | **Shipped** |
| **V3** — anticipatory / "working on the first one" states | "New projection states (#6993)" | **Real**: `empty` / `generating` + `retry_after_seconds` |
| **V2** — first-card reveal (ai-spark conjure) | Needed an `AutomationTaskAutomated` event | **Enabled**: trigger on the `generating → ready` transition |
| Approve → run | Prompt injection through `handleSend` | **Superseded**: `suggestion.start` returns `{thread_id, run_id}` |
| Live per-card status | Retired — no durable binding existed | **Un-retired**: cards carry durable `thread_id`/`run_id` |
| Dismiss | Local component state | **Durable** `DELETE` |
| **V4** docked drawer · **V5** named greeting | Frontend / open decision | Unaffected |

**The most consequential line is "live per-card status."** `IMPLEMENTATION.md` slice 2b was
retired on the finding that card-persistent status "requires a durable per-task record …
it's slice 6." #7694 *is* that record. A returning user's card can now show real state
because the suggestion→thread/run binding is durable and survives restart.

---

## 3. The conflict: the connect model has no backing

> **⚠ Superseded by the shipped contract.** #7694 landed on `main` (and is merged into
> this branch) carrying **`icon`** (a required semantic task enum) and **`sources`** (1–5
> human-readable provenance labels, for display). Cards therefore carry provenance, but
> they do not carry routable extension identity. The connect model **stays decoupled**
> (§3.1), cards stay startable regardless of connection, and `icon` drives only a neutral
> task glyph. The paragraph below describes the
> *original* branch state (before the merge) and is kept only to explain why the connect
> conflict existed. Current icon/enum details: [SUGGESTION-ICONS.md](SUGGESTION-ICONS.md).

The generated card **originally** carried no tool identity and no connect state:

```
{ id, title, description, suggested_prompt, thread_id?, run_id? }   // pre-#7694
```

That absence was deliberate, not an oversight. `prompts/suggestion_generation.md` instructs the
generator: *"Do not claim that an account, extension, credential, or capability is
available when you have not been given evidence that it is,"* and to prefer *"work the
assistant can carry out in a normal conversation."* The model can *see* extensions
(extension search is in the allowlist) but the output schema gives it nowhere to declare
one.

That contradicts the connect-centric spine of the POR:

| POR claim | Source | Status |
|---|---|---|
| "each suggested card carries a **Connect &lt;Tool&gt;** CTA" | `PROPOSAL.md` §2 | **Unsupported** — no tool to name |
| "each starts in a *connect* state" | `PROPOSAL.md` §2 | **Unsupported** — no such card state |
| **P3** per-card connect CTA + OAuth | `PROPOSAL.md` §3.1 | **Retired** as a card affordance |
| **V1** cold-start batched multi-tool OAuth panel | `PROPOSAL.md` §4 | **Re-homed** — see §3.1 |

### 3.1 Resolution — V1 decouples into its own surface

**Decision:** connect is no longer a card state. The Vision cold-start connect panel
survives as an **independent landing surface driven by the extensions catalog**, not by
suggestion cards. Connect and suggestions become two sibling surfaces:

```
Landing
├── Connect panel      ← extensions catalog (useExtensions), batched OAuth walk
└── Suggestion drawer  ← GET /suggestions, tool-agnostic cards
```

Consequences:

- **Cards never gate on connection.** A card is startable the moment it exists.
- **Just-in-time auth still works.** If a started run needs an unconnected tool, the agent
  emits the existing `AuthRequired` gate frame and the thread renders `AuthOauthCard` —
  a shipped path, unchanged. *(Assumption to verify in QA: `AuthRequired` fires for a
  suggestion-started run. It is the same agent loop, so it should — but it is untested.)*
- **The panel loses per-card "why this tool matters" framing.** Accepted trade-off; the
  panel explains value generically rather than per-suggestion.
- **`resolveConnectExtension` is orphaned.** Its job was card `app` id → catalog extension.
  A catalog-driven panel reads the catalog directly, so the resolver and its tests are
  removed. The *reuse pattern it proved* — lazy-load the real `ConfigureModal` rather than
  cloning `useOauthSetup`'s ~250-line popup/polling state machine — **carries forward to
  the V1 panel** and is the reason that panel is cheap to build.

---

## 4. Decisions that fall out of the contract

1. **Cards run in parallel — the single-active lock is removed. (Decided.)**
   `PROPOSAL.md` §2A.3 grounded "one active job at a time" in `submit_turn` returning
   `DeferredBusy`/`RejectedBusy` while a run is active **on a thread**. But
   `suggestion.start` creates a **separate thread per suggestion**, so there is no backend
   constraint to reflect — cards can genuinely run at the same time, which is what Vision's
   multi-card drawer wants. The lock (former slice 5) is gone from the code; approving one
   card never disables the others.
2. **"+ Automation" is removed. (Decided.)** The card schema has no `automation_prompt`
   field and #7694 adds no automation route, so there is nothing to build against; rather
   than carry a client-synthesized prompt-injection affordance with no durable backing, the
   action is dropped from the card entirely. (If recurring automations become a first-run
   goal later, they need their own backend contract — not a bolt-on here.)
3. **The events/projection contract is superseded.**
   [`AUTOMATION-TASKS-CONTRACT.md`](AUTOMATION-TASKS-CONTRACT.md) §§1–3 specify an
   `AutomationTask` domain model with five durable events and a projection. #7694 instead
   uses a **typed store over `ScopedFilesystem`** with bounded CAS and no SQL migration.
   Those sections are stale as an implementation target; they remain useful only as a
   record of the original design intent.

---

## 5. Refactor map for PR #6994

| Keep | Change | Delete |
|---|---|---|
| `SuggestedTaskCard` (presentational) | `DEMO_TASKS` → `GET /suggestions` + generate/poll | `resolveConnectExtension` + tests |
| `SuggestedTaskSurface` shell | Approve → `POST /{id}/start` → `onSelectThread(thread_id)` | `unconnected` card state |
| Always-on suggestion surface (routes are unflagged) | Dismiss → `DELETE /{id}` | `ConfigureModal` wiring *from the card* |
| Lazy-load + bundle discipline | `SuggestedTask` type → `RebornSuggestion` shape | `chat.oobe.connectUnavailable` (11 locales) |
| `NearProcessIndicator` render-prop | Card status derived from the bound `run_id` | `connectedIds` state |
| vm-harness test conventions | | `approvePrompt` / `automationPrompt` / `app` fields |

### 5.1 Slice plan

1. ✅ **Suggestions API client + types** — four typed calls over `lib/api.ts` conventions
   (`apiFetch`, `clientActionId()`), DTOs mirroring `RebornSuggestion`.
2. ✅ **Surface consumes real data** — `list` on mount; `empty` → generate CTA; `generating`
   → poll on `retry_after_seconds`; `ready` → render cards; `failed` → retry affordance.
3. ✅ **Start + dismiss** — Approve calls `start` and navigates to the returned `thread_id`;
   dismiss calls `DELETE`.
4. ✅ **Strip the connect model** — removed the `unconnected` state, resolver, and i18n key.
5. ✅ **V3 anticipatory states** — empty CTA / generating skeleton / failed retry; the
   generating state renders the branded NEAR indicator over static `.v2-skeleton` tiles.
6. ✅ **V2 reveal** — restrained card entrance via `.oobe-card-reveal` (reuses the sanctioned
   `v2-page-in` keyframe, `prefers-reduced-motion`-suppressed). *Note:* this is the
   governance-clean version, **not** the mockup's ad-hoc conic-gradient ai-spark border
   sweep, which would bypass the static-motion policy in `app.css`.
7. ⏳ **Live card status** (formerly the retired 2b) — subscribe to the bound `run_id` and
   reflect running/completed/failed on a returning user's card. *Not built.*
8. ⏳ **V1 connect panel** — catalog-driven cold-start "Connect your tools" surface (batched
   OAuth), reusing the `ConfigureModal` pattern. *Not built* (connect stays decoupled, §3.1).
9. ✅ **V4 docked drawer frame** — the surface renders as a bordered drawer with a
   "Suggested for you · approve to run, or tweak first" header, docked close to the composer.
   *Refinement not done:* true border-merge flush with the composer (kept as a rounded frame
   with a tight gap to avoid touching the shipped `ChatInput`).
10. ✅ **Refresh + connect entry** (issue #7815, F1/F2) — the drawer header carries a refresh
   control (re-runs `generate`; disabled while one is in flight) and a `/extensions` entry,
   and the empty/failed CTA rows pair generate/retry with the same connect entry. Refresh is
   honestly a refresh, not "more": the backend generation is replace-only until it gains an
   additive top-up transition. This is the route *entry* only — slice 8's batched-OAuth panel
   is still not built, and connect stays decoupled from the cards (§3.1).

Slices 1–6, 9, and 10 are **built** (1–6/9 on #6994). The remaining Vision follow-ups (not built), tracked
here for the design review:

- **Live card status (slice 7)** — needs the bound-run event subscription.
- **V1 connect panel (slice 8)** — the cold-start batched-OAuth surface.
- **Agent-mode selector** — Suggest / Plan / Auto / Bypass in the composer (net-new; a durable
  home + typed gate wiring, see PROPOSAL §7). Not part of the suggestions contract.
- **Pills-collapse on typing** — the drawer condensing to a scrollable pill row (+ dismiss /
  in-composer restore) when the user starts typing.
- **Named greeting + client username call-out in the header (V5)** — the mockup's
  "Welcome to IronClaw, &lt;name&gt;" hero and the account name in the top bar. Deferred: needs
  a deterministic username source (§10 open decision). *Explicitly tracked as a Vision
  follow-up per review.*
- **ai-spark reveal** — the mockup's richer first-card border-sweep, if an approved
  animation approach is adopted that doesn't bypass the motion policy (slice 6 shipped the
  restrained entrance instead).

### 5.2 Sequencing — resolved

#7694 (durable backend suggestions) and #7693 (native structured output) **landed on
`main`** and are merged into this branch, so the `/api/webchat/v2/suggestions` routes and
the `RebornSuggestion` contract are present here. The frontend was reconciled to the shipped
shape (field `sources` not `source_ids`; semantic `icon` required — see
[SUGGESTION-ICONS.md](SUGGESTION-ICONS.md)). Live preview QA is now possible end-to-end;
verify the deployed preview actually serves the routes before drawing a QA conclusion.

---

## 6. Open questions for review

*(Two earlier questions are now decided and moved to §4: cards run in parallel — the
single-active lock is removed; and "+ Automation" is removed.)*

1. **`AuthRequired` on a suggestion-started run** — verify in QA before relying on
   just-in-time auth as the connect story (§3.1).
2. **Agent modes (P4 / N6 / V6 Bypass)** — untouched by #7694; still net-new, still needs a
   durable home and typed gate wiring.
3. **Replacement UX** — a new generation clears the previous set. What does the drawer do
   if the user is mid-read when a replacement lands?
4. **Per-card connect.** The shipped `icon`/`sources` fields (§3,
   [SUGGESTION-ICONS.md](SUGGESTION-ICONS.md)) are presentation and provenance, not
   routable extension identity. Current decision keeps connect decoupled (§3.1), and auth
   stays a just-in-time in-thread prompt. A future per-card connect action must receive a
   typed extension identity from the backend or resolve through the extension catalog; it
   must not infer identity from `icon` or the human-readable `sources` strings.
