# Hermes-Style Context Management for Reborn — Tool Disclosure, NEAR AI Prompt Caching, and Context Compression

**Status:** Fusion design — **SIGNED OFF**. Opus 4.8 = ACCEPT; GPT-5.5 (xhigh) = ACCEPT_WITH_NONBLOCKING_NOTES. Zero blockers. Council: 2 independent drafts → 1 cross-review round (both ACCEPT_WITH_CHANGES) → fusion → signoff. Non-blocking notes folded into §3/§7/§8. See Agreement Ledger (§11).
**Date:** 2026-06-23
**Owners:** Reborn runtime / agent-loop

## 1. Problem statement

Every model call ships all ~91 tool JSON schemas + system prompt (identity files, skills index, guidance) + the growing conversation. At `message_count=5` the request is already ~25,800 input tokens, the tool schemas being the dominant fixed chunk. A single user turn makes ~4 sequential model calls (`model_call=4`), each re-sending that whole prompt to NEAR AI. NEAR AI inference is slow (53–83s healthy) and currently exceeds the 120s request timeout → 3 retries → turn fails → no answer. We must (a) cut per-call prompt size and (b) maximize server-side prefix-cache reuse, without changing provider.

## 2. Goals / non-goals

**Goals**
- Cut the early-turn request from ~25.8k to **≤8–12k input tokens** (before conversation growth) by not advertising all 91 tool schemas every call.
- Maximize NEAR AI automatic server-side prefix-cache reuse via byte-stable prompt prefixes.
- A context-compression engine that bounds prompt growth over long sessions, compressing only the *prompt view*.
- Stay correct: any tool reachable today must stay reachable; all execution keeps its audit/safety/approval guarantees.

**Non-goals**
- Switching providers (must work on NEAR AI / OpenAI-compatible `/v1/chat/completions`).
- Anthropic-style `cache_control` (unsupported by NEAR AI).
- Deleting or mutating any DB transcript/tool/reasoning rows ("LLM data is never deleted").
- Changing the `ToolDispatcher` safety/audit pipeline.

## 3. Constraints and assumptions

- NEAR AI takes tools via the API `tools=` array (OpenAI function-schema). Caching, if any, is **automatic server-side prefix KV-cache** (vLLM/SGLang-style), eviction-driven and replica-dependent. **Prefix stability is necessary but not sufficient** for a cache hit. We do not assume `cached_tokens` is reported — it is an *empirical gate* (§7).
- "LLM data is never deleted" — compression edits only the prompt *view*; the full transcript is always reconstructable from the DB.
- Every tool execution routes through `ToolDispatcher::dispatch()`; bridge tools must be provably equivalent to direct dispatch.
- Changes contained to: agent-loop prompt assembly (`crates/ironclaw_agent_loop/src/executor/prompt.rs`, `model.rs`), the model gateway (`crates/ironclaw_reborn/src/model_gateway.rs`), a new tool catalog/index, a new prompt-view compressor, and a new artifact store.
- All thresholds below are **config defaults / rollout knobs** (`src/config/`), to be tuned from production traces — not baked constants. They land as **named constants on one config surface** (not scattered across the compressor/disclosure/cache modules) so canary tuning has a single home: `4k` per-result and `16k` aggregate cheap-prune triggers, the `~50%` input-budget dominant gate, the `24-tool / 12k-schema-token` advertise cap, and promotion `N=2`.

## 4. Final design

### 4.1 Progressive tool disclosure — bridges-always + append-only earned promotion

**Canonical tool-surface order (the cache-stability invariant):**

```
[ stable core ]  →  [ 3 bridges ]  →  [ append-only promoted tools ]
```

This order is **never re-sorted**. Tools are only *appended*. This is the resolution of the central design tension (see §6 / §11): it gives the model the correctness floor of bridges-always-present, the relevance of direct schemas, and the byte-stability the prefix cache needs — simultaneously.

