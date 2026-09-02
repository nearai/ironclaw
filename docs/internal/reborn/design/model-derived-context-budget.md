# Model-derived prompt context budget

Shape spec. Resolve the provider-advertised context window once per run, carry
it on `LoopRunContext`, and read it from every consumer that today reaches an
independent `PromptContextTokenBudget::default()`.

## Extend-vs-fork verdict

**Extends** `PromptContextTokenBudget` — the existing canonical budget type in
`ironclaw_loop_contracts`. No new budget type, no parallel "dynamic" path, no
cargo feature. `visible_transcript_tokens()` already computes
`limit − max(reserve, max_output)`; `estimate_tokens_from_chars` already does
chars/4. The only new behavior is what populates `context_limit_tokens`.

## Deletion-first check

Deleting alone does not solve it — a real per-model number still has to come
from somewhere. But the change **is** net-subtractive at the wiring level: five
independent production `::default()` sites collapse to one resolver, and the
`with_prompt_context_token_budget` builders (today test-only dead weight,
`ironclaw_loop_host/src/lib.rs:482` and `:1592`) gain their first production
callers instead of being deleted.

## Alternatives considered

Recorded because this changes a serialized contracts struct and recomputes four
replay-identity digests — both awkward to reverse once runs exist.

| Option | Mechanism | Why not |
|---|---|---|
| **Do nothing** | Keep 128k for every model. | A 2M-window model compacts at 108k, discarding 95% of usable context. A sub-128k model is sent oversized prompts, and the only overflow remedy (`executor/model.rs:475-481`) forces compaction against a ceiling the prompt already satisfies, so the run cannot converge. Rejected: this is a live correctness bug, not just waste. |
| **B — resolve at family composition, via `FamilyOverrides`** | Add a budget field to the existing overrides seam (`families/mod.rs:78-84`). | `FamilyOverrides` carries only `iteration_limit` and `model_availability_attempts` today, and `default_with_overrides` applies them through `.with_budget()` and `.with_recovery()` (`:118-128`) — it does not reach the compaction strategy at all, so a budget override would mean extending that seam too. More decisively, everything it builds is constructed inside `ironclaw_agent_loop`, which is barred from depending on `ironclaw_loop_host`, so it could never reach the ports at all — the loop-host consumers would still need separate wiring, giving two carriers of one number. And the digest is a pure function of resolved values, so every distinct window yields a distinct family identity, fragmenting replay identity per model. |
| **C — static per-model table consulted directly by the ports** | Generalize `gemini_context_length` and look it up at each consumer. | No async and a tiny diff, but each consumer performs its own lookup — the same multi-carrier problem as B, and it ignores what providers actually report. Adopted *as the data source* for the follow-on slice (see below), rejected as the *transport*. |
| **A — resolve per run, carry on `LoopRunContext`** ✅ | One resolution at host construction; every consumer reads one field. | Chosen. One carrier, one value; the digest stays stable across models; `should_compact` already receives `&LoopRunContext` and ignores it, so the agent-loop side costs almost nothing. |

Cost to undo: moderate. One `Option` field on a contracts struct (serde-default,
so old runs replay) and one digest bump to revert.

## Untouched

- `ironclaw_threads` — its `truncate_context_window` (`contract.rs:873`) is a
  message-**count** cap on a different axis. Not this change.
- The durable transcript. This is a read-time projection throughout; root
  `AGENTS.md:126` ("LLM data is never deleted") is unaffected.
- `LlmModelCatalogEntry` (`ironclaw_product_contracts/src/operator_llm.rs`) —
  modality-only presentation metadata, documented as forbidden from
  influencing routing or policy. Do not route context length through it.
- `ironclaw_agent_loop`'s dependency set. It stays contracts-only
  (`reborn_dependency_boundaries.rs:308-317`); the budget reaches it as a
  contracts-tier value on `LoopRunContext`, never as a provider dependency.
- `MODEL_WORK_ESTIMATED_CHARS_PER_TOKEN` (`model_work.rs:13`) — a second chars/4
  constant, but it feeds `budget_accountant.rs:380` (spend), not a context
  ceiling. Different axis; leave it alone.
- **`ThreadBackedLoopModelGateway::issue_host_prompt_bundle`**
  (`model_gateway.rs:311-326`) builds a context port with the cache but no
  budget, so it keeps the 128k default. It is non-test code exported at
  `lib.rs:122`, but `ThreadBackedLoopModelGateway::new` is instantiated only by
  `ironclaw_loop_host/tests/llm_gateway.rs` — never by composition, which wires
  `ThreadResolvingLoopModelGateway`. Left on the default deliberately: wiring an
  unreachable path would add a call site with no production meaning. If it is
  ever composed, it must be wired in that change.

