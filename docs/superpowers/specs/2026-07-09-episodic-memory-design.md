# Episodic Memory for IronClaw (Sub-project 1 of 2)

**Date:** 2026-07-09
**Status:** Approved design, pre-implementation
**Scope:** Give IronClaw automatic cross-session continuity — it writes a durable summary of every conversation when it ends, and silently opens each new conversation already knowing the recent ones. This is Sub-project 1 of the "friend memory" feature; Sub-project 2 (a long-term, curated model of the user, distilled from this episodic record) is a **separate later spec** and out of scope here.

## Problem

IronClaw starts every conversation fresh. It has retrieval memory (workspace files + hybrid FTS+vector RRF search) and identity files in its prompt, but that is a searchable archive, not lived continuity: an ordinary-length conversation that never triggers context compaction leaves **no durable trace**, and even when summaries exist they are not surfaced into the next conversation. The goal is the felt experience of "it remembers where we left off," delivered by two automatic behaviors bracketing every conversation.

## Non-goals (Sub-project 2 or later)

- A curated long-term model of the user (preferences, ongoing projects, people, patterns). That is Sub-project 2, which *consumes* this episodic record.
- *Automatic* relevance-blended recall — proactively searching the archive at session start and merging topic-relevant past sessions into the recency window. (On-demand pull search over the session files IS available in Sub-project 1 via the existing `memory_*` tools; what Sub-project 2 adds is doing it *automatically* and blending it with the push window.)
- Any change to the existing `memory_*` tools, RRF search, or compaction beyond reusing them.

## Existing infrastructure this builds on

- **Workspace memory** (`src/workspace/`): markdown files with hybrid RRF search, versioning, `daily/` logs, curated `MEMORY.md`, and identity files. Philosophy: "memory is database, not RAM — write explicitly."
- **`Workspace::system_prompt()`**: the single place identity/memory files are assembled into the system prompt — the recall injection point.
- **Compaction summarizer** (`src/agent/compaction.rs::generate_summary`): an LLM distiller of conversation turns, currently triggered only by context pressure. Reused here.
- **Session lifecycle**: `HookEvent::SessionEnd` fires from the session-manager's idle-prune (`src/agent/session_manager.rs`), where the pruned session still holds its threads/turns in memory. `OnSessionStart` also exists.
- **Heartbeat** (`src/agent/heartbeat.rs`): the periodic background executor — the backstop scheduler.
- **Durability guarantee**: LLM data is never deleted; conversation turns are always in the DB, so nothing is lost if a summary write is missed.

## Architecture

Two automatic behaviors, both riding the seams above:

1. **Auto-write** on conversation end → a durable, structured session summary in the workspace.
2. **Auto-recall** on conversation start → the recent summaries silently in the system prompt (adaptive: IronClaw *has* them and surfaces them only when relevant — no forced greeting).

### Component 1 — Session summarizer

A shared summarizer, extracted/generalized from compaction's `generate_summary`, that turns a conversation's turns into a compact **structured summary**:

- `title` (short), `gist` (1–3 sentences), `decisions` (list), `open_threads` (list of unfinished items / next steps), `user_notes` (notable user context or tone).
- Plus metadata: `timestamp` (conversation end time), `conversation_id`, `channel`.

Uses the lightweight summarization LLM path (`cheap_llm`, which falls back to the main model — on this box that resolves to the local `qwen3.6:27b`). Summarization is a background task; latency is not user-facing.

### Component 2 — Episodic store (workspace markdown)

Reuses the whole workspace stack (RRF search, versioning, `memory_*` tools):

- **Per-session file:** `workspace/memory/sessions/YYYY-MM-DD-<conv-short>.md`, with YAML frontmatter (`timestamp`, `conversation_id`, `channel`, `title`) and the **full** structured summary (gist, decisions, open threads, user notes) as the body. Individually **searchable** via the existing hybrid RRF, so any past session — however old — is recall-able by content. This is the unbounded half of the recall model (see "Recall model" below).
- **`workspace/memory/recent.md`:** a compact newest-first **digest** — NOT full summaries. Each entry is terse: **title + one-line gist + open threads only** (~60–120 tokens); the full detail lives in the per-session file. This is the always-injected continuity surface. Prepended on each write and bounded by BOTH a count (`N`, default 5) AND a total size cap (~1.5k tokens), whichever binds first — because it rides the system prompt on every turn. Entries with unresolved open threads are weighted to stay longer than fully-wrapped ones (open loops are the highest-value continuity signal).

Granularity is **per conversation/thread** (`conversation_id`), so "our last conversation" maps to a single real episode.

### Recall model — two complementary channels

Recall is **push + pull**, and the `N` cap bounds only the push channel, not what is recallable overall:

| Channel | Surface | Bound | Trigger |
|---|---|---|---|
| **Continuity (push)** | `recent.md` injected into `system_prompt()` | small (N≈5, ~1.5k tokens) | automatic, every conversation |
| **Relevant recall (pull)** | hybrid RRF search over the per-session files (via the existing `memory_*` tools) | **unbounded** | when the topic calls for it |

The push channel is what delivers "silently already knows where we left off" — at the start of a chat there is no query to search on, so recent continuity must be *pushed* into the prompt. The pull channel gives infinite, relevance-based recall of any past conversation, but only fires when IronClaw decides to search — which is why it cannot, on its own, provide proactive continuity. Sub-project 2 later adds *automatic* relevance-blended recall (searching the archive at session start and merging it with the recency window) plus the long-term user model; Sub-project 1 delivers the recency push + the searchable archive that makes on-demand pull possible.

### Component 3 — `SessionMemory` (the coordinator)

A component holding the summarizer LLM handle, the workspace, and the store. Two entry points:

- `summarize_and_store(conversation_id, turns, channel)` — distill + write the per-session file + update `recent.md`. Skips trivial conversations (no substantive user turns). Idempotent: no-ops if a file for `conversation_id` already exists.
- Invoked (a) from the **session-end path** (idle-prune, where the in-memory session still holds the turns), and (b) from the **backstop sweep**.

### Component 4 — Auto-write wiring (session end)

At idle-prune in `session_manager`, before/at the `SessionEnd` fire, hand each stale conversation's turns to `SessionMemory::summarize_and_store`. (`session_manager` gains the `SessionMemory` handle as an optional dependency; when absent — e.g. in tests — auto-write is simply skipped.)

### Component 5 — Backstop sweep (durability)

On **startup** and on the **heartbeat**, enumerate conversations that ended recently (via the conversations table's `updated_at`) with **no** per-session file, and summarize them from the durable DB turns. Idempotent (skips any `conversation_id` already summarized). Covers a crash between conversation end and idle-prune.

### Component 6 — Auto-recall (session start, the push channel)

`Workspace::system_prompt()` includes `recent.md` as a clearly-labeled "Recent conversations" section (only if present and non-empty). Every new conversation thus opens with the terse last-N digest silently in context — the push channel. Adaptive: IronClaw *has* it and surfaces it only when relevant. The full detail of any recent (or old) session is one search away via the pull channel — the per-session files are always searchable — so nothing that rolls off `recent.md` is lost.

## Data flow

```
conversation ends (idle-prune / close)
  → SessionMemory.summarize_and_store(conv_id, turns, channel)
      → summarizer LLM → structured summary
      → write workspace/memory/sessions/<date>-<conv>.md
      → prepend to workspace/memory/recent.md (prune to N)

startup + heartbeat
  → for each recently-ended conversation with no summary file:
      → load turns from DB → summarize_and_store  (idempotent)

new conversation starts
  → Workspace::system_prompt() includes recent.md
  → IronClaw silently knows the last N conversations; surfaces when relevant
```

## Failure handling — memory never blocks a conversation

- Auto-write failure (LLM or file error) → logged, **non-fatal**; raw turns stay in the DB; the backstop retries next sweep.
- Trivial/empty conversations (no substantive user turns) → skipped; no summary noise.
- `recent.md` missing/unreadable at recall → the prompt omits that section, degrading to today's behavior. Never hangs or errors a turn.
- Idempotency everywhere keyed on `conversation_id` so session-end and the backstop can't double-write.
- **Token budget:** `recent.md` capped at N summaries, each length-bounded, so recall cannot bloat the prompt.
- Multi-tenant: all paths scope by `user_id` (the workspace is already per-owner).

## Testing

1. **Summarizer (unit, stub LLM):** turns → structured summary with the expected fields populated.
2. **Episodic store (unit, temp workspace):** per-session file written with correct frontmatter and full body; `recent.md` prepended newest-first with **terse** entries (title + gist + open threads only, not the full body), bounded by N **and** the size cap (whichever binds first), with open-thread entries retained preferentially over wrapped ones.
3. **Recall (unit):** `system_prompt()` includes `recent.md` content when present; omits the section gracefully when absent.
4. **Idempotency / skip (unit):** `summarize_and_store` no-ops on a duplicate `conversation_id`; skips a trivial conversation.
5. **Backstop (integration):** summarizes an un-summarized recently-ended conversation from DB turns; a second sweep is a no-op.
6. **End-to-end:** hold a real conversation → let it end → start a new one → IronClaw naturally references the prior when relevant.

## Deliverable

Every IronClaw conversation leaves a durable, searchable, timestamped episode, and every new conversation opens already holding the recent ones — the "picks up where we left off" foundation, and the raw material Sub-project 2 will distill into a long-term model of the user.