**`CapabilityCatalog`** (built once per `CapabilitySurfaceVersion`, frozen): for each of the ~91 tools it stores provider tool name, canonical capability id, safe one-line description, schema summary, full JSON schema (recursively canonicalized: object keys sorted), required params, runtime/effects (read-only vs side-effecting), source (builtin/extension/skill), estimated schema tokens, and a stable per-tool digest. Sorted by provider tool name. A per-catalog in-memory **BM25 index** (tool name + description + keywords/tags) powers discovery — *not* the workspace memory FTS+vector store (tool discovery must be small, deterministic, per-surface, embedding-free).

**Core set (~12–16, always advertised):** the bridges plus loop primitives that telemetry shows are used in essentially every turn. Membership is **derived from production tool-call frequency + permissions, profile-specific**, and **side-effecting tools are not auto-core merely because common** (e.g. `shell`/`apply_patch` are core only where the profile warrants). Candidate core: `tool_search`, `tool_describe`, `tool_call`, `result_read`, `memory_search`, `memory_read`, `memory_write`, `skill_search`, `file_read`, `list_dir`, plus profile-pinned essentials. Exact list is an open question pending log analysis (§6).

**Bridge tools** (real dispatched tools, next to `capability_info`):
- `tool_search(query, limit=10)` → BM25 over the catalog; returns names, one-line descriptions, required params, effects, scores.
- `tool_describe(name)` → one full canonical schema + digest for a visible or deferred capability.
- `tool_call(name, args)` → resolves name → capability id; rejects recursive bridge calls; verifies the target was visible in the active catalog epoch **OR was disclosed by `tool_search`/`tool_describe` earlier in the same turn** (the same-turn discovery exception); then invokes `ToolDispatcher::dispatch()`. Audit records both the bridge invocation and the target invocation with a parent dispatch id.

**Promotion (earned, never guessed):** a deferred tool earns a stable direct slot — *appended* to the promoted suffix — when any of: it was `tool_describe`d then used; it was used ≥ N times this session (default N=2); it is profile-pinned; it is named explicitly in the user message; or a high-confidence deterministic classifier matches. **No per-turn keyword re-scoring drives the wire surface**, and promoted tools are never reordered. When the promoted suffix would exceed the budget cap (default 24 advertised tools / 12k schema tokens), eviction starts a **deliberate new prompt-surface epoch** (a measured, logged cache-reset), rather than silently reshuffling.

**Deferral trigger:** advertise the full set only while total schema cost ≤ `min(12k tokens, 10% of context_limit)` and tool count ≤ 32; above that, run deferred mode (core + bridges + promoted).

**Per-turn memoization:** the active set is computed once per user turn and reused for all ~4 model calls, so `tools=` is byte-identical across the calls of a turn.

### 4.2 NEAR AI prompt caching — stable-prefix discipline

No `cache_control`; we optimize exact-prefix stability and raw size. Assemble the request in four contiguous tiers, **volatile last**:

1. **Stable session prefix** — identity files (AGENTS/SOUL/USER/IDENTITY), static guidance, frozen skills index, session-frozen snippets. Built once per session / run-profile epoch into a cached `Arc<str>`, reused verbatim.
2. **Stable tool list** — canonical, append-only `tools=` from §4.1. In deferred steady state this is core + bridges + promoted, stable across turns.
3. **Stable context** — committed transcript + summaries in sequence order, never re-edited above the compaction watermark.
4. **Volatile (last)** — timestamp, memory-recall snippets, loop-control/repeat warnings, the final-answer nudge, current user turn + recent tail.

Each tier carries a fingerprint (`stable_prefix_digest`, `tool_list_digest`, `context_digest`, `volatile_digest`) + estimated tokens, logged per call. Canonical JSON (sorted keys), integer token estimates, name tie-breaks — determinism everywhere in tiers 1–3.

