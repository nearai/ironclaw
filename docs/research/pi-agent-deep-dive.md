# Pi Agent Harness — Deep Dive

Research notes on **pi** (Mario Zechner's `pi-mono`, `@earendil-works/pi-coding-agent`),
the minimal coding agent that multiple 2025–2026 same-model harness benchmarks rank
best or near-best on cost and token utilization at equal task quality. Source checked
out at `/data/illia/research/pi-mono` (github.com/badlogic/pi-mono); all `file:line`
references below are into that tree and were spot-verified against source on 2026-08-01.

The goal of this document: understand *mechanically* how pi's agent loop, tool system,
and context management work, and extract why it is cheap — as input to IronClaw's own
agent-loop and context design.

---

## 1. Architecture map

```
packages/ai            unified multi-provider LLM SDK (10 wire APIs, ~35 providers)
packages/agent         the agent runtime: agent-loop.ts (pure loop) → agent.ts
                       (stateful Agent) → harness/ (session-backed AgentHarness, "Pi 2.0")
packages/coding-agent  the shipping CLI: drives a raw Agent (not AgentHarness) and
                       re-implements sessions/compaction/retry in core/agent-session.ts
packages/tui           terminal UI with differential rendering
packages/{client,server,protocol,storage}  RPC/remoting layers
```

Layering discipline is strict: the loop is a pure function over `(messages, config,
streamFn)`; the `Agent` adds state and queues; the harness/coding-agent add durability,
compaction, and retry **outside** the loop. Errors are values everywhere — the stream
function must never throw (`packages/agent/src/types.ts:24-27`), tool failures become
`toolResult` messages with `isError`, and run-level throws are converted into a synthetic
failure `AssistantMessage` with a complete replayed event sequence (`agent.ts:496`,
`harness/agent-harness.ts:609`).

Deliberate omissions (`packages/coding-agent/docs/usage.md:301`): **no built-in MCP, no
sub-agents, no permission popups, no plan mode, no todo lists, no background bash.**
No permission sandbox either — containerization is delegated to Docker/microVMs.
These omissions are load-bearing for the cost numbers in §6.

## 2. The agent loop (`packages/agent/src/agent-loop.ts`)

### 2.1 Shape

Two nested `while` loops, no step cap, no state machine (`runLoop`, :155-275):

```ts
let pendingMessages = (await config.getSteeringMessages?.()) || []; // pre-poll
while (true) {                       // outer: drains follow-up messages
  let hasMoreToolCalls = true;
  while (hasMoreToolCalls || pendingMessages.length > 0) {  // inner: turns
    // drain steering messages into context
    // one LLM call (streamAssistantResponse)
    // hard-exit on stopReason error|aborted
    // execute tool calls (or fail them all if stopReason === "length")
    // hasMoreToolCalls = !batch.terminate
    // prepareNextTurn?  → may swap context/model/reasoning between turns
    // shouldStopAfterTurn? → early exit
    // pendingMessages = getSteeringMessages()
  }
  // getFollowUpMessages() → re-enter inner loop, else break
}
```

Termination is purely: text-only assistant response with empty queues, or
`stopReason ∈ {error, aborted}`, or a `shouldStopAfterTurn` callback, or every tool
result in a batch setting `terminate: true` (`.every()`, not `.some()` — :582-584).

Observable protocol is a flat event union (`types.ts:422-437`): `agent_start/end`,
`turn_start/end`, `message_start/update/end`, `tool_execution_start/update/end`.
A "turn" = one assistant response + its tool batch.

### 2.2 Message model — declaration-merged app messages

The loop carries `AgentMessage = Message | CustomAgentMessages[...]` (`types.ts:319`).
Apps extend `CustomAgentMessages` via TypeScript declaration merging (harness adds
`bashExecution`, `compactionSummary`, `branchSummary`, `custom`). Projection to the
three LLM roles (`user | assistant | toolResult`) happens **once per turn** at the LLM
boundary via `config.convertToLlm` — e.g. a user's `!` bash execution becomes a
synthetic user message with fenced output, and `!!` executions carry
`excludeFromContext` and are dropped entirely. One projection point means the durable
transcript can be richer than what the model sees.

During streaming, the partial assistant message physically lives in `context.messages`
and is overwritten in place by each delta and finally by the complete message
(:321, :337, :350) — no separate streaming buffer to reconcile.

### 2.3 Tool execution — parallel with sequential preflight

`executeToolCalls` (:411-426): default mode is parallel, but **one tool marked
`executionMode: "sequential"` forces the entire batch sequential**.

The parallel path (:489-554) is three-phase, not `Promise.all(map(...))`:

1. **Preflight is sequential in source order**: emit `tool_execution_start`, validate
   args (TypeBox), run the `beforeToolCall` hook — so approval/permission hooks always
   run one at a time, in order, before anything executes.
2. Prepared calls become thunks; `Promise.all` runs them concurrently.
3. `tool_execution_end` events fire in **completion order** (live UI), but `toolResult`
   messages are appended in **assistant source order** (deterministic transcript).

A per-file mutation queue (`harness/tools/file-mutation-queue.ts`) serializes concurrent
edit/write on the same *canonical* path (symlinks resolved), so parallel edits can't race.

Every failure mode — unknown tool, validation error, hook block, tool throw — becomes a
normal `isError` tool result; nothing escapes the batch as an exception.

### 2.4 Truncation-poisoned tool calls

If the assistant message ends with `stopReason === "length"`, pi does **not** execute
any tool calls in it (:208-213). Streamed tool arguments are finalized by a
salvage-JSON parser and may *validate while silently truncated*; every call gets a
synthesized error result telling the model to re-issue. Naive loops execute these and
corrupt files.

### 2.5 Steering, follow-up, interruption

Three distinct queues with different drain points:

| Queue | Drained | Semantics |
|---|---|---|
| steering | before loop + after every turn | injected before the next LLM call; current tool batch still completes |
| follow-up | only when the inner loop would exit | wakes a finished agent for another run |
| nextTurn | with the next user prompt | never mid-run |

Default queue mode is `one-at-a-time`. `Agent.prompt()` while running is a hard error
pointing to `steer()`/`followUp()` — concurrency is never silently queued.
`skipInitialSteeringPoll` is a one-shot latch preventing double-drain when
`continue()` itself consumed the queue.

### 2.6 Retry lives outside the loop

The loop never retries. Three independent layers:

1. **HTTP layer** (`packages/ai/src/utils/provider-retry.ts`): mirrors the official SDK
   policy (408/409/429/5xx, `retry-after`, exp backoff capped at 8s with jitter). A
   server-requested delay above 60s is converted into an error and bubbles up.
2. **`retryAssistantCall`** (`packages/ai/src/utils/retry.ts`): used only for
   compaction/branch-summary calls. Classification is two regexes over the error
   string: account/billing limits fail fast; overload/429/5xx/transport strings retry.
3. **Turn-level driver** (`core/agent-session.ts:1058-1099`): `prompt()` then
   `while (handlePostAgentRun()) continue()`. On a retryable error it **pops the error
   assistant message out of the LLM context** (kept in the session file for history),
   sleeps with abortable exponential backoff, and calls `agent.continue()`. Context
   overflow is routed to compaction instead of retry, with a
   one-shot `_overflowRecoveryAttempted` guard (compact → retry once → give up
   with an explanatory message).

Keeping retry outside makes the loop pure/re-entrant and lets the driver surgically
rewrite the transcript before resuming.

## 3. Tool system (`packages/coding-agent/src/core/tools/`)

### 3.1 The tools

Seven built-ins — `read, bash, edit, write, grep, find, ls` — but the **default set
exposed to the model is only four**: `read, bash, edit, write`
(`core/system-prompt.ts:81`). grep/find/ls exist mainly for read-only bundles; by
default the model composes `rg`/`fd` through bash. Tool schemas are TypeBox, which *is*
JSON Schema — passed to providers verbatim with zero generation step
(`packages/ai/src/api/openai-completions.ts:1318`).

Key schema/prompt choices:

- **read**: `path`, `offset` (1-indexed line), `limit`. Returns raw lines with **no
  line-number gutter** (cheaper tokens than Claude Code's `cat -n` format). Supports
  images (resized to ≤2000×2000/4.5MB, with a coordinate-scale note).
- **bash**: `command`, optional `timeout` in seconds, **no default timeout**. Output
  tail-truncated; overflow spills to a temp file whose path is given to the model.
  Process-tree SIGKILL on abort. Pluggable `BashOperations` backend (SSH etc.).
- **edit**: `edits[]` of `{oldText, newText}` — multiple targeted replacements in one
  call, each matched against the *original* file (not incrementally). Prompt
  guidelines: "Keep edits[].oldText as small as possible while still being unique…
  do not pad with large unchanged regions."
- **write**: whole-file create/overwrite; guideline restricts it to new files or full
  rewrites.

### 3.2 Truncation as prompt engineering

Central limits (`core/tools/truncate.ts:11-13`): **2000 lines / 50KB** (first hit
wins), grep lines clipped at 500 chars. Head-truncate for reads, tail-truncate for
bash. The important part is that **every truncation notice names the exact
continuation action**:

- `[Showing lines A-B of TOTAL. Use offset=B+1 to continue.]`
- `[100 matches limit reached. Use limit=200 for more, or refine pattern. …]`
- `[Showing lines A-B of TOTAL. Full output: /tmp/pi-bash-<hex>.log]`
- `[Line N is 87.3KB, exceeds 50.0KB limit. Use bash: sed -n 'Np' <path> | head -c 51200]`

So recovery from truncation is one tool call, not a search. Success payloads are
minimal (`Successfully replaced N block(s) in <path>.`); the full diff lives in a
`details` field that goes to the UI/session only, **never into LLM context**.

### 3.3 Edit matching

`core/tools/edit-diff.ts:304-366` — exact-first, then one bounded fuzzy pass:

1. `indexOf` exact match.
2. On miss, normalize *both* sides: NFKC, per-line `trimEnd()`, smart quotes → ASCII,
   Unicode dashes → `-`, NBSP → space. Leading indentation is **never** normalized.
3. Fuzzy replacements are overlaid back so untouched lines keep exact original bytes.
4. Uniqueness is a hard error (`Found N occurrences… provide more context`); **no
   replace-all exists**. Overlapping edits rejected with "merge them into one edit".
5. Edits applied in reverse offset order; BOM and CRLF/LF preserved.
6. **No-op detection**: producing identical content is an error, not a silent success —
   prevents a wasted round-trip on a hallucinated match.

`prepareArguments` is a model-compat shim (parses `edits` sent as a JSON string,
folds legacy top-level `oldText/newText`) that runs *before* schema validation.

### 3.4 Extensibility without MCP

No MCP, by design. Instead: extensions are plain TypeScript files
(`~/.pi/agent/extensions/` or `-e file.ts`) with `pi.registerTool(def)` working **at
runtime in the same session** — new tools become callable without reload. Extensions
can intercept/mutate/block every tool call (`tool_call` / `tool_result` middleware).
The documented "dynamic tool loading" pattern registers hundreds of tools inactive
behind one active `search_tools` loader that calls `setActiveTools()`; the provider
layer then uses Anthropic `defer_loading` + `tool_reference` blocks so the **cached
tool prefix stays byte-identical** while tools appear mid-conversation. A tool only
occupies system-prompt space if it opts in with a one-line `promptSnippet`; docs
explicitly warn that activating a tool with prompt metadata rebuilds the system prompt
and invalidates the prefix cache.

Tool results support text + images plus a UI-only `details` channel, an optional
`usage` (billed to the session), `addedToolNames` (records which tools existed from
that transcript point, for replay), and `terminate`.

## 4. Context management

### 4.1 Sessions: a JSONL tree

Sessions are JSONL, one entry per line, forming a **tree**, not a linear log
(`packages/coding-agent/docs/session-format.md`). Entries carry
`{id, parentId, timestamp}`; branching (`/tree`) just moves the leaf pointer;
`/fork`/`/clone` create new files referencing `parentSession`. Context rebuild walks
leaf→root; if a compaction entry is on the path, the summarized prefix is dropped and
`[compaction summary] + kept-verbatim tail` is emitted. Nothing touches disk until the
first *assistant* message arrives, so abandoned prompts don't litter the session dir.
Model/thinking-level changes are themselves entries, replayed to derive current state.

### 4.2 Compaction

Triggers (`core/agent-session.ts:1953-2042`):

- **Threshold**: `contextTokens > contextWindow − reserveTokens` (defaults
  `reserveTokens: 16384`, `keepRecentTokens: 20000` —
  `harness/compaction/compaction.ts:176-177,265`). Also checked before each new prompt.
- **Overflow**: ~24 provider-specific error regexes plus two silent-overflow
  heuristics (`packages/ai/src/utils/overflow.ts`); pops the error message, compacts,
  retries **once**.
- **Manual** `/compact [instructions]`.

Mechanics: walk backwards accumulating a `chars/4` token estimate until
`keepRecentTokens`, then snap to a valid cut point — user/assistant/bash-execution
boundaries, **never a toolResult** (tool call + result stay together). Everything
before the cut is summarized; everything after is kept **byte-for-byte verbatim**. If
a single turn exceeds the budget, it is split with a separate turn-prefix summary.

The summarization prompt forces a structured checkpoint:

```
## Goal / ## Constraints & Preferences / ## Progress (Done|In Progress|Blocked)
## Key Decisions / ## Next Steps / ## Critical Context
"Preserve exact file paths, function names, and error messages."
```

Two efficiency details stand out:

- **Iterative updates**: when a previous summary exists, a distinct
  `UPDATE_SUMMARIZATION_PROMPT` preserves it and folds in only the delta ("move items
  from In Progress to Done…") — the Nth compaction doesn't re-summarize history.
- **File-op state survives compaction**: tool calls are scanned for read/write/edit
  paths, accumulated **across** compactions (seeded from the previous compaction
  entry), and appended as literal `<read-files>…</read-files>` /
  `<modified-files>…</modified-files>` tags, so the model knows what it has already
  seen without re-reading.

Cost controls on the summarization call itself: tool results are clipped to 2000 chars
*inside the summarizer's input only* (live context keeps them intact); `maxTokens` for
the summary is capped at `0.8 × reserveTokens`; and the request runs with
`cacheRetention: "none"` and a **fresh throwaway sessionId**, so a one-off summary
neither writes a useless cache entry nor disturbs session routing affinity.

### 4.3 Prompt caching

Anthropic `cache_control` breakpoints are placed at exactly three stable boundaries
(`packages/ai/src/api/anthropic-messages.ts`):

1. the system prompt block(s) (:977-999),
2. the **last tool definition only** (:1320, gated by compat since some
   Anthropic-compatible hosts reject it),
3. the last content block of the last user message (:1256-1281).

TTL: default ephemeral 5-minute; `PI_CACHE_RETENTION=long` → `ttl: "1h"`, with the 2×
write premium correctly modeled in cost math (`models.ts:650-656`). Anthropic-style
markers are also emitted over OpenAI-compatible endpoints when
`compat.cacheControlFormat === "anthropic"` (e.g. OpenRouter + `anthropic/*`); OpenAI
Responses uses `prompt_cache_key: sessionId` + 24h retention.

What makes the cache actually hit:

- The system prompt is built once per session/config change and contains **no
  timestamp, date, or git state** — only tool list, guidelines, doc paths, project
  context files, skills, cwd.
- The tools array (part of the cached prefix) stays byte-identical thanks to deferred
  tool references (§3.4).
- A stable session UUID flows into `x-session-affinity`/`session_id` headers for
  replica-affinity routing.
- **Cache-waste accounting** (`core/cache-stats.ts`): pi computes, per turn, how many
  prompt tokens *should* have been cache reads but were re-billed
  (`missed = min(prevPrompt, prompt) − cacheRead`), prices the delta, and can surface
  warnings like "Cache miss after model switch: 45.2k tokens re-billed (~$0.34)".
  The harness measures its own cache discipline.

### 4.4 Accounting and pruning

Context usage is **hybrid**: real provider `usage` from the last assistant response
plus `chars/4` estimates only for messages after it — no token-counting API calls.
The footer shows `↑in ↓out Rcache-read Wcache-write CH<hit%> $cost pct%/window`, and
session totals include compaction/summary calls so summarization work is billed
visibly. If a compaction just happened and no response has arrived, usage shows `?`
rather than a stale pre-compaction number.

**Old tool results are never pruned or truncated in live context** — they're kept
byte-for-byte until a compaction cut drops them wholesale. All truncation happens at
capture time (§3.2). This maximizes prefix-cache hits: rewriting history would
invalidate the cache every turn.

### 4.5 System prompt

`core/system-prompt.ts:28-162`. Base prompt is **~1.5KB**: a two-sentence identity,
one line per tool (opt-in via `promptSnippet`), a handful of deduplicated guideline
bullets, a pi-docs routing table ("read only when the user asks about pi itself"),
then `<project_context>` (AGENTS.md/CLAUDE.md walked from `/` down to cwd, first
match per directory), a skills block (one `<skill>` name/description/location line
each — bodies loaded lazily), and `Current working directory: <cwd>`. Compare:
Claude Code's fixed per-request prompt overhead is ~20× larger (§6).

## 5. The AI layer (`packages/ai`)

- Three concepts: **API** (10 wire protocols, each a module exporting
  `stream`/`streamSimple`), **Provider** (~35, owning auth + model list), **Model**
  (catalog entry pinning api + provider + pricing + compat flags). Cross-provider
  quirks are declarative per-model `compat` structs, not code branches.
- Unified event stream: `start`, `text_/thinking_/toolcall_ start|delta|end`,
  `done|error`, every event carrying the growing `partial` message; errors encoded
  in-stream, never thrown.
- **Usage/cost**: `Usage {input, output, cacheRead, cacheWrite, cacheWrite1h?,
  reasoning?, cost{...}}` computed per request by one shared `calculateCost()` with
  tiered pricing and the Anthropic 1h-write surcharge. Pricing data is hydrated at
  build time primarily from `models.dev/api.json` plus live provider catalogs and
  hand-maintained overrides.
- **Anthropic specifics**: adaptive thinking (`effort` low→max) for Opus 4.7+/Fable-class
  models vs `budget_tokens` for older ones; interleaved-thinking beta on by default;
  thinking signatures round-tripped; hand-rolled SSE decoder enforcing event pairing.
- **OAuth (Claude Pro/Max)**: full PKCE flow; requests with an OAuth token impersonate
  Claude Code — Bearer auth with the `claude-code` beta, claude-cli user-agent, a
  mandatory "You are Claude Code…" system block prepended, and tools renamed to Claude
  Code's canonical casing on the way out and back. This lets subscription users avoid
  raw API billing entirely (a large share of pi's real-world cost advantage for
  individuals). Token refresh is serialized so concurrent requests can't double-refresh.

## 6. Why pi wins on cost — benchmarks and mechanics

### 6.1 External evidence (same model, different harness)

- **nqawhc, "Your agent harness is an efficiency decision, not a quality decision"
  (Jul 2026)** — 4 harnesses × identical DeepSeek V4 Flash, 8 bug-fix tasks: pi
  2.1 min / 14.8k output tokens / 4 tools / **1.3k tokens fixed overhead** vs Claude
  Code 8.0 min / 58.4k tokens / 27 tools / **23.1k overhead** — quality statistically
  indistinguishable. 3–4× cost for the same result.
- **Databricks (Jul 2026)** — same model + thinking effort across harnesses on tasks
  from real internal PRs: >2× cost-per-task spread at equal quality; **pi sent ~3×
  less context per turn** than Claude Code/Codex and finished in fewer turns.
- **openbench (Jul 2026)** — correctness saturates across frontier harnesses; token
  spread up to ~8×, wall-clock ~4×; "pi is repeatedly the fastest/leanest harness."
- **Portkey "Harness Tax" (Apr 2026)** — per-request fixed overhead ~2.6k tokens (pi)
  vs ~15k (Codex) vs ~27k (Claude Code).
- Canonical design post: Mario Zechner, *"What I learned building an opinionated and
  minimal coding agent"* (Nov 2025), with a Terminal-Bench 2.0 run.

Consistent caveat: pi is **cheaper at equal quality**, not higher quality —
correctness saturates across good harnesses; the harness choice is an efficiency
decision.

### 6.2 The mechanics behind the numbers

1. **Tiny fixed tax.** Sub-2KB system prompt + 4 tool schemas ≈ ~1.3k tokens on
   *every request*, vs 20k+ for heavy harnesses. This multiplies across every turn of
   every task.
2. **Fewer, cheaper turns.** Less context per turn → faster model reads → fewer
   confused detours; Databricks measured both fewer turns and less context per turn.
3. **No MCP schema rent.** MCP servers cost 13–18k tokens of schemas up front;
   pi's answer is bash + runtime-registered extension tools + deferred tool references.
4. **Cache-first design.** Timestamp-free stable system prompt, byte-identical tool
   prefix (deferred tools), append-only history (no retroactive pruning), stable
   session affinity headers, throwaway session ids for summaries, and a built-in
   cache-waste meter. Every design choice preserves the prefix.
5. **Truncation with continuation handles.** Capture-time limits (2000 lines/50KB)
   with exact `offset=`/`limit=`/temp-file continuations — large outputs never enter
   context, and recovery is one call.
6. **Compaction that preserves momentum.** Verbatim 20k-token tail, structured
   iterative summaries, `<read-files>/<modified-files>` carryover — the model rarely
   re-reads what it already saw.
7. **Token-shaped tool design.** Multi-edit calls with minimal `oldText`, no
   line-number gutter on reads, one-line success messages, diffs kept UI-side,
   no-op edits rejected instead of silently burning a round trip.

## 7. Comparison with IronClaw's Reborn loop — and what to adopt

Based on a matching deep-read of `crates/ironclaw_agent_loop`, `ironclaw_loop_host`,
`ironclaw_turn_runner`, `ironclaw_llm`, and `ironclaw_composition` (all `file:line`
refs below into the IronClaw tree, verified 2026-08-01).

### 7.1 Side-by-side

| Dimension | pi | IronClaw Reborn |
|---|---|---|
| Loop shape | Two nested while-loops, no cap, policy via callbacks | Fixed ordered stage pipeline (`executor/canonical.rs:18-604`), sealed strategy objects, typed `Step` enums |
| Step cap | None | 1,024-iteration backstop with a model-visible terminal warning first (`strategies/budget.rs:35`) |
| Termination | Text-only response + empty queues | Rich stop heuristics: reply-only turn, no-progress ×3, repeated-call signature, diminishing returns, rejected replies (`strategies/stop.rs:307-385`) |
| LLM retry | Outside the loop; error message popped from LLM context, kept in history | In-stage, per-error-class budgets up to 45 attempts (`strategies/recovery.rs:58,333-450`); error surfaces as an *ephemeral* inline message, never a durable message |
| Context overflow | Compact + retry exactly once | ShrinkContext retry ×2 + one observation-assisted attempt, forced compaction (`recovery.rs:376-382`) |
| Tool execution | Parallel with sequential preflight; source-order persistence; per-file mutation queue | **Sequential always** — `Parallel` verdict only controls park behavior; port iterates one at a time (`ironclaw_loop_host/src/capability_port.rs:1886-1910`) |
| `length`-stopped tool calls | Never executed; per-call synthesized error results tell the model to re-issue | Never executed; whole response becomes `OutputTruncated`, one observation-assisted continuation then abort (`model_gateway.rs:1683-1688,1816`) |
| Steering | Three queues with explicit drain points | Steering + follow-up drains at two points (`canonical.rs:77-99,277-314`); `allow_steering`/`allow_interrupt` policy flags exist but are never consulted (`strategies/drain.rs:33-41`) |
| Persistence | JSONL tree, buffered until first assistant message | Write-per-event + full-state checkpoints (`BeforeModel`/`BeforeSideEffect`/`Final`), resumable; projection layer (`is_model_context_visible`) keeps durable transcript strictly richer than model context |
| Compaction trigger | Estimate > window − 16k reserve, from real usage + tail estimates | Estimate ≥ 108k **hardcoded** (128k − 20k), pure chars/4, not derived from the actual model window (`ironclaw_loop_contracts/src/context_budget.rs:15-34`) |
| Compaction content | Structured checkpoint summary, iterative delta updates, `<read-files>` carryover, verbatim 20k tail | Structured summary (fresh + update prompts in `ironclaw_loop_host/prompts/`), 8k verbatim tail, injection-scanned and leak-redacted; no file-op carryover |
| Prompt caching | 3 explicit `cache_control` breakpoints; prefix stability engineered everywhere | Single top-level ephemeral marker relying on automatic caching (`ironclaw_llm/src/rig_adapter.rs:1086-1095`); **OAuth path has no caching at all** (`anthropic_oauth.rs:485-490`) |
| Prefix stability | Timestamp-free system prompt, byte-identical tool array (deferred refs), append-only history | Unstable: inline nudges prepended *before* identity, minute-precision timestamp in runtime context, per-run memory retrieval, mid-run tool promotion, sliding-window eviction, provider-switch history rewrite |
| Cache telemetry | Per-turn cache-waste meter with $ attribution | Better: break detection attributed to `tool_definitions_changed`/`system_prompt_changed` (`ironclaw_turn_runner/src/model_gateway/prompt_cache_activity.rs`) — which recorded the 82% → 29% hit-rate collapse |
| Tool output limits | 2000 lines/50KB, continuation-naming messages | Comparable and already good: 2000 lines/64KiB reads, 16KiB shell preview + disk spill, `result_read` continuation over reference envelopes |
| Fixed per-request tax | ~1.3k tokens (prompt + 4 tool schemas) | ~15–25k tokens: 24 core tools / ~12k schema tokens + base prompt + up to 8k identity + skills/memory — and unreliably cached |
| Cost accounting | Per-request cost from shared rate table; session totals include summarization | Cumulative usage in loop state, USD reserve/settle via `ResourceGovernor`, priced at product edge — richer, but budget reservation estimates tokens from the content *reference string*, not content (`ironclaw_loop_contracts/src/model_work.rs:38-45`) |

### 7.2 Where IronClaw is already ahead

Adopting from pi must not regress these — pi simply doesn't have them, because it's a
single-user trusted CLI and IronClaw is a multi-channel product with untrusted input:

- **Durability**: full-state checkpoints with replay-deduplicable recovery and runner
  re-drive; pi's loop dies with its process.
- **Projection layer**: `ToolResultReference` + `result_read` means huge tool results
  never fully enter context at all — stronger than pi's capture-time truncation.
- **Security**: compaction summaries are injection-scanned and leak-redacted; capability
  dispatch goes through authorization/approval gates. Pi has none of this by design.
- **Budget governance**: pre-call USD reservation with approval blocking.
- **Cache-break attribution**: IronClaw's `prompt_cache_activity` diagnostics are more
  precise than pi's cache-waste meter — IronClaw measured its problem; it hasn't fixed it.

### 7.3 What to adopt, in priority order

**P0 — cache-prefix stability program.** This is where the 82% → 29% collapse lives,
and every pi mechanism is directly transplantable. Filed as #6984 (breakpoints),
#6985 (prefix mutation), #6986 (tool array), #6987 (regression test):

1. **Explicit `cache_control` breakpoints** on system prompt, last tool definition,
   and last user block — replacing the single top-level ephemeral hint in
   `rig_adapter.rs`, and adding caching to the OAuth transport (which currently sends
   none). Pi's placement (`anthropic-messages.ts:977-999,1256-1281,1320`) is the model.
2. **Stop mutating the prefix.** Move inline loop-control nudges from *before* identity
   (`instruction_bundle.rs:245-254`) to the transcript tail — IronClaw already has the
   right vehicle (`LoopInlineMessage` ephemeral injection). Move the minute-precision
   timestamp out of the system block into a tail message. Pin per-run memory retrieval
   for the duration of a run.
3. **Byte-identical tool array.** Tool promotion mid-run (`tool_disclosure.rs`
   `PromotedSet`) rewrites the advertised set and breaks the prefix; adopt Anthropic
   `defer_loading` + `tool_reference` blocks the way pi does, so deferred tools surface
   in transcript content instead of the cached tools array.
4. **Assert stability in tests**: a regression test that two consecutive prompt bundles
   in one run produce byte-identical system + tools prefixes (IronClaw already computes
   the cache signatures to assert on — `prompt_cache_activity.rs:215+`).

**P1 — correctness of the accounting that drives compaction.** Filed as #6988
(window-derived budget), #6989 (hybrid accounting + reservation bug), #6990
(no-cache summaries):

5. **Derive the context budget from the model's actual window** instead of the
   hardcoded 128k (`context_budget.rs:15-34`) — a 200k/1M-window model currently
   compacts at 108k, paying summarization cost for nothing.
6. **Hybrid token accounting**: last provider-reported usage + chars/4 for the tail
   only (pi's `estimateContextTokens`), replacing pure chars/4 — and fix the
   reference-string length bug in `ModelWorkRequest::for_assistant`.
7. **Summaries must not pollute cache/affinity**: run compaction inference with
   caching off and a throwaway request identity (pi: `cacheRetention:"none"` + fresh
   uuid), if the compaction task doesn't already.

**P2 — loop mechanics:**

8. **Real parallel tool execution** with pi's three-phase shape: sequential preflight
   (validation + approval gates in source order — fits IronClaw's authorization model),
   concurrent execution, results persisted in source order, plus a canonical-path
   mutation queue for file-writing tools. Today `Parallel` is a misnomer.
9. **Per-call error results for `length`-stopped tool calls**: instead of collapsing
   to `OutputTruncated` with one recovery lane, synthesize a model-visible error result
   per discarded call instructing re-issue — cheaper and keeps the loop moving.
10. **File-op carryover across compaction**: accumulate read/modified paths across
    compactions and emit `<read-files>/<modified-files>` in the summary, so the model
    doesn't re-read after a compact. IronClaw already has fresh/update summarizer
    prompts; this is an additive detail.

**P3 — fixed-tax reduction (biggest long-term $ lever, needs product judgment):**

11. **Shrink the always-on surface.** Pi ships 4 tools ≈ 1.3k tokens; IronClaw ships
    24 core tools ≈ 12k schema tokens + 15–25k total prefix. Candidates: fold more
    core tools behind the existing `tool_search`/`tool_describe` disclosure bridges
    (made cache-safe by item 3), trim schemas/descriptions, and audit which of the 24
    earn their rent per turn. The external benchmarks (§6) say this fixed tax — paid
    every request, multiplied by cache misses — is the single largest driver of the
    3–4× harness cost gap.
12. **Repo hygiene from the comparison**: `wall_clock_limit`,
    `ResourceBudgetPolicy.max_model_calls`/`max_capability_invocations`, and
    `SteeringPolicy.allow_steering`/`allow_interrupt` are defined but never enforced —
    wire them or delete them.

### 7.4 What *not* to adopt

- Pi's no-permission, no-sandbox trust model — incompatible with IronClaw's
  untrusted-channel surface.
- Ripping out MCP/hosted extensions: IronClaw's extension ecosystem is a product
  feature. The lesson is not "no MCP" but "no *always-resident* schemas" — deferred
  references give the same economics.
- Retry-outside-the-loop as literal structure: IronClaw's in-stage recovery is coupled
  to checkpoints and budget settlement and should stay; the adoptable idea is only
  that model *errors* stay out of the durable/LLM-visible transcript, which IronClaw
  already does via ephemeral observations.

## Sources

- Code: `/data/illia/research/pi-mono` (clone of github.com/badlogic/pi-mono, 2026-08-01)
- IronClaw comparison (§7): deep-read of `crates/ironclaw_agent_loop`,
  `ironclaw_loop_host`, `ironclaw_turn_runner`, `ironclaw_llm`,
  `ironclaw_composition` at commit `fe8f5c245` (2026-08-01)
- Mario Zechner, *What I learned building an opinionated and minimal coding agent*
  (mariozechner.at, 2025-11-30) + Terminal-Bench 2.0 results gist
- Databricks Engineering, *Benchmarking Coding Agents on Databricks' Multi-Million
  Line Codebase* (2026-07-08)
- nqawhc, *Your agent harness is an efficiency decision, not a quality decision*
  (2026-07-19)
- openbench (github.com/minghinmatthewlam/openbench, 2026-07)
- Portkey, *The Harness Tax* (2026-04-13)
- HN discussion: *Pi – A minimal terminal coding harness* (id 47143754)
