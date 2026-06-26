# Reborn Memory — Host-Managed Lifecycle (mem0 flow) — v1 Design

- **Date:** 2026-06-25
- **Base:** `reborn/memory-lift-followups` (PR #5205) @ `ad84d34c1`
- **Branch:** `reborn/memory-lifecycle`
- **Status:** approved-in-conversation; build against it.

## Goal

Implement the mem0 host-managed memory flow on top of #5205, with the entire PR
surface area confined to **(A) the native memory implementation** and **(B)
run-level orchestration**. Make memory actually reach the model — today the loop
hardcodes `memory_snippets: Vec::new()` (`loop_support/lib.rs:404`) and never
calls the (working) native retrieval.

## The flow (mem0 shape → IronClaw)

```
on_run_start (once per run):
  long_term  = memory.search(query=latest_user_message, scope={tenant,user})
  short_term = memory.search(query=latest_user_message, scope={tenant,user}, filter={thread_id})
before_model_call:
  prompt = system + long_term + short_term + conversation
after_each_turn (= our run end):
  memory.add(exchange=[user,assistant], user_id, thread_id, agent_id, run_id, provenance, ttl)
on_run_end (optional):
  optional thread summary; TTL / evict-from-surfacing only — never hard-delete
```

**Terminology mapping (decided):** mem0 "run" = our **thread** (conversation);
mem0 "turn" = our **run** (one user→assistant exchange). The short-term lane is
**`thread_id`-scoped** (chosen over per-run `run_id`): short-term = "this
conversation," accumulating across the user's messages in the thread.

## Surfaces

### A. Native memory (`ironclaw_memory` / `ironclaw_memory_native`)

- **`long_term` retrieval — already implemented**, just never called.
  `retrieve_context` (`memory_native/service.rs:335`) and `search` (`:95`) do
  real FTS via `MemorySearchRequest`. Wire, don't build.
- **`short_term` — new:** tag writes with `thread_id` (on `DocumentMetadata`) and
  accept a `thread_id` **filter** on `search`/`retrieve_context`. Stays inside
  the `(tenant,user[,agent,project])` scope isolation (fail-closed); the thread is
  a filter *within* the user's own memory, never a cross-user key.
- **host `add` — new:** the first host-initiated write (today all writes are
  agent-tool-only). Verbatim / host-curated (no LLM extraction in v1), stamped
  with **provenance + a default TTL**, tagged `user/thread/agent/run` so it feeds
  both lanes on the next run.

### B. Run-level orchestration (`ironclaw_reborn/src/turn_run_executor.rs`)

- **`on_run_start`** (`:162-171`; `run_id`/`thread_id` already on
  `LoopRunContext`, `turns/run_profile/host.rs:551`): fetch both lanes **once**
  and stash for the run — replacing the current per-model-step re-fetch. Invalidate
  on mid-run input/query change (`canonical.rs:65-86` / `:234-264`).
- **inject:** the existing `"memory"` prompt section
  (`instruction_bundle.rs:338`); reuse the host admission (512 B/snippet, 4 KiB
  total, untrusted envelope).
- **`after each turn`** (`apply_exit`, `:252`): host `add` of `[user, assistant]`.
- **`on_run_end`:** optional thread summary; **no hard-delete**.

Nothing touches the lower capability contract (`host_runtime/lib.rs:323-329`
origin-exclusion respected — run/origin coordination stays in the upper run
executor, never threaded into `MemoryService`/`RuntimeCapabilityRequest`).

## Resolved decisions (handoff open questions)

| # | Question | Decision |
|---|---|---|
| Q1 | run_id model | Reuse `LoopRunContext.{run_id,thread_id}`; **`thread_id`** for short-term. No new id. |
| Q2 | layering | Native impl + run-level orchestration; **not** the lower capability contract. |
| Q3 | what `add` records | **Host passes the data; the provider decides** (Ben, 2026-06-26). A low-level `MemoryService::record_interaction(messages, run_id, metadata)` — mem0 `add` shape; `user_id`/`agent_id`/`thread_id` ride the invocation scope. Native stores the full turn history under `threads/<thread_id>/`; a mem0 provider could run extraction (`infer=true`). No host-side verbatim-vs-extract decision. Default no-op trait impl → providers opt in. |
| Q4 | provenance/TTL | **Provider concern, not host.** The host passes `metadata`; provenance / TTL / extraction are each provider's choice. For native self-scoped thread scratch, none are needed in v1. (This is also why the heavy Trap-4 machinery doesn't bind here — the data is the user's own exchange in their own thread.) |
| Q5 | delete scratch vs "never delete LLM data" | **TTL / evict-from-surfacing only; archive, never hard-delete.** |
| Q6 | per-run cache + invalidation | Fetch once per run; invalidate on latest-user-message / input-cursor change. |

## Phased TDD plan (red → green per step)

- **Phase 1 — read path at the run level + thread filter**
  1. Native: `thread_id` tag on write + `thread_id` filter on search.
     *Red:* a thread-filtered search returns only thread-tagged docs; cross-user
     scope isolation still holds.
  2. Run-level: fetch `long_term`+`short_term` once at run start and inject.
     *Red (caller-level):* memory reaches the model; retrieval fires once per run,
     not per iteration.
- **Phase 2 — after-turn `add`**
  - Host-driven add at run end, provenance + TTL, dual-tagged.
    *Red (caller-level, through the run):* after a run, an add persists and is
    retrievable next run in **both** lanes; a forced add error does not fail the turn.
- **Phase 3 — `on_run_end`**
  - Optional thread summary; TTL respected on retrieval.
    *Red:* an expired item is not surfaced but is **not** deleted.

## Constraints (non-negotiable)

No lower capability-contract change · no LLM in the retrieval/surfacing path ·
scope isolation fail-closed · no hard-delete (TTL/evict only) · `debug!` not
`info!`/`warn!` in background paths · dual-backend parity for any new persistence ·
caller-level tests for the run hooks (`.claude/rules/testing.md`) · prompt
templates in `prompts/*.md`.

## Progress log

- **2026-06-25 · Phase 1 step 1 — native short-term thread-scoping — DONE (TDD red→green, fmt+clippy clean).**
  Key finding: `ResourceScope` *already* carries `thread_id: Option<ThreadId>`
  (`host_api/resource.rs:59`) and `MemoryInvocation` already holds a
  `ResourceScope`, so `thread_id` already flows into the native provider — **zero
  contract-crate change.** Implemented as one conditional retain in
  `NativeMemoryService::retrieve_context` (`memory_native/service.rs`): when
  `invocation.scope.thread_id` is `Some(T)`, restrict results to the
  `threads/<T>/` path prefix (`thread_memory_prefix` helper); when `None`, the
  long-term lane is unchanged. Test:
  `native_context_retrieve_scopes_short_term_to_active_thread` in
  `tests/memory_service_facade.rs` (13/13 pass). The run level fetches twice — long
  term with `ResourceScope::without_thread_and_mission()`, short term with the
  thread kept.
- **2026-06-26 · Phase 1 step 2 — run-level fetch + inject — DONE (subagent-built, TDD, diffs reviewed by me).**
  `host_runtime/memory_context.rs` `load_memory_snippets` now fetches BOTH lanes
  once (short-term thread-kept + long-term thread-cleared via
  `ResourceScope::without_thread_and_mission()`), concatenates short-term-first,
  admits over the combined 4 KiB block, per-lane degrade-to-empty. `loop_support/lib.rs`
  `ThreadBackedLoopContextPort` gains an `Arc<OnceCell<Vec<LoopContextSnippet>>>`
  per-run cache + `with_memory_context_service`; `load_loop_context` fetches once
  per run (query = latest user message) and surfaces into the `"memory"` section,
  degrading to empty on any failure. Threaded composition → `loop_driver_host`
  factory → port, mirroring `user_profile_source`. Tests: host two-lane (3) +
  rendering (1) + caller-level once-per-run cache (2).
  **Caveats / follow-ups (tracked for the PR):**
  - Memory resolves on the **local-dev runtime path only**; the production graph
    wires `None` (deferred — issue #5013, same as `user_profile_source`).
  - **Coverage gap:** composition→host→port wiring is compile-verified + port-tested
    with a fake, but no e2e yet proves the *real* service reaches the model
    (`RebornBinaryE2EHarness`). Close before/with the PR — test-through-the-caller.
  - **Tuning:** combined budget is short-term-first, capped at `max_snippets`; a
    scratch-heavy thread can starve the long-term lane (per-lane sub-budgets later).
  - Local `cargo test` shows 3 pre-existing `sandbox_process` failures = no Docker
    (`/var/run/docker.sock`), unrelated to memory; CI runs them with Docker.
- **2026-06-26 · Phase 2 — after-turn `record_interaction` — IN PROGRESS (subagent).**
  Reframed per Ben: a low-level `MemoryService::record_interaction(invocation, { messages, run_id, metadata })`
  (mem0 `add` data shape; default no-op impl so providers opt in). Native override
  stores the full turn history under `threads/<thread_id>/log.md` (same convention
  its short-term read lane filters on). Host hook = `AfterTurnMemoryRecorder` (new
  file in `ironclaw_reborn`), fired at `apply_exit` when `state.status == Completed`:
  reads the exchange from the thread transcript with the **owner-rewritten** scope
  (`ThreadScopeResolver::resolve_for_turn`), passes it down; failure-isolated
  (never fails the completed run). Wired via `DefaultPlannedRuntimeParts.after_turn_memory_writer`
  (raw `Arc<dyn MemoryService>`) + composition resolve. This closes the write half:
  the after-turn record feeds the short-term lane the Phase-1 read surfaces.
- **Next:** the full add→surface **e2e** (run 1 records → run 2's short-term lane
  surfaces it in the model request), then the full gate, then the PR + audit +
  CodeRabbit loop above. Phase 3 (`on_run_end` durable summary) optional / follow-up.

## Ship & review plan (post-implementation — per Ben, 2026-06-25)

When the implementation is finished and the full gate is green
(`cargo fmt` · `cargo clippy --all --benches --tests --examples --all-features`
zero warnings · `cargo test`):

1. Push the branch + open a PR whose **base is `reborn/memory-lift-followups`
   (#5205)**.
2. **In parallel:** (a) dispatch an agent to adversarially audit the diff;
   (b) wait for CodeRabbit to post its full review on the PR.
3. Triage **every** finding (audit + CodeRabbit): fix (TDD where behavioral) /
   resolve / respond to each.
4. Push the fixes.
5. Request a **full CodeRabbit re-review**.

Open logistics to resolve at PR time:
- **Push remote:** `origin` (nearai) vs the `benkurrek` fork — reborn memory
  branches have historically lived on the fork; #5205's head branch is the base,
  so the PR head must be pushed somewhere GitHub can open the PR from.
- **"Finished" scope:** the core flow = Phase 1 (fetch both lanes at run start +
  inject) + Phase 2 (after-turn `add`). Phase 3 (`on_run_end` summary/cleanup) is
  optional per Ben's "that's IT" — include if cheap, else flag as follow-up.