**Measurement & empirical gate:** extend `nearai_chat` usage parsing to capture `usage.prompt_tokens_details.cached_tokens` (and vLLM/SGLang variants) into the existing `cache_read_input_tokens` on `HostManagedModelResponse`. Treat `cached_tokens` as **ground truth where present**; treat prefix-digest reuse as a **lower-bound proxy only** (it over-counts because hits are eviction/replica-dependent). **Primary KPI is raw `prompt_tokens` reduction**, which wins regardless of whether NEAR caches. Cache behavior is verified empirically before any cache-specific tuning is justified.

### 4.3 Context compression — `PromptViewCompressor` (DB-backed, deterministic-first)

Transforms only the prompt view; never deletes/mutates DB transcript or tool rows. It may add summary artifacts + prompt-view metadata.

**Artifact store contract (new):** a **session/run-scoped artifact table** (not user-facing workspace memory) with: scope, retention/TTL, authorization, redaction, an audited recall API, and a dedicated core recall tool **`result_read(result_ref, offset, max_bytes)`**. Full tool output is always reconstructable from the DB; the artifact store is the prompt-view handle layer.

**Triggers (single dominant absolute gate + always-on cheap pruning):**
- Always run the cheap deterministic phases (1–3) when any single tool result > 4k chars or the old-tool-result aggregate > 16k chars.
- Run full compression when estimated total input exceeds the dominant gate (config default; start ~50% of the input budget as an absolute token count — *one* unit, not a relative/absolute mix).
- Force compression after `ShrinkContext`, a provider timeout, or per-turn tool output aggregate > 128k chars.
- **Always preserve the exact tail:** last ~12k tokens, last user message, last assistant message, unresolved tool calls, and pending auth/approval/resource gates.

**Phases:**
1. **Result-handle substitution** — old tool results → stable DB handles: `[tool_result ref=… capability=… bytes=… digest=… summary=…]`; duplicate digests → `[same_as ref=…]`. Recall via `result_read`. *(Improves on Hermes: reference-by-handle into our DB, lossless and cache-stable, instead of LLM re-summarizing results.)*
2. **Tool-type one-liners** — deterministic per-family summaries (terminal: cmd/exit/lines; http: url/status/bytes; file: path/op/bytes; memory: query/count; skill: names/action); unknown → safe summary + bytes + digest. Cap each old observation to 300 chars, 6k total.
3. **Old-argument slimming** — assistant tool calls outside the tail keep name + required args + digest + head/tail snippets; full args stay in DB.
4. **Middle summarization (last resort)** — if still over budget, summarize only the already-reduced middle via NEAR AI system-inference; persist an idempotent `SummaryArtifact` keyed `(thread, start_seq, end_seq, source_digest)` with `ReplaceRangeWhenSelected`; reuse the artifact instead of re-summarizing.
5. **Anti-thrash** — keyed on artifact-reuse hit-rate *and* savings: if two consecutive passes each save < 15% and produce no reusable artifact, disable LLM summarization until the transcript grows by 8k tokens / 10 messages.

## 5. Runtime / security / data implications

- **Bridge equivalence:** `tool_call` must pass the same schema validation, auth, approval gates, sandboxing, rate limits, audit/`ActionRecord`, and error semantics as a direct call — verified by through-the-caller tests (§8). Unknown/invisible target → model-recoverable `Failed(InvalidInput)`, never `Err` (run death).
- **Artifact store** is scope/authz/TTL-bounded and redaction-aware; recall is audited. It is distinct from workspace memory.
- **Determinism** in tiers 1–3 is a security/correctness property (predictable surface) as well as a cache property.