## 1. Derivation — `crates/contracts/ironclaw_loop_contracts/src/context_budget.rs`

Add `serde::Deserialize` to the derive at `:7` (today it is `Serialize` only;
`LoopRunContext` is deserialized on replay).

```rust
impl PromptContextTokenBudget {
    /// Fraction of the advertised window we will fill. The margin absorbs
    /// chars/4 estimate error, which is the only reason it exists — the
    /// response headroom is `reserve_tokens`, a separate axis.
    pub const DEFAULT_USABLE_FRACTION_PERCENT: u64 = 90;

    /// Derive a budget from a provider-advertised total context window.
    ///
    /// `None` reproduces today's compiled-in 128k/20k exactly, so a provider
    /// that reports nothing behaves as it does now.
    pub fn from_advertised_window(advertised_tokens: Option<u64>) -> Self {
        let Some(advertised) = advertised_tokens.filter(|tokens| *tokens > 0) else {
            return Self::default();
        };
        let context_limit_tokens =
            advertised.saturating_mul(Self::DEFAULT_USABLE_FRACTION_PERCENT) / 100;
        // A small-window model would otherwise have its whole budget consumed
        // by the flat response reserve, leaving zero visible transcript.
        let reserve_tokens = Self::DEFAULT_RESERVE_TOKENS.min(context_limit_tokens / 4);
        Self {
            context_limit_tokens,
            reserve_tokens,
            main_loop_max_output_tokens: Self::DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS,
        }
    }
}
```

## 2. Source — `crates/loop/ironclaw_loop_host/src/lib.rs`

New defaulted method on `#[async_trait] pub trait HostManagedModelGateway`
(`:2293`), copying the narrow shape of its neighbour `diagnostic_effective_model`
(`:2297`) — best-effort, route-keyed, `None`-defaulting:

```rust
    async fn advertised_context_window_tokens(
        &self,
        _model_profile_id: &ModelProfileId,
        _resolved_model_route: Option<&HostManagedModelRouteSnapshot>,
    ) -> Option<u64> {
        None
    }
```

Override in `LlmProviderModelGateway` (`model_gateway.rs:406`) — the gateway
composition actually wires (`ironclaw_composition/src/model_gateway_assembly.rs:138`)
— reading `model_metadata().context_length`, the field at
`ironclaw_llm/src/provider.rs:932` that has no consumer today.

**The route parameter is load-bearing, not decoration.** `model_metadata()` takes
no model argument (`provider.rs:1016`); it describes whatever model the provider
was configured with. But the same gateway's request path resolves the served
model through `request_model_override` (`model_gateway.rs:1221-1248`) as
*route-requested → route override → `provider.active_model_name()`*, and its own
comment records that "providers that honor per-request overrides (e.g. NEAR AI)
serve the requested model." So on a run whose advisory route overrides the model,
`model_metadata()` can describe a **different model than the one served** — and a
window borrowed from the wrong model is worse than no window, because guessing
high produces the provider rejection this work exists to prevent.

The override must therefore verify identity and fail safe:

```rust
    async fn advertised_context_window_tokens(
        &self,
        model_profile_id: &ModelProfileId,
        resolved_model_route: Option<&HostManagedModelRouteSnapshot>,
    ) -> Option<u64> {
        let metadata = self.provider.model_metadata().await.ok()?;
        // Resolve the model this run will actually be served, the same way
        // stream_model does, and only trust the window if it describes that
        // model. A mismatch falls back to the compiled-in default.
        let route = self.policy.route_for(model_profile_id)?;
        let served = request_model_override(
            route,
            self.provider.as_ref(),
            resolved_model_route.map(HostManagedModelRouteSnapshot::model_id),
        )
        .ok()?;
        (served == metadata.id).then_some(())?;
        metadata.context_length.map(u64::from)
    }
```

This is what makes the derived budget genuinely a function of the pinned route.
Where it cannot be — a provider that serves an overridden model without
reporting metadata for it — the run keeps today's behavior rather than a wrong
number.