## 6. Key decisions and alternatives rejected

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Bridges always present + **append-only earned promotion** | Correctness floor (any tool discoverable) + relevance (earned direct slots) + cache stability (no resort). Resolves Opus's "discovery tax every turn" and GPT's "scorer-as-contract / churn" simultaneously. | Pure bridges-only default (recurring discovery tax); pure per-turn scored overlay (cross-turn churn, fragile scorer in the runtime contract). |
| BM25 over a per-surface catalog | Small, deterministic, embedding-free, decoupled from user memory state. | Workspace memory FTS+vector (latency, nondeterminism, cache-unstable). |
| Reference-by-handle into a scoped artifact store + `result_read` | Lossless, deterministic, cache-stable; full data in DB. | LLM re-summarizing tool results (lossy, cache-hostile, costs aux calls); overloading `memory_read` (wrong semantics). |
| Deterministic phases before any LLM summary; single absolute trigger | Cheaper, testable, cache-stable; avoids unit-conflation. | Dual relative+absolute trigger (Opus flagged contradictory firing). |
| `cached_tokens` as empirical gate; raw tokens as primary KPI | Prefix stability is necessary-not-sufficient on vLLM/SGLang. | Assuming digest-reuse ≈ cache savings (over-counts). |
| Thresholds as config knobs from traces | Avoids baking unvalidated constants. | Hard-coded 10%/20k/55%/8k. |

**Open questions:** exact Core membership (needs tool-call frequency + permission analysis, per profile); does NEAR AI actually emit `cached_tokens`; promotion N and eviction/epoch policy tuning; interaction of `PromptViewCompressor` with the existing compaction axis (`compaction.rs` / `active_task_compaction`); cheap deterministic aux model for phase-4 and whether temp-0 is reproducible on NEAR infra.

## 7. Test and validation plan

**Unit:** catalog digest/order stable across HashMap iteration and JSON key order; active set keeps core, enforces caps, identical across the 4 calls of a turn; promotion is append-only and never re-sorts; bridges deny invisible/recursive targets and honor the same-turn discovery exception; NEAR usage parser captures `cached_tokens` when present; compressor preserves the protected tail and substitutes handles; anti-thrash disables/re-enables correctly.

**Integration (`--features integration`, through the caller):** `tool_call` through `ToolDispatcher` produces the same `ActionRecord`/safety/approval path as a direct call (including a safety-sensitive tool); **explicitly cover invoking a tool via `tool_call` after only `tool_search`/`tool_describe` disclosure (not in the active catalog epoch)** to lock that the same-turn-discovery + recursion-reject + visibility gate matches direct-dispatch semantics; 91-tool Reborn fixture drops from ~25.8k to ≤12k input tokens; Postgres + libSQL both retain original rows while the prompt view is compressed; timeout/`ShrinkContext` forces compression without dropping the latest user intent; tiers 0–2 byte-stable across the 4 calls of a simulated turn.

**Metrics:** total input tokens, schema tokens, advertised tool count, `cached_tokens` (where present), prefix-digest reuse rate (lower-bound), common-prefix estimated tokens, p50/p95 model latency, timeout/retry rate, bridge search/describe/call counts, compression ratio, and turn-completion call count. **Gate:** PinchBench ≥ 0.768 (no benchmark regression).

## 8. Rollout / migration

- **Phase 0 — shadow:** compute catalog, active set, token estimates, tier digests, and compression plan, but send the existing prompts. Measure token deltas with zero behavior change. **Phase 0 must produce a definitive yes/no on whether NEAR AI populates `usage.prompt_tokens_details.cached_tokens` at all**, so the empirical cache gate is answered *before* Phase 1 rather than mid-rollout. Per both slots: the first rollout *validates* NEAR cache behavior, it does not *depend* on it — raw prompt-token reduction is the primary success path regardless.
- **Phase 1 — disclosure:** enable core + bridges + earned promotion behind `REBORN_TOOL_DISCLOSURE` (per-profile). Canary compares advertised-count, cached_tokens, p95, and turn-completion-call-count; canary decides per-profile promotion aggressiveness.
- **Phase 2 — deterministic compression:** enable phases 1–3 (handle substitution, one-liners, arg slimming).
- **Phase 3 — LLM middle-summary:** enable phase 4 after fixture + canary pass.
- All phases flag-gated, default-off, staged tiering → disclosure → compression.

## 9. Risks and mitigations

- **Prefix churn from promotion/eviction** → append-only order + deliberate epoch on eviction; track `tool_list_digest`; fall back to bridges-only per profile if measured reuse drops.
- **Bridge bypass of safety** → equivalence tests through the real dispatcher path, epoch + recursion guards.
- **Model can't find a deferred tool** → bridges + gateway `recover_textual_tool_calls` + "describe first"/unknown-name model-visible errors.
- **Compression loss** → exact tail + unresolved protocol messages preserved; `result_read` hydration; full data in DB.
- **No server cache** → raw-token reduction is the load-bearing win; caching is a bonus multiplier.
- **Token-estimate drift** → conservative integer estimator with headroom; log estimated-vs-actual from returned usage.

## 10. Definition of done

Phase-1 lands disclosure with ≤12k early-turn tokens and no PinchBench regression; Phase-2/3 bound long-session growth; `cached_tokens` measured (or definitively shown absent); zero run-borking errors introduced (per the no-run-borking goal).

## 11. Agreement ledger

- **Both slots ACCEPT_WITH_CHANGES** on their independent drafts; no blockers.
- **Tool disclosure (the one real tension):** fused to **bridges-always + append-only earned promotion**. Adopts GPT-5.5's canonical order (core→bridges→append-only promoted) and "earned, not guessed" promotion (which resolves the hysteresis-vs-sort contradiction GPT identified); adopts Opus's argument that the recurring discovery tax must be avoided (promoted tools avoid re-discovery) and its same-turn discovery exception. Per-turn keyword *re-scoring of the wire surface* — rejected by both in the end.
- **Both** independently converged on: CapabilityCatalog per surface version; BM25 (not memory FTS/vector); 3 bridge tools through `ToolDispatcher`; per-turn memoized active set; stable/volatile tiers volatile-last; DB-handle references over LLM result-summarization; deterministic-before-LLM compression; idempotent summary artifacts; protect-tail; Phase-0 shadow rollout.
- **From GPT-5.5 review (adopted):** concrete artifact-persistence contract + dedicated `result_read` (not `memory_read`); single dominant compression trigger; thresholds as trace-derived knobs; core membership from telemetry/permissions with side-effecting tools not auto-core; `cached_tokens` as empirical gate; bridge equivalence tests.
- **From Opus review (adopted):** prefix-digest reuse is a lower-bound proxy, `cached_tokens` is ground truth; same-turn discovery exception in the epoch check; anti-thrash keyed on artifact-reuse hit-rate; PinchBench gate.

## 12. Unresolved blockers

None. (Open *questions* in §6 are tuning/telemetry items, not blockers.)

## 13. Implementation status & codebase grounding (2026-06-23)

Implemented with Codex agents on branch `firat/reborn-context-management`, each pass reviewed for safety/correctness before commit. Grounding the design in the real code surfaced two important corrections to the plan's premises — recorded here honestly rather than papered over.

**Delivered (the genuine gap):**
- **Phase 0 — shadow measurement** (`8f56d7526`): NEAR AI `cached_tokens` capture (`prompt_tokens_details.cached_tokens` + top-level fallback) routed into `cache_read_input_tokens`; per-call `debug!` shadow logs (`target: ironclaw::reborn::context_shadow`) of prompt/cached tokens + tool-schema token estimate. Zero behavior change. **This also satisfies Phase 2's measurement/empirical-gate requirement.**
- **Phase 1.1 — disclosure catalog + selector + benchmark** (`db9d09978`): `CapabilityCatalog`, `select_active_set` (bridges-always + append-only promotion, council-agreed canonical order), 3 bridge definitions, deterministic `tool_search_rank`. **Token benchmark on a representative 91-tool surface: 21,240 → 1,427 tool-schema tokens = 93.3% reduction.**
- **Phase 1.2 — live wiring + bridge execution** (`dda1e5944`): implemented as a `LoopCapabilityPort` decorator (`ToolDisclosureCapabilityDecorator`) behind `REBORN_TOOL_DISCLOSURE` (off|bridged, **default off** ⇒ byte-identical when off). `visible_capabilities()` filters to the active set; bridge `tool_call` routes its synthetic target through `inner.validate/register/invoke` (same audit/safety/approval as a direct call); guards reject recursion / unknown / not-disclosed-this-turn as model-recoverable failures; append-only per-thread promotion. 267 lib tests pass. *Follow-up before prod enable: negative-path bridge tests + canary.*