## 3. Transport — `crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs:236`

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_context_budget: Option<PromptContextTokenBudget>,
```

Set in `loop_driver_host.rs` beside the existing route resolution
(`attach_model_route_snapshot`, `:2131`), before the ports are built.

## 4. Consumers — the complete inventory

There are **five** production `PromptContextTokenBudget::default()` sites across
**three** types. Getting this list wrong is the central risk: the spec's whole
premise is that one number governs, and wiring a subset produces a loop that
*selects* messages against one ceiling while *compacting* against another.

| # | Site | Type | How it gets the resolved value |
|---|---|---|---|
| 1 | `compaction.rs:103` | `DefaultCompactionStrategy` | reads `ctx.resolved_context_budget` in `should_compact` |
| 2 | `loop_driver_host/config.rs:17` | `TextOnlyLoopHostConfig` | stays the fallback when nothing is advertised |
| 3 | `lib.rs:430` | `ThreadBackedLoopContextPort` | `.with_prompt_context_token_budget(..)` (`:482`) |
| 4 | `lib.rs:1538` | `ThreadBackedLoopModelPort::new` | `.with_prompt_context_token_budget(..)` (`:1592`), threaded through the gateway |
| 5 | `lib.rs:1566` | `ThreadBackedLoopModelPort::with_milestone_sink` | same |

**a. Compaction trigger** — `ironclaw_agent_loop/src/strategies/compaction.rs`.
`CompactionStrategy::should_compact` (`:21-25`) **already receives
`ctx: &LoopRunContext` and both impls ignore it** (`compaction.rs:114`,
`active_task_compaction.rs:45` — named `_ctx`). Use it:

```rust
fn effective_budget(&self, ctx: &LoopRunContext) -> PromptContextTokenBudget {
    ctx.resolved_context_budget.unwrap_or(self.prompt_context_budget)
}
```

`can_evaluate` (`:56`) and `trigger_at` (`:76`) take a `budget` parameter
instead of reading the field.

**b. Prompt context port** — `loop_driver_host.rs:1647`, via the builder at
`lib.rs:482`. Feeds the prompt's assembled context.

**c. Model port — the one that sizes the outbound request.**
`ThreadResolvingLoopModelGateway` is what composition wires
(`loop_driver_host.rs:1957-1998`). Its `stream_model_inner`
(`thread_resolving_model_gateway.rs:135-142`) builds
`ThreadBackedLoopModelPort::new(..)` with no budget, so the port keeps the
default; its `resolve_model_messages` (`lib.rs:2010-2021`) then calls
`select_prompt_context_messages(context.messages, self.prompt_context_budget, ..)`
— the call that decides which transcript messages actually reach the provider.
`ThreadResolvingLoopModelGatewayParts` (`:27-44`) has twelve fields and none is a
budget, so it needs a thirteenth, stored on the gateway and applied in
`stream_model_inner` via the builder at `lib.rs:1592`.

**d. Structured finalization** — `loop_driver_host.rs:2018` reads the same local
as (b) and (c), inside the same function, so it follows automatically.

## 5. Lifetime of the resolved value — deliberately not persisted

`LoopRunContext` is **not** the durable run record. `TurnRunState`
(`ironclaw_turns/src/status.rs:180-203`) is, and `create_host`
(`loop_driver_host.rs:2714-2736`) rebuilds `LoopRunContext` from it on every
claim, carrying forward only the five fields `TurnRunState` actually has. So
`resolved_context_budget` is **re-derived once per claim, not persisted**.

That is the intended design, not an oversight:

- The budget is a pure function of `resolved_model_route`, which *is* pinned
  durably on `TurnRunState:203` and carried forward at `:2732-2733`. Persisting
  the budget too would duplicate derivable state — the mirror-DTO pattern the
  repo bans.
- Re-deriving self-corrects. A frozen number stays wrong if it was resolved from
  a provider having a bad moment.

The one behavior this accepts: if a provider changes its advertised window for
the same model between claims of one run, the budget changes mid-run. That is
bounded — the route is pinned and §2's identity check rejects a window that
describes a different model — and preferable to freezing a stale value.

There is also a guard that short-circuits when a caller supplies a budget
explicitly. **Do not describe it as mirroring `attach_model_route_snapshot`'s
"already present" branch.** That branch is live in production because
`create_host` carries `resolved_model_route` forward from `TurnRunState`
(`:2731-2733`); no equivalent carry exists for the budget, so this guard's
precondition never occurs on a production claim. It protects direct callers of
`build_text_only_host_with_capabilities` — test harnesses today — from having a
deliberately-set budget silently overwritten. That is a narrow but real purpose;
it is not resume stability, and claiming the parallel would overstate it.

## 6. Replay fingerprint

`context_limit=128000` and `reserve=20000` are hand-typed into four fingerprint
strings: `families/mod.rs:35`, `families/subagent.rs:20`,
`families/unbound.rs:24,45`. Replace both literals with `context_limit=run_context`
and `reserve=run_context` and recompute the four `ComponentDigest` constants.
The digest then stays **stable across models**.
Digest tests (`families/mod.rs:164-173`) recompute from the fingerprint function
and self-heal; the `const` byte arrays are the edit. No production code compares
a persisted digest against a live one, so a revert is a code-only change.

## 7. Tests

| Tier | Where | Pins |
|---|---|---|
| 1 | `-p ironclaw_loop_contracts` | `from_advertised_window`: `None` → 128k/20k; 2M → 1.8M/20k; 8k → 7.2k clamped, non-zero visible |
| 1 | `-p ironclaw_agent_loop` | `should_compact` honors `ctx.resolved_context_budget` for both strategies |
| 1 | `-p ironclaw_loop_host` | the routed gateway returns the provider's `context_length`; the model port selects against an injected budget |
| 2 | `tests/integration/` | a small advertised window both compacts earlier **and** shrinks the message set actually sent |
| 3 | `-p ironclaw_architecture_tests` | `ironclaw_agent_loop` still contracts-only |

The integration assertion must cover the **selected message set**, not only a
compaction milestone. A compaction-only assertion passes while consumer (c) is
still on the default — the exact split this spec exists to prevent.

## Follow-on slice — populating the providers

Only `gemini_oauth.rs:2158` populates `ModelMetadata.context_length` today, from
a static name table (`:136-155`). `bedrock.rs:240` and
`openai_codex_provider.rs:416` return `None`; every other provider inherits the
`None` default. **The main slice makes those providers behave exactly as they do
now** and gives them a seam that pays off as each is filled.

### Why not `/models` alone

`/models` carries a window for roughly half the fleet — Gemini
(`inputTokenLimit`), OpenRouter / Groq / Together, GitHub Copilot, Ollama via a
second `/api/show`. It carries **nothing** for OpenAI, Anthropic, or Bedrock.
Verify each shape with one `curl` before writing its parser.

The catalog is also not cached: `list_model_catalog()` has one consumer outside
this crate — `llm_config_service.rs:805`, a live admin probe returned straight to
the settings UI. Binding the run path to it would mean an HTTP round-trip per run
or new cache infrastructure. So the catalog is a *free bonus where it exists*,
not the mechanism.

### Two parts

**a. Static table** — new `model_context_windows.rs` in `ironclaw_llm`'s
model-catalog family. Add it to the sub-owner table in `CONTRACT.md` or
`tests/module_charter.rs` fails.

**Deviation from the neighbouring idiom, deliberately:** `vision_models.rs:14`
and `reasoning_models.rs:31` match with `lower.contains(pattern)`. Do **not**
copy that here. Substring matching is safe for a boolean capability and unsafe
for a magnitude — `"gpt-4"` is a substring of both `gpt-4o` and `gpt-4.1`, whose
windows differ by 8×, and guessing high causes exactly the provider rejection
this work exists to prevent. Match exact model id first, then explicit
longest-prefix family:

```rust
/// Returns `None` for anything unrecognized — the caller then falls back to
/// the compiled-in default. Never guess a window for an unknown model.
pub fn context_window_for(model_id: &str) -> Option<u64>;
```

Populate from each provider's published limits at implementation time; do not
carry values over from this document.

**b. Catalog field** — add `context_length: Option<u32>` to `DiscoveredModel`
(`models.rs:36`) and read it where the body is already deserialized.
`parse_nearai_models` (`nearai_chat.rs:56`) is the pattern: alias-tolerant,
`#[serde(default)]`, lossy.

Resolution order in `model_metadata()`: catalog → static table → `None`.

### Later, if it earns it

`error.rs:437-465` already parses the provider's authoritative limit into
`LlmError::ContextLengthExceeded { used, limit }`, and `model_gateway.rs:2692`
matches `{ .. }` and discards it. Carrying `limit` through would make the budget
self-correcting — but only after one overflow, and it needs a durable store.

## Why this bug is worth fixing beyond wasted headroom

`executor/model.rs:475-481`: the sole corrective action on a provider
`context_length_exceeded` is `force_compact_on_next_iteration`, aimed at the
**same unchanged 128k ceiling**. For a model whose real window is under 128k,
compaction believes there is headroom, shrinks nothing meaningful, and the run
aborts on the second overflow (`recovery.rs:489-505`). Today that model can
never converge. Note this requires consumer (c) to be wired: fixing only the
compaction trigger changes when compaction fires but not the size of the request
that overflowed.