**Pre-existing in the codebase (plan premises corrected):**
- **Phase 3 — context compression is ALREADY IMPLEMENTED.** The plan's premise ("we send growing context with effectively no compression") is wrong: `crates/ironclaw_agent_loop/` ships a wired `CompactionStrategy` (`ActiveTaskPreservingCompactionStrategy`, the `compose_default()` default) + `BudgetStrategy` (`DefaultBudgetStrategy`) + `PromptContextTokenBudget` + `CompactionPromptSnapshot`, and the data model already does reference-by-handle/summary-only via `LoopContextMessage { message_ref: Option<…>, safe_summary }` (`message_ref: None` ⇒ use `safe_summary` verbatim) with `LoopContextCompactionMetadata`. A from-scratch `PromptViewCompressor` was prototyped, found to be the wrong layer (the prompt-view `LoopModelMessage` is only `{role, content_ref}` — content is already externalized behind refs), and **discarded.** Any future work here is *targeted improvement* to the existing strategy (e.g. tool-type-aware one-liners), gated on measurement showing a real gap — not a new engine.
- **Phase 2 — stable-prefix prompt is largely pre-existing.** Instruction/context bundle assembly is already deterministic with a stable fingerprint (`InstructionBundleFingerprint`, `stable_ref_hash`, identity-first ordering); Phase 1.2 adds a deterministic, per-turn-memoized tool list. Combined with Phase-0's `cached_tokens` measurement, Phase 2's concrete asks are met. Further explicit volatile-last tiering is **deferred until production `cached_tokens` data shows NEAR caches at all** (the design's own empirical gate) — building it speculatively would violate the "measure first" decision.

**Net:** the reported production pain (≈25.8k-token request at `message_count=5`, dominated by 91 tool schemas, timing out on NEAR AI) is addressed by **Phase 1 tool disclosure** — the one genuinely-missing capability. Phases 0/1 are committed and verified; Phases 2/3 are satisfied by existing infrastructure + measurement.

Negative-path + flag-off safety tests landed (`2dc01271f`): recursion and unknown-target `tool_call` are recoverable `InvalidInput` failures that never dispatch to the inner port, and explicit `Off` never attaches the decorator (byte-identical request path).

> **⚠️ TEMPORARY benchmark override (revert before GA):** `ToolDisclosureMode::from_env` currently defaults to **`Bridged` when `REBORN_TOOL_DISCLOSURE` is unset**, because the remote benchmark cannot set env vars and we need it to exercise the disclosure path. Explicit `REBORN_TOOL_DISCLOSURE=off` remains the escape hatch. **This makes disclosure ON by default on this branch** — revert the catch-all arm in `from_raw` back to `Self::Off` once benchmarking is done. Tracked here as the one knowingly-non-default-off state.

The remaining validation is the benchmark A/B itself: run PinchBench with disclosure on (the temporary default) vs an `=off` control, compare the Phase-0 shadow-log token deltas (and whether `cached_tokens` is populated) and task quality (gate: ≥ 0.768, no regression vs the `=off` control). Watch for discovery friction — the conservative 10-tool core defers side-effecting tools, so if quality drops, widen the core (e.g. add `shell`/`write_file`) rather than abandoning disclosure.
