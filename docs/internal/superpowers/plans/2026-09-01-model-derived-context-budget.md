# Model-Derived Prompt Context Budget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the agent loop's prompt context budget derive from the model's real advertised context window instead of a compiled-in 128,000-token constant applied to every model on every provider.

**Architecture:** The budget type (`PromptContextTokenBudget`), the "limit minus buffer" formula (`visible_transcript_tokens()`), and the chars/4 estimator (`estimate_tokens_from_chars`) all already exist and are unchanged. So does the provider seam that reports a real window (`ModelMetadata.context_length`), which today has zero consumers outside `ironclaw_llm`. This plan connects them: the loop host asks its gateway for the window once per run, derives a budget, and puts it on `LoopRunContext` — from which all four consumers that today each reach an independent `PromptContextTokenBudget::default()` read it instead.

**Tech Stack:** Rust 2024 edition, tokio, `async_trait`, serde, BLAKE3 (replay digests), `cargo test` / `cargo clippy`.

**Spec:** `docs/internal/reborn/design/model-derived-context-budget.md`

## Global Constraints

- **`ironclaw_agent_loop` may take normal dependencies on contracts-layer crates only.** Enforced with zero exceptions by `crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs:308-317`. Never add `ironclaw_llm` (or any non-contracts crate) to `crates/loop/ironclaw_agent_loop/Cargo.toml`. The budget reaches this crate as a contracts-tier value on `LoopRunContext`.
- **No `.unwrap()` or `.expect()` in production code.** Tests are fine. Propagate with `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`.
- **Zero clippy warnings.** CI denies warnings: `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`.
- **No new cargo feature.** This is runtime configuration (`.claude/rules/cargo-features.md`).
- **LLM data is never deleted** (root `AGENTS.md:126`). Every change here is a read-time projection. No task may delete, redact, or skip persisting a transcript message because it no longer fits a budget.
- **Preserve existing defaults.** A provider that advertises nothing must behave exactly as it does today: 128,000 limit / 20,000 reserve.
- **All five default sites must end up on one value.** There are five production `PromptContextTokenBudget::default()` sites across three types: `compaction.rs:103` (`DefaultCompactionStrategy`), `loop_driver_host/config.rs:17` (`TextOnlyLoopHostConfig`), `lib.rs:430` (`ThreadBackedLoopContextPort`), and `lib.rs:1538` + `:1566` (`ThreadBackedLoopModelPort`). Wiring a subset produces a loop that *selects* messages against one ceiling while *compacting* against another — the exact failure this work exists to prevent. `ThreadBackedLoopContextPort` and `ThreadBackedLoopModelPort` are different types with nearly identical names; both take a budget, and wiring one reads like wiring both.
- **Structural vs behavioral commits never mix.** Tasks 3 and 6 are structural (no behavior change); every other task is behavioral. Do not combine them in one commit.
- **Every test helper a task names must be real or declared new.** Before writing any test, `rg` for each helper the task names. If it exists, the task cites its `file:line`; if it does not, the task says so outright and names the nearest existing pattern to model it on. Never write a test as though a helper exists when it does not — an implementer who trusts the plan will search, find nothing, and either stall or invent undisclosed shared test infrastructure. Both `ironclaw_loop_host` and `ironclaw_turn_runner` use **one bespoke double per test**: there is no shared `FakeProvider`, no `test_policy()`, no `test_host_factory()`.
- **`main_loop_max_output_tokens` default stays `0`.** Not in scope.

---

### Task 1: Derive a budget from an advertised window

Adds the pure derivation function to the existing budget type. No caller yet.

**Files:**
- Modify: `crates/contracts/ironclaw_loop_contracts/src/context_budget.rs:7` (derive), `:14-35` (impl block)
- Test: `crates/contracts/ironclaw_loop_contracts/src/context_budget.rs` (the existing `#[cfg(test)] mod tests` at `:47`)

**Interfaces:**
- Consumes: nothing.
- Produces: `PromptContextTokenBudget::from_advertised_window(advertised_tokens: Option<u64>) -> PromptContextTokenBudget` and the associated const `DEFAULT_USABLE_FRACTION_PERCENT: u64 = 90`. `PromptContextTokenBudget` also gains `serde::Deserialize`, which Task 2 requires.

**Context you need:** `PromptContextTokenBudget` has three public fields — `context_limit_tokens`, `reserve_tokens`, `main_loop_max_output_tokens` — and one method, `visible_transcript_tokens()`, which returns `context_limit_tokens - max(reserve_tokens, main_loop_max_output_tokens)`. That is the number of tokens of transcript the loop will actually put in a prompt. The two knobs are independent: `reserve_tokens` holds room for the model's *response*, while the new 90% fraction absorbs error in our chars/4 token estimate. Both apply; they are not the same buffer.

- [ ] **Step 1: Write the failing tests**

Append to the existing `mod tests` block at the end of `crates/contracts/ironclaw_loop_contracts/src/context_budget.rs`:

```rust
    #[test]
    fn advertised_window_of_none_reproduces_the_compiled_in_default() {
        // A provider that reports nothing must behave exactly as it does
        // today. This is the compatibility guarantee of the whole change.
        assert_eq!(
            PromptContextTokenBudget::from_advertised_window(None),
            PromptContextTokenBudget::default()
        );
    }

    #[test]
    fn advertised_window_of_zero_is_treated_as_unknown() {
        assert_eq!(
            PromptContextTokenBudget::from_advertised_window(Some(0)),
            PromptContextTokenBudget::default()
        );
    }

    #[test]
    fn large_advertised_window_keeps_the_flat_response_reserve() {
        let budget = PromptContextTokenBudget::from_advertised_window(Some(2_000_000));

        assert_eq!(budget.context_limit_tokens, 1_800_000);
        assert_eq!(
            budget.reserve_tokens,
            PromptContextTokenBudget::DEFAULT_RESERVE_TOKENS
        );
        assert_eq!(budget.visible_transcript_tokens(), 1_780_000);
    }

    #[test]
    fn small_advertised_window_clamps_the_reserve_and_keeps_budget_usable() {
        // An 8k model would otherwise have its entire budget consumed by the
        // flat 20k response reserve, leaving zero visible transcript and a
        // loop that cannot run at all.
        let budget = PromptContextTokenBudget::from_advertised_window(Some(8_000));

        assert_eq!(budget.context_limit_tokens, 7_200);
        assert_eq!(budget.reserve_tokens, 1_800);
        assert!(
            budget.visible_transcript_tokens() > 0,
            "a small-window model must still have room for transcript"
        );
    }

    #[test]
    fn advertised_window_matching_todays_constant_is_reduced_by_the_margin() {
        // 128k advertised is NOT the same as the 128k fallback: the fallback
        // is a guess, an advertised value gets the estimate-error margin.
        let budget = PromptContextTokenBudget::from_advertised_window(Some(128_000));

        assert_eq!(budget.context_limit_tokens, 115_200);
        assert_eq!(
            budget.reserve_tokens,
            PromptContextTokenBudget::DEFAULT_RESERVE_TOKENS
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p ironclaw_loop_contracts context_budget`
Expected: FAIL to compile — `no function or associated item named 'from_advertised_window' found`.

- [ ] **Step 3: Add the `Deserialize` derive**

In `crates/contracts/ironclaw_loop_contracts/src/context_budget.rs:7`, change:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
```

to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
```

- [ ] **Step 4: Write the derivation**

In the same file, inside `impl PromptContextTokenBudget` (after `DEFAULT_MAIN_LOOP_MAX_OUTPUT_TOKENS` at `:17`), add:

```rust
    /// Fraction of a provider-advertised window we are willing to fill.
    ///
    /// This margin exists to absorb error in the chars/4 token estimate
    /// (`estimate_tokens_from_chars`), which is the only reason for it. Room
    /// for the model's *response* is a separate axis — `reserve_tokens`.
    pub const DEFAULT_USABLE_FRACTION_PERCENT: u64 = 90;
```

and, after `visible_transcript_tokens` (`:31-34`), add:

```rust
    /// Derive a budget from a provider-advertised total context window.
    ///
    /// `None` (or a nonsense zero) reproduces the compiled-in default
    /// exactly, so a provider that advertises nothing behaves as it always
    /// has. Never guess a window for an unknown model: guessing high
    /// produces the provider rejection this mechanism exists to avoid.
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p ironclaw_loop_contracts context_budget`
Expected: PASS, all five new tests plus the pre-existing ones.

- [ ] **Step 6: Commit**

```bash
git add crates/contracts/ironclaw_loop_contracts/src/context_budget.rs
git commit -m "feat(loop-contracts): derive a prompt context budget from an advertised window"
```

---

### Task 2: Carry the resolved budget on the run context

Adds the transport field. Still no producer or consumer.

**Files:**
- Modify: `crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs:236-259` (struct), `:273-283` (`new`), `:345-348` (beside `with_resolved_model_route`)
- Test: same file's test module

**Interfaces:**
- Consumes: `PromptContextTokenBudget` with `Deserialize` (Task 1).
- Produces: `LoopRunContext.resolved_context_budget: Option<PromptContextTokenBudget>` and `LoopRunContext::with_resolved_context_budget(self, budget: PromptContextTokenBudget) -> Self`.

**Context you need:** `LoopRunContext` is the per-run context handed to every loop strategy. It is `Serialize + Deserialize` and is persisted, so runs recorded before this change must still deserialize — hence `#[serde(default)]`. It already carries `resolved_model_route: Option<LoopModelRouteSnapshot>` resolved the same way and at the same point, which is the pattern to copy.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs`. If the module imports a helper that builds a `LoopRunContext`, reuse it; otherwise build one with `LoopRunContext::new(...)` following the nearest existing test in that file.

```rust
    #[test]
    fn run_context_defaults_to_no_resolved_context_budget() {
        let context = sample_run_context();

        assert_eq!(context.resolved_context_budget, None);
    }

    #[test]
    fn run_context_carries_a_resolved_context_budget() {
        let budget = PromptContextTokenBudget::from_advertised_window(Some(200_000));
        let context = sample_run_context().with_resolved_context_budget(budget);

        assert_eq!(context.resolved_context_budget, Some(budget));
    }

    #[test]
    fn run_context_without_a_budget_field_still_deserializes() {
        // Runs recorded before this change must replay, landing on the
        // compiled-in default rather than failing to deserialize.
        let context = sample_run_context();
        let mut wire = serde_json::to_value(&context).expect("serialize");
        wire.as_object_mut()
            .expect("object")
            .remove("resolved_context_budget");

        let restored: LoopRunContext = serde_json::from_value(wire).expect("deserialize");

        assert_eq!(restored.resolved_context_budget, None);
    }

    #[test]
    fn resolved_context_budget_round_trips_through_the_wire() {
        let budget = PromptContextTokenBudget::from_advertised_window(Some(1_000_000));
        let context = sample_run_context().with_resolved_context_budget(budget);

        let wire = serde_json::to_string(&context).expect("serialize");
        let restored: LoopRunContext = serde_json::from_str(&wire).expect("deserialize");

        assert_eq!(restored.resolved_context_budget, Some(budget));
    }
```

If no `sample_run_context()` helper exists in that module, add one modeled on the nearest existing `LoopRunContext::new(...)` construction in the same file, and use it in all four tests.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p ironclaw_loop_contracts run_context`
Expected: FAIL to compile — `no field 'resolved_context_budget'` and `no method named 'with_resolved_context_budget'`.

- [ ] **Step 3: Add the field**

In `crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs`, in `pub struct LoopRunContext` (after `resolved_model_route` at `:247`):

```rust
    /// Prompt context budget resolved from this run's model at host
    /// construction. `None` — an older serialized context, or a provider that
    /// advertises no window — means the compiled-in default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_context_budget: Option<PromptContextTokenBudget>,
```

Add `resolved_context_budget: None,` to the struct literal in `LoopRunContext::new` (beside `resolved_model_route: None,` at `:281`). Import `PromptContextTokenBudget` at the top of the file if it is not already in scope (it lives at `crate::context_budget::PromptContextTokenBudget`).

- [ ] **Step 4: Add the builder**

Immediately after `with_resolved_model_route` (`:345-348`):

```rust
    pub fn with_resolved_context_budget(mut self, budget: PromptContextTokenBudget) -> Self {
        self.resolved_context_budget = Some(budget);
        self
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p ironclaw_loop_contracts`
Expected: PASS. If other constructions of `LoopRunContext` in this crate fail to compile because of the new field, add `resolved_context_budget: None,` to each — do not change their behavior.

- [ ] **Step 6: Commit**

```bash
git add crates/contracts/ironclaw_loop_contracts/src/host/run_context.rs
git commit -m "feat(loop-contracts): carry a resolved context budget on LoopRunContext"
```

---

### Task 3: Thread the budget through the compaction helpers (STRUCTURAL)

**This task must not change behavior.** It makes the budget an argument instead of a field read, so Task 4 can vary it. Every call site passes `self.prompt_context_budget`, so every decision the loop makes is byte-identical before and after.

**Files:**
- Modify: `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs:56` (`can_evaluate`), `:77` (`trigger_at`), `:116`/`:125`/`:129`/`:140` (call sites)
- Verify only (no edit): `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs:398-416` — see Step 4
- Modify: `crates/loop/ironclaw_agent_loop/src/strategies/active_task_compaction.rs:48`/`:62`/`:72` (call sites)

**Interfaces:**
- Consumes: nothing new.
- Produces: `DefaultCompactionStrategy::can_evaluate(&self, state: &LoopExecutionState, budget: PromptContextTokenBudget) -> bool` and `DefaultCompactionStrategy::trigger_at(&self, state: &LoopExecutionState, budget: PromptContextTokenBudget, drop_through_seq: u64) -> CompactionDecision`. Both stay `pub(super)`.

**Context you need:** `DefaultCompactionStrategy` decides *when* the loop should summarize old transcript. It reads `self.prompt_context_budget.visible_transcript_tokens()` in exactly two places — `can_evaluate` (is the observed prompt over the threshold?) and `trigger_at` (what threshold do we record as the effectiveness baseline?). `ActiveTaskPreservingCompactionStrategy` wraps it via a `base` field and calls both. There are exactly 7 call sites across the 2 files (`compaction.rs:116,125,129,140`; `active_task_compaction.rs:48,62,72`) and **no test calls either helper directly** — Step 4 verifies that.

- [ ] **Step 1: Change the two helper signatures**

In `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs`, change `can_evaluate` (`:56`) from reading the field to taking a parameter:

```rust
    pub(super) fn can_evaluate(
        &self,
        state: &LoopExecutionState,
        budget: PromptContextTokenBudget,
    ) -> bool {
        if state.compaction_prompt.message_index.is_empty() {
            return false;
        }
        let threshold = budget.visible_transcript_tokens();
```

Leave the rest of the function body unchanged.

Change `trigger_at` (`:77`) the same way:

```rust
    pub(super) fn trigger_at(
        &self,
        state: &LoopExecutionState,
        budget: PromptContextTokenBudget,
        drop_through_seq: u64,
    ) -> CompactionDecision {
```

and inside it, replace `self.prompt_context_budget.visible_transcript_tokens()` with `budget.visible_transcript_tokens()`.

Keep the `prompt_context_budget` field on the struct — Task 4 uses it as the fallback.

- [ ] **Step 2: Update the call sites in `compaction.rs`**

In `DefaultCompactionStrategy::should_compact` (`:111-142`), pass `self.prompt_context_budget` at each of the four sites:

```rust
        if !self.can_evaluate(state, self.prompt_context_budget) {
```

and, at `:125`, `:129`, and `:140`:

```rust
                    .map(|sequence| self.trigger_at(state, self.prompt_context_budget, sequence))
```

- [ ] **Step 3: Update the call sites in `active_task_compaction.rs`**

In `ActiveTaskPreservingCompactionStrategy::should_compact` (`:43-74`), at `:48`:

```rust
        if !self.base.can_evaluate(state, self.base.prompt_context_budget) {
```

and at `:62` and `:72`:

```rust
            .map(|sequence| self.base.trigger_at(state, self.base.prompt_context_budget, sequence))
```

- [ ] **Step 4: Confirm no test needs editing**

Run: `rg -n "can_evaluate\(|trigger_at\(" crates/loop/ironclaw_agent_loop/`
Expected: only the definitions (`compaction.rs:56`, `:77`) and the internal call sites you just changed (`compaction.rs:116,125,129,140`; `active_task_compaction.rs:48,62,72`). **No test calls either helper directly** — note that `can_evaluate_skips_when_visible_threshold_equals_preserve_tail` (`compaction.rs:398-416`) is named after `can_evaluate` but actually asserts on `strategy.should_compact(&state, &context)`. If this grep shows a test-level call, stop: the refactor's blast radius is larger than this plan assumes.

This zero-test-churn result is the proof that Task 3 is purely internal.

- [ ] **Step 5: Run the full crate suite to prove nothing moved**

Run: `cargo test -p ironclaw_agent_loop`
Expected: PASS, with **zero test edits in this task**. Compaction behavior is unchanged; if any test fails, you changed behavior — revert and redo. In particular the family digest tests must still pass, because the fingerprint string is untouched in this task.

- [ ] **Step 6: Verify clippy is clean**

Run: `cargo clippy -p ironclaw_agent_loop --tests -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs \
        crates/loop/ironclaw_agent_loop/src/strategies/active_task_compaction.rs
git commit -m "refactor(agent-loop): pass the compaction budget as an argument

Structural only. Every call site passes the strategy's own
prompt_context_budget, so every compaction decision is identical."
```

---

### Task 4: Let the run context override the compaction ceiling

**Files:**
- Modify: `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs` (new `effective_budget` + `should_compact`), `crates/loop/ironclaw_agent_loop/src/strategies/active_task_compaction.rs:43-74`
- Modify: `crates/loop/ironclaw_agent_loop/src/families/mod.rs:35` and `:53-56`, `crates/loop/ironclaw_agent_loop/src/families/subagent.rs:20` + its digest const, `crates/loop/ironclaw_agent_loop/src/families/unbound.rs:24`/`:45` + both digest consts
- Test: `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs` test module

**Interfaces:**
- Consumes: `LoopRunContext.resolved_context_budget` (Task 2), the parameterized helpers (Task 3).
- Produces: compaction that honors a per-run budget. No new public API.

**Context you need:** `CompactionStrategy::should_compact` already receives `ctx: &LoopRunContext` and both implementations currently name it `_ctx` and ignore it. This task uses it. The literal `context_limit=128000` also appears in four hand-typed "replay fingerprint" strings that get BLAKE3-hashed into family identity digests; those must change to say the ceiling is now run-scoped, and the digest constants must be recomputed.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs`. Follow the nearest existing test in that module for how to build a `LoopExecutionState` with a given observed prompt size and a `LoopRunContext`; reuse its helpers rather than inventing new ones.

```rust
    #[test]
    fn run_context_budget_overrides_the_strategy_default_for_compaction() {
        // The strategy's own budget would not trigger, but the run's model
        // has a far smaller real window, so this prompt is already over.
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
            preserve_tail_tokens: 10,
            deadline_ms: 30_000,
        };
        let ctx = test_run_context("compaction-budget-override")
            .with_resolved_context_budget(PromptContextTokenBudget::new(40_000, 5_000, 0));
        let state = state_with_observed_prompt_tokens(50_000, &ctx);

        assert!(matches!(
            strategy.should_compact(&state, &ctx),
            CompactionDecision::Trigger { .. }
        ));
    }

    #[test]
    fn absent_run_context_budget_falls_back_to_the_strategy_default() {
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
            preserve_tail_tokens: 10,
            deadline_ms: 30_000,
        };
        let ctx = test_run_context("compaction-budget-fallback");
        let state = state_with_observed_prompt_tokens(50_000, &ctx);

        assert_eq!(
            strategy.should_compact(&state, &ctx),
            CompactionDecision::Skip
        );
    }
```

Add the equivalent pair to `active_task_compaction.rs`'s test module against `ActiveTaskPreservingCompactionStrategy`, so the wrapper is proven too and not just the base:

```rust
    #[test]
    fn active_task_strategy_honors_the_run_context_budget() {
        let strategy = ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(128_000, 20_000, 0),
            preserve_tail_tokens: 10,
            deadline_ms: 30_000,
        });
        let ctx = test_run_context("active-task-budget-override")
            .with_resolved_context_budget(PromptContextTokenBudget::new(40_000, 5_000, 0));
        let state = state_with_observed_prompt_tokens(50_000, &ctx);

        assert!(matches!(
            strategy.should_compact(&state, &ctx),
            CompactionDecision::Trigger { .. }
        ));
    }
```

`state_with_observed_prompt_tokens` does not exist yet. Write it in the test module using the real helpers the neighbouring test at `compaction.rs:398-416` already uses — `CompactionPromptSnapshot::from_message_index` derives `observed_prompt_tokens` as the sum of the entries' `estimated_tokens` (`state/compaction.rs:129-138`), so one entry carrying the whole figure is enough:

```rust
    fn state_with_observed_prompt_tokens(
        tokens: u64,
        context: &LoopRunContext,
    ) -> LoopExecutionState {
        let mut state = LoopExecutionState::initial_for_run(context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: tokens,
            }]);
        state
    }
```

Build the context with `crate::test_support::test_run_context("<label>")`, exactly as `compaction.rs:399` does. Because the helper needs the context, construct the context first in each test and pass it in. A single-entry index is enough to trip `can_evaluate`; if a test needs a real compaction *boundary* rather than just the threshold, add further `MessageIndexEntry` values with ascending `sequence` following the same idiom.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p ironclaw_agent_loop compaction`
Expected: FAIL — `run_context_budget_overrides_the_strategy_default_for_compaction` returns `Skip` because the run-context budget is still ignored.

- [ ] **Step 3: Read the budget from the run context**

In `crates/loop/ironclaw_agent_loop/src/strategies/compaction.rs`, add to `impl DefaultCompactionStrategy`:

```rust
    /// The budget this run actually runs with: the one resolved from the
    /// run's model when present, otherwise this strategy's compiled-in
    /// default.
    pub(super) fn effective_budget(&self, ctx: &LoopRunContext) -> PromptContextTokenBudget {
        ctx.resolved_context_budget
            .unwrap_or(self.prompt_context_budget)
    }
```

In `DefaultCompactionStrategy::should_compact`, rename `_ctx` to `ctx`, resolve once at the top, and pass it to the four call sites Task 3 parameterized:

```rust
    fn should_compact(
        &self,
        state: &LoopExecutionState,
        ctx: &LoopRunContext,
    ) -> CompactionDecision {
        let budget = self.effective_budget(ctx);
        if !self.can_evaluate(state, budget) {
            return CompactionDecision::Skip;
        }
```

and replace each `self.trigger_at(state, self.prompt_context_budget, sequence)` with `self.trigger_at(state, budget, sequence)`.

- [ ] **Step 4: Do the same in the wrapper**

In `crates/loop/ironclaw_agent_loop/src/strategies/active_task_compaction.rs`, rename `_ctx` to `ctx` in `should_compact`, and at the top:

```rust
        let budget = self.base.effective_budget(ctx);
        if !self.base.can_evaluate(state, budget) {
            return CompactionDecision::Skip;
        }
```

Replace both `self.base.trigger_at(state, self.base.prompt_context_budget, sequence)` calls with `self.base.trigger_at(state, budget, sequence)`.

- [ ] **Step 5: Update the four replay fingerprints**

The compaction ceiling is no longer a fixed property of the family, so the fingerprint must stop claiming a number. In `crates/loop/ironclaw_agent_loop/src/families/mod.rs:35`, `families/subagent.rs:20`, and `families/unbound.rs:24` and `:45`, change:

```
compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,
```

to:

```
compaction:ActiveTaskPreservingCompactionStrategy(context_limit=run_context,reserve=run_context,
```

Leave the rest of each string (`preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3`) exactly as it is — those knobs really are still family properties.

- [ ] **Step 6: Recompute the four digest constants**

Run: `cargo test -p ironclaw_agent_loop families`
Expected: FAIL. Four tests assert a hand-written `ComponentDigest` byte array against the BLAKE3 of the fingerprint (`families/mod.rs:164-173` and its siblings in `subagent.rs` / `unbound.rs`). Each failure prints both the expected and actual arrays. Copy the **actual** array into the matching constant:
- `DEFAULT_FAMILY_DIGEST` — `families/mod.rs:53-56`
- `SUBAGENT_FAMILY_DIGEST` — `families/subagent.rs`
- `UNBOUND_DEFAULT_FAMILY_DIGEST` and `UNBOUND_STRUCTURED_FAMILY_DIGEST` — `families/unbound.rs`

Do not hand-compute these. Take them from the test output.

- [ ] **Step 7: Run the full crate suite**

Run: `cargo test -p ironclaw_agent_loop`
Expected: PASS, including the new tests and all four digest assertions.

- [ ] **Step 8: Verify clippy is clean**

Run: `cargo clippy -p ironclaw_agent_loop --tests -- -D warnings`
Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/loop/ironclaw_agent_loop/src/strategies/ crates/loop/ironclaw_agent_loop/src/families/
git commit -m "feat(agent-loop): honor a run-resolved context budget when compacting

The compaction ceiling is no longer a fixed family property, so the
replay fingerprints say run_context and the four digests are recomputed."
```

---

### Task 5: Ask the gateway what the model's window is

**Files:**
- Modify: `crates/loop/ironclaw_loop_host/src/lib.rs:2293` (the `HostManagedModelGateway` trait)
- Modify: `crates/loop/ironclaw_loop_host/src/model_gateway.rs:405-426` (the `LlmProviderModelGateway` impl block)
- Test: `crates/loop/ironclaw_loop_host/src/model_gateway.rs` test module

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `HostManagedModelGateway::advertised_context_window_tokens(&self, model_profile_id: &ModelProfileId, resolved_model_route: Option<&HostManagedModelRouteSnapshot>) -> Option<u64>`, an `async` trait method defaulting to `None`. **Task 7 Step 3 is its only caller** — Task 6 threads a budget field through the gateway parts and never calls this method.

**Context you need:** `HostManagedModelGateway` is the loop's abstraction over an LLM provider; `ironclaw_loop_host` is the one crate in the `loop/` family chartered to name `ironclaw_llm` at all (`crates/loop/AGENTS.md`). The trait already has a defaulted, route-keyed, best-effort lookup with exactly this shape — `diagnostic_effective_model` (`lib.rs:2297`), overridden by `LlmProviderModelGateway` at `model_gateway.rs:410`. Copy that shape. `HostManagedModelRouteSnapshot` is a type alias for `ironclaw_loop_contracts::LoopModelRouteSnapshot` (`lib.rs:2696`). The provider's `model_metadata()` returns `ModelMetadata { id, context_length: Option<u32> }` — the field that has no consumer outside `ironclaw_llm` today.

- [ ] **Step 1: Write the failing tests**

**There is no shared fake provider in this crate — do not go looking for one.** The convention in `model_gateway.rs`'s `mod tests` is one bespoke, single-purpose double per test; `StopSequenceRecordingProvider` (`:2852-2891`) is the model to copy. It implements exactly four `LlmProvider` methods — `model_name`, `cost_per_token`, `complete`, `complete_with_tools` — and `unreachable!()`s the ones its test never touches. Write a double in that shape:

```rust
    struct WindowReportingProvider {
        model_id: String,
        context_length: Option<u32>,
    }

    #[async_trait]
    impl LlmProvider for WindowReportingProvider {
        fn model_name(&self) -> &str {
            &self.model_id
        }

        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            Default::default()
        }

        async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
            Ok(ModelMetadata {
                id: self.model_id.clone(),
                context_length: self.context_length,
            })
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            unreachable!("the advertised-window tests never dispatch a completion")
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unreachable!("the advertised-window tests have no tool surface")
        }
    }
```

Build the policy the way production does (`ironclaw_composition/src/model_gateway_assembly.rs:137`): `LlmModelProfilePolicy::new().allow_model_profile(profile_id, None)`.

```rust
    fn window_test_profile_id() -> ModelProfileId {
        ModelProfileId::new("interactive_model").expect("valid profile id")
    }

    #[tokio::test]
    async fn gateway_reports_the_providers_advertised_context_window() {
        let provider = Arc::new(WindowReportingProvider {
            model_id: "base-model".to_string(),
            context_length: Some(200_000),
        });
        let policy = LlmModelProfilePolicy::new()
            .allow_model_profile(window_test_profile_id(), None);
        let gateway = LlmProviderModelGateway::new(provider, policy);

        let window = gateway
            .advertised_context_window_tokens(&window_test_profile_id(), None)
            .await;

        assert_eq!(window, Some(200_000));
    }

    #[tokio::test]
    async fn gateway_reports_none_when_the_provider_advertises_nothing() {
        let provider = Arc::new(WindowReportingProvider {
            model_id: "base-model".to_string(),
            context_length: None,
        });
        let policy = LlmModelProfilePolicy::new()
            .allow_model_profile(window_test_profile_id(), None);
        let gateway = LlmProviderModelGateway::new(provider, policy);

        let window = gateway
            .advertised_context_window_tokens(&window_test_profile_id(), None)
            .await;

        assert_eq!(window, None);
    }

    #[tokio::test]
    async fn gateway_reports_none_when_the_route_overrides_to_another_model() {
        // The provider describes "base-model" but the policy routes this
        // profile to a different one. Budgeting from the base model's window
        // could hand a small model a budget sized for a large one.
        let provider = Arc::new(WindowReportingProvider {
            model_id: "base-model".to_string(),
            context_length: Some(200_000),
        });
        let policy = LlmModelProfilePolicy::new().allow_model_profile(
            window_test_profile_id(),
            Some("other-model".to_string()),
        );
        let gateway = LlmProviderModelGateway::new(provider, policy);

        let window = gateway
            .advertised_context_window_tokens(&window_test_profile_id(), None)
            .await;

        assert_eq!(
            window, None,
            "a window for a different model must not be trusted"
        );
    }
```

The third test drives the mismatch through `model_override` on the route, which `request_model_override` prefers over `provider.active_model_name()` (`model_gateway.rs:1235-1239`) — no `LoopModelRouteSnapshot` needed. Import `ModelMetadata` from `ironclaw_llm` in the test module if it is not already there.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p ironclaw_loop_host gateway_reports`
Expected: FAIL to compile — `no method named 'advertised_context_window_tokens'`. (All three test names share the `gateway_reports` prefix, so this filter selects the whole set; a compile failure blocks the crate regardless, but the filter should still name what you mean.)

- [ ] **Step 3: Add the defaulted trait method**

In `crates/loop/ironclaw_loop_host/src/lib.rs`, inside `#[async_trait] pub trait HostManagedModelGateway` (after `diagnostic_effective_model`, which ends at `:2307`):

```rust
    /// Best-effort provider-advertised total input context window, in tokens,
    /// for the route this run will use.
    ///
    /// Gateways that own provider selection should override this. The default
    /// returns `None`, which keeps the compiled-in budget — a gateway that
    /// knows nothing must not change how any run is budgeted.
    async fn advertised_context_window_tokens(
        &self,
        _model_profile_id: &ModelProfileId,
        _resolved_model_route: Option<&HostManagedModelRouteSnapshot>,
    ) -> Option<u64> {
        None
    }
```

- [ ] **Step 4: Override it in the provider-backed gateway**

In `crates/loop/ironclaw_loop_host/src/model_gateway.rs`, inside `impl<P> HostManagedModelGateway for LlmProviderModelGateway<P>` (after `diagnostic_effective_model`, which ends at `:426`):

```rust
    async fn advertised_context_window_tokens(
        &self,
        model_profile_id: &ModelProfileId,
        resolved_model_route: Option<&HostManagedModelRouteSnapshot>,
    ) -> Option<u64> {
        // Advisory only: a provider that cannot report a window for the model
        // this run will actually be served must leave the run on the
        // compiled-in budget, never fail the run.
        let metadata = self.provider.model_metadata().await.ok()?;
        // `model_metadata()` takes no model argument — it describes whatever
        // model the provider was configured with. The served model is resolved
        // per request, so verify they agree before trusting the window.
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

**Why the identity check, and why it is not optional.** `request_model_override`
(`model_gateway.rs:1221-1248`) resolves the served model as *route-requested →
route override → `provider.active_model_name()`*, and its comment records that
"providers that honor per-request overrides (e.g. NEAR AI) serve the requested
model." Without the check, a run whose advisory route overrides the model would
be budgeted from the *base* model's window. Borrowing a larger model's window is
precisely the provider rejection this work exists to prevent. `route_for` and
`request_model_override` are the same helpers `diagnostic_effective_model` uses
at `:410-426` — copy that call shape, not just the signature.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p ironclaw_loop_host`
Expected: PASS. Any other `HostManagedModelGateway` implementor in the workspace keeps compiling — the method is defaulted, so no test double needs updating.

- [ ] **Step 6: Verify clippy is clean**

Run: `cargo clippy -p ironclaw_loop_host --tests -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/loop/ironclaw_loop_host/src/lib.rs crates/loop/ironclaw_loop_host/src/model_gateway.rs
git commit -m "feat(loop-host): expose the provider-advertised context window on the gateway"
```

---

### Task 6: Thread a budget through the model gateway to the port that sizes requests (STRUCTURAL)

**This task must not change behavior.** It gives `ThreadResolvingLoopModelGateway` a budget field and passes it down to `ThreadBackedLoopModelPort`, with every call site supplying `self.config.prompt_context_budget` — which today *is* `PromptContextTokenBudget::default()`, the exact value the port already defaults to. Task 7 then changes that value everywhere at once.

Without this task the plan ships the split it exists to prevent: compaction would fire at the model's real ceiling while the outbound request is still packed to 128k.

**Files:**
- Modify: `crates/loop/ironclaw_loop_host/src/thread_resolving_model_gateway.rs:27-44` (`Parts`), `:54-71` (struct), `:78-105` (`new`), `:135-142` (`stream_model_inner`)
- Modify: `crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs:1957-1998` (both construction arms)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `ThreadResolvingLoopModelGatewayParts.prompt_context_budget: PromptContextTokenBudget` (a thirteenth field), carried onto `ThreadResolvingLoopModelGateway` and applied to the model port. Task 7 supplies the resolved value here.

**Context you need:** `ThreadResolvingLoopModelGateway` is what composition actually wires — `loop_driver_host.rs:1957-1998` builds it in both arms of a `resolve_for_scope` match, inside the same `build_text_only_host_with_capabilities` Task 7 edits. Its `stream_model_inner` (`thread_resolving_model_gateway.rs:135-142`) constructs `ThreadBackedLoopModelPort::new(..).with_prompt_bundle_authority(..)`, and that port's `resolve_model_messages` (`lib.rs:2010-2021`) calls `select_prompt_context_messages(context.messages, self.prompt_context_budget, ..)` — **the call that decides which transcript messages actually reach the provider.** The builder needed already exists at `lib.rs:1592` (`impl ThreadBackedLoopModelPort`, block at `:1519`) and is currently called only from tests.

Do not confuse this with `ThreadBackedLoopModelGateway` (`model_gateway.rs:196`), a different type instantiated only by `ironclaw_loop_host/tests/llm_gateway.rs`. The spec's "Untouched" section explains why that one is deliberately left alone.

**Expect one user-visible side effect.** `lib.rs:1793` (`context_limit: self.prompt_context_budget.context_limit_tokens`) sits in this same port and feeds the WebUI inspector's prompt diagnostic, which today always reports `128000`. Once Task 7 supplies a resolved value, the inspector shows the model's real window. That is the intended improvement, not a regression — but it means the fixtures pinning `context_limit: 128_000` (`ironclaw_assistant/src/inspector_store.rs:2451`, `ironclaw_webui/frontend/src/pages/chat/inspector/inspector-panel.test.tsx:75`) are describing a value that is no longer universal. They are fixtures, so they keep passing; leave them alone unless a test asserts the number is always 128k.

- [ ] **Step 1: Write the failing test**

**None of the helpers below exist — you are writing them.** Run `rg -n "ThreadResolvingLoopModelGateway" crates/loop/ironclaw_loop_host/` to find the module that already exercises this gateway and follow its construction; if none does, build `ThreadResolvingLoopModelGatewayParts` field by field (twelve fields today, thirteen after Step 3). To read back the request the provider actually received, write a bespoke recording provider in the shape of `StopSequenceRecordingProvider` (`model_gateway.rs:2852-2891`), which stores requests in a `Mutex<Vec<CompletionRequest>>`.

Pin that a budget handed to the gateway reaches message selection:

```rust
    #[tokio::test]
    async fn gateway_applies_its_prompt_context_budget_to_message_selection() {
        // A budget this small admits only the newest message; with the
        // default 128k budget every message would be selected.
        let parts = test_gateway_parts()
            .with_prompt_context_budget(PromptContextTokenBudget::new(4, 0, 0));
        let gateway = ThreadResolvingLoopModelGateway::new(parts);

        let response = gateway
            .stream_model(test_request_with_transcript_messages(5))
            .await
            .expect("model call succeeds");

        assert_eq!(
            captured_request_message_count(&response),
            1,
            "selection must respect the gateway's budget, not the port default"
        );
    }
```

`test_gateway_parts`, `test_request_with_transcript_messages` and `captured_request_message_count` are names for helpers **you must write** — none exists. The assertion that matters is **the number of transcript messages in the request the provider received**, not the reply text.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p ironclaw_loop_host gateway_applies_its_prompt_context_budget`
Expected: FAIL to compile — `Parts` has no `prompt_context_budget` field.

- [ ] **Step 3: Add the field to `Parts` and the gateway**

In `crates/loop/ironclaw_loop_host/src/thread_resolving_model_gateway.rs`, add to `ThreadResolvingLoopModelGatewayParts` (`:27-44`, after `prompt_diagnostic_sink`):

```rust
    pub prompt_context_budget: PromptContextTokenBudget,
```

Add the identical field to `ThreadResolvingLoopModelGateway` (`:54-71`), and thread it through `new` (`:78-105`) — add `prompt_context_budget` to both the destructuring `let ThreadResolvingLoopModelGatewayParts { .. } = parts;` and the `Self { .. }` literal. Import `PromptContextTokenBudget` from `ironclaw_loop_contracts` if not already in scope.

- [ ] **Step 4: Apply it to the model port**

In `stream_model_inner` (`:135-142`), extend the builder chain:

```rust
        let mut model_port = ThreadBackedLoopModelPort::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            request.context,
            Arc::clone(&self.host_gateway),
            self.max_messages,
        )
        .with_prompt_bundle_authority(self.prompt_authority.clone())
        .with_prompt_context_token_budget(self.prompt_context_budget);
```

- [ ] **Step 5: Supply the existing default at the call sites**

In `crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs`, both arms of the gateway construction (`:1957-1998`) build `ThreadResolvingLoopModelGatewayParts { .. }`. Add to **each**:

```rust
                        prompt_context_budget,
```

`prompt_context_budget` is the local already bound at `:1604` from `self.config.prompt_context_budget`. Since that is `PromptContextTokenBudget::default()` today and the port also defaulted to it, behavior is unchanged — which is what makes this commit structural.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p ironclaw_loop_host && cargo test -p ironclaw_turn_runner`
Expected: PASS, including the new test. **No pre-existing test should change**; if one does, the value you passed differs from the port's old default — fix that rather than editing the test.

- [ ] **Step 7: Verify clippy is clean**

Run: `cargo clippy -p ironclaw_loop_host -p ironclaw_turn_runner --tests -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/loop/ironclaw_loop_host/src/thread_resolving_model_gateway.rs \
        crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs
git commit -m "refactor(loop-host): thread a prompt context budget to the model port

Structural only. Every call site passes the config default the port
already used, so request sizing is unchanged."
```

---

### Task 7: Resolve the budget once per run and wire every consumer

The task that makes the feature live.

**Files:**
- Modify: `crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs:1591-1660` (inside `build_text_only_host_with_capabilities`)
- Test: `crates/loop/ironclaw_turn_runner/` test module for the driver host

**Interfaces:**
- Consumes: `PromptContextTokenBudget::from_advertised_window` (Task 1), `LoopRunContext::with_resolved_context_budget` (Task 2), `HostManagedModelGateway::advertised_context_window_tokens` (Task 5).
- Produces: a `LoopRunContext` carrying a model-derived budget, plus every production consumer configured with the same value.

**Context you need:** `build_text_only_host_with_capabilities` (`:1591`) is where a run's host is assembled. At `:1604` it already binds `let prompt_context_budget = self.config.prompt_context_budget;` (today always the 128k default), and at `:1606` it resolves the model route via `attach_model_route_snapshot`. The gateway is `self.model_gateway: Arc<G>` (`:1038`). The run's model profile is `run_context.resolved_run_profile.model_profile_id`.

**All four consumers must end up on the same value.** This is the whole point of the change; wiring a subset produces a loop that *selects* messages against one ceiling while *compacting* against another. Three of the four already flow from the single `prompt_context_budget` local you redefine in Step 3, because there is no function boundary anywhere between `:1591` and `:2025`:

| Consumer | How it receives the value | Verified in |
|---|---|---|
| Compaction strategy (`DefaultCompactionStrategy`) | reads `ctx.resolved_context_budget` off `LoopRunContext` | Step 1 test + Task 4 |
| `ThreadBackedLoopContextPort` (prompt context) | Step 4's explicit builder call at `:1647` | Step 1 test |
| `ThreadBackedLoopModelPort` (**sizes the outbound request**) | via `ThreadResolvingLoopModelGatewayParts.prompt_context_budget`, which Task 6 already reads from this same local at `:1957-1998` | Step 6 |
| Structured finalization | `:2018` reads the same local directly | Step 7 |

If Task 6 has not landed, stop — without it the third row is impossible and the feature ships broken.

- [ ] **Step 1: Write the failing test**

In the driver-host test module, using the crate's existing host-construction test helpers:

```rust
    #[tokio::test]
    async fn resolved_budget_reaches_the_run_context_when_the_gateway_advertises_a_window() {
        let factory = test_host_factory_with_advertised_window(Some(40_000));
        let request = test_host_request();

        let host = factory
            .build_text_only_host_with_capabilities(request, test_capabilities())
            .await
            .expect("host builds");

        assert_eq!(
            host.run_context().resolved_context_budget,
            Some(PromptContextTokenBudget::from_advertised_window(Some(40_000)))
        );
    }

    #[tokio::test]
    async fn run_context_carries_no_budget_when_the_gateway_advertises_nothing() {
        let factory = test_host_factory_with_advertised_window(None);
        let request = test_host_request();

        let host = factory
            .build_text_only_host_with_capabilities(request, test_capabilities())
            .await
            .expect("host builds");

        assert_eq!(host.run_context().resolved_context_budget, None);
    }
```

**There is no host-factory helper.** `test_host_factory_with_advertised_window`, `test_host_request` and `test_capabilities` are names for code **you must write** — `loop_driver_host.rs`'s own `mod tests` never calls `build_text_only_host_with_capabilities` at all. Two sibling files already hand-roll exactly this construction; copy the closer one rather than inventing a third shape:

- `crates/loop/ironclaw_turn_runner/src/loop_driver_host/run_lease_fence_tests.rs:102-124` — `RebornLoopDriverHostFactory::new(thread_service, thread_scope, Arc::new(UnusedGateway), …, TextOnlyLoopHostConfig { max_messages: 8, prompt_context_budget: Default::default(), require_model_route_snapshot: false }, InstructionSafetyContext::non_production_noop())`, then `.build_text_only_host_with_capabilities(RebornLoopDriverHostRequest { claimed_run, loop_run_context }, Arc::new(EmptyLoopCapabilityPort))`.
- `crates/loop/ironclaw_turn_runner/src/loop_driver_host/compaction_tests.rs:152-167` — the same shape, and the **closer** model here because it already defines gateway doubles (`RecordingScopedGateway` at `:43`, `LoudFallbackGateway` at `:65`) that override gateway trait methods.

Write your own double implementing `HostManagedModelGateway::advertised_context_window_tokens` to return the configured value, modeled on `compaction_tests.rs:43-90`. If the constructed host exposes no `run_context()` accessor, assert at whichever seam those two files use, and say so in the test name.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p ironclaw_turn_runner resolved_budget`
Expected: FAIL — `resolved_context_budget` is `None` because nothing resolves it yet.

- [ ] **Step 3: Resolve the budget after the route is attached**

In `crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs`, replace the two lines at `:1604-1606`:

```rust
        let prompt_context_budget = self.config.prompt_context_budget;
        let run_context = self.attach_model_route_snapshot(request.loop_run_context)?;
```

with:

```rust
        let run_context = self.attach_model_route_snapshot(request.loop_run_context)?;
        // Resolve the scope-specific gateway ONCE, here, and reuse the same
        // object below at :1959. (This is NOT the `attach_model_route_snapshot`
        // "already present" idiom -- see the lifetime note below.) Asking `self.model_gateway` for the window
        // while the run is served by a `resolve_for_scope` override would let
        // the budget describe a different gateway than the one issuing the
        // request. (Production overrides none — `resolve_for_scope` defaults
        // to `None`, `lib.rs:2343` — but a test harness that does would get a
        // silently mismatched budget.)
        let scoped_gateway = self.model_gateway.resolve_for_scope(&run_context.scope);
        // A caller that already supplied a budget is authoritative.
        let run_context = if run_context.resolved_context_budget.is_some() {
            run_context
        } else {
            // Ask the run's model how much context it really holds. A gateway
            // that cannot say leaves the run on the configured default, which
            // is exactly today's behavior.
            let profile_id = &run_context.resolved_run_profile.model_profile_id;
            let route = run_context.resolved_model_route.as_ref();
            let advertised = match scoped_gateway.as_ref() {
                Some(gateway) => {
                    gateway
                        .advertised_context_window_tokens(profile_id, route)
                        .await
                }
                None => {
                    self.model_gateway
                        .advertised_context_window_tokens(profile_id, route)
                        .await
                }
            };
            match advertised {
                Some(window) => run_context.with_resolved_context_budget(
                    PromptContextTokenBudget::from_advertised_window(Some(window)),
                ),
                None => run_context,
            }
        };
        let prompt_context_budget = run_context
            .resolved_context_budget
            .unwrap_or(self.config.prompt_context_budget);
```

Then at `:1959`, replace `if let Some(gw) = self.model_gateway.resolve_for_scope(&run_context.scope)` with `if let Some(gw) = scoped_gateway` so the same resolution serves both — and `resolve_for_scope` is called once per build instead of twice.

**Ordering matters twice here.** First, `prompt_context_budget` is now derived *after* `run_context`, because it depends on the resolved route. Second — and easy to miss — the lines immediately below this insertion point deliberately kick off three prefetches in parallel with host setup: `communication_fetch` (`:1608`, "Kick off advisory communication-context fetches"), and the `tokio::spawn`s for `user_profile_fetch` and `cancellation_handle_fetch` (`:1631`, "Build the live cancellation handle in parallel with the rest of host setup"). A blocking `.await` placed *ahead* of them delays all three.

Today that costs nothing — the only provider populating `context_length` reads a static table (`gemini_oauth.rs:136-155`), so the future resolves immediately. But the spec's follow-on slice makes this call provider-latency-bearing. So put the gateway query **after** the three prefetch kickoffs and `.await` it just before `prompt_context_budget` is first needed (the context-port build at `:1647`), rather than at `:1606`. Bind `scoped_gateway` early — it is a synchronous call — and defer only the `.await`.

```rust
// ponytail: the window lookup is awaited after the prefetch kickoffs so it
// cannot serialize them. If a provider ever makes model_metadata() do real
// I/O, promote it to a tokio::spawn alongside user_profile_fetch instead.
``` Deriving it *from* `run_context.resolved_context_budget` (rather than from a separate `match`) is what guarantees the field and the local can never disagree. Import `PromptContextTokenBudget` in this file if it is not already in scope.

**On lifetime — and what the guard does and does not do.** `create_host` (`:2714-2736`) rebuilds `LoopRunContext` from the durable `TurnRunState` on every claim, and `TurnRunState` (`ironclaw_turns/src/status.rs:180-203`) has no budget field. So on every production claim the `is_some()` branch is **false** and the budget is resolved fresh. The guard is not a resume-stability mechanism — it exists only so a caller that constructs a request with a budget already set (test harnesses, and `build_text_only_host_with_capabilities`'s direct callers) is not overridden. Do not describe it as resolve-once-per-run, and **do not justify it by analogy to `attach_model_route_snapshot`'s "already present" branch** — that branch is live because `create_host` carries `resolved_model_route` forward from `TurnRunState` (`:2731-2733`), and no equivalent carry exists here. The shapes rhyme; the guarantees do not.

Re-deriving per claim is the accepted design, for the reason in the spec's "Lifetime of the resolved value" section. Do not add a field to `TurnRunState` as part of this plan.

- [ ] **Step 4: Give the context port the same budget**

At `:1647`, the context adapter is built and then `.with_context_window_cache(...)` is chained. Add the budget to that chain:

```rust
        let mut context_adapter = ThreadBackedLoopContextPort::new(
            Arc::clone(&self.thread_service),
            effective_scope.clone(),
            run_context.clone(),
            max_messages,
        )
        .with_context_window_cache(Arc::clone(&context_window_cache))
        .with_prompt_context_token_budget(prompt_context_budget);
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p ironclaw_turn_runner`
Expected: PASS, including both new tests and the pre-existing driver-host suite.

- [ ] **Step 6: Verify the model port — the request-sizing consumer — got the resolved value**

Run: `sed -n '1957,1998p' crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs | grep -n prompt_context_budget`
Expected: **two** hits, one per construction arm, each reading the bare local `prompt_context_budget` you redefined in Step 3 (added by Task 6, not by this task). If either arm is missing it, or either reads `self.config.prompt_context_budget`, fix it now — this is the consumer that decides how large the outbound request is, and a miss here is the exact split this plan exists to prevent.

- [ ] **Step 7: Verify structured finalization inherited the same budget**

Run: `sed -n '2010,2022p' crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs`
Expected: `token_budget: prompt_context_budget,` — reading the local you redefined in Step 3, not `self.config.prompt_context_budget`. If it reads the config field instead, change it to the local so all four consumers agree.

- [ ] **Step 8: Verify clippy is clean**

Run: `cargo clippy -p ironclaw_turn_runner --tests -- -D warnings`
Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/loop/ironclaw_turn_runner/src/loop_driver_host.rs
git commit -m "feat(turn-runner): resolve the prompt context budget from the run's model"
```

---

### Task 8: Teach the SDK-seam fake to advertise a window

The integration tier uses the **real** `LlmProviderModelGateway` and the real `ironclaw_llm` decorator chain, faking only the vendor-SDK seam (`tests/integration/AGENTS.md`). Its rule 3 is explicit: *"Mock only at the SDK seam. Use `RebornScriptedReply`; do not swap the gateway or stub internals."* So the window must be advertised by the scripted provider, not by a substituted gateway.

`TraceLlm` — the SDK-seam fake — does not implement `model_metadata()` today, so it inherits the trait default and reports `None`. Until that changes, no integration test can exercise this feature at all.

**Files:**
- Modify: `tests/support/trace_llm.rs:280-293` (struct), `:511-524` (`from_trace`), `:893` (`impl LlmProvider for TraceLlm`)
- Modify: `tests/integration/support/builder.rs:213` (beside `script`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `TraceLlm::with_advertised_context_window(self, tokens: u32) -> Self`, a `model_metadata()` implementation on `TraceLlm`, and a harness builder method `advertised_context_window(self, tokens: u32) -> Self`. Task 9 uses the builder method.

**Context you need:** `TraceLlm::from_trace(trace: LlmTrace) -> Self` (`:511`) is the only constructor; `from_file` delegates to it. The struct's fields are all private. The harness builder retains the concrete `Arc<TraceLlm>` as `scripted_llm` (`builder.rs:854`) before upcasting it, so the setter must be applied to the `TraceLlm` *before* it is wrapped in an `Arc`.

- [ ] **Step 1: Write the failing test**

In the test module at the bottom of `tests/support/trace_llm.rs` (there is an existing one — see the `TraceLlm::from_trace(LlmTrace { .. })` construction at `:1055` for the trace-building idiom):

```rust
    #[tokio::test]
    async fn trace_llm_advertises_no_context_window_by_default() {
        let provider = TraceLlm::from_trace(LlmTrace {
            model_name: "test-model".to_string(),
            turns: Vec::new(),
        });

        let metadata = provider.model_metadata().await.expect("metadata");

        assert_eq!(metadata.context_length, None);
    }

    #[tokio::test]
    async fn trace_llm_advertises_a_configured_context_window() {
        let provider = TraceLlm::from_trace(LlmTrace {
            model_name: "test-model".to_string(),
            turns: Vec::new(),
        })
        .with_advertised_context_window(40_000);

        let metadata = provider.model_metadata().await.expect("metadata");

        assert_eq!(metadata.context_length, Some(40_000));
        assert_eq!(metadata.id, "test-model");
    }
```

Match the `LlmTrace` construction to whatever the existing helper at `:1055` uses — if `LlmTrace` has more fields, copy that call rather than the literal above.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test trace_llm_tests advertises`
Expected: FAIL to compile — `no method named 'with_advertised_context_window'`. If the test module lives in a different binary, use the command that runs `tests/support/trace_llm.rs`'s own tests, per how `tests/trace_llm_tests.rs` is wired.

- [ ] **Step 3: Add the field and setter**

In `tests/support/trace_llm.rs`, add to `pub struct TraceLlm` (after `captured_calls` at `:292`):

```rust
    /// Provider-advertised total context window, in tokens. `None` — the
    /// default — reports no window, exactly like a provider that does not
    /// implement `model_metadata`.
    advertised_context_window: Option<u32>,
```

Add `advertised_context_window: None,` to the struct literal in `from_trace` (`:515-522`).

Add to `impl TraceLlm` (`:511`):

```rust
    /// Make this fake advertise a total context window, as a real provider
    /// that populates `ModelMetadata::context_length` would.
    pub fn with_advertised_context_window(mut self, tokens: u32) -> Self {
        self.advertised_context_window = Some(tokens);
        self
    }
```

- [ ] **Step 4: Implement `model_metadata`**

In `impl LlmProvider for TraceLlm` (`:893`), after `model_name` (`:894`):

```rust
    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.model_name.clone(),
            context_length: self.advertised_context_window,
        })
    }
```

Add `ModelMetadata` to the `ironclaw_llm` import list at the top of the file if it is not already there.

- [ ] **Step 5: Expose it on the harness builder**

In `tests/integration/support/builder.rs`, add beside `script` (`:213`):

```rust
    /// Make the scripted model advertise a total context window, so the run's
    /// prompt context budget is derived from it instead of the compiled-in
    /// default. Unset — the default — advertises nothing.
    pub fn advertised_context_window(mut self, tokens: u32) -> Self {
        self.advertised_context_window = Some(tokens);
        self
    }
```

Add a matching `advertised_context_window: Option<u32>` field to the builder struct and `None` to its `Default`/constructor. Where the builder constructs the `TraceLlm` it retains as `scripted_llm` (`:854`), apply the setter before the `Arc::new`:

```rust
        let mut scripted = TraceLlm::from_trace(trace);
        if let Some(tokens) = self.advertised_context_window {
            scripted = scripted.with_advertised_context_window(tokens);
        }
        let scripted_llm = Arc::new(scripted);
```

Match the surrounding construction — if the builder already binds the `TraceLlm` to a name before wrapping it, thread the setter into that expression instead of restructuring it.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --test trace_llm_tests`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add tests/support/trace_llm.rs tests/integration/support/builder.rs
git commit -m "test(support): let the scripted model advertise a context window"
```

---

### Task 9: Prove it through a real turn

Crate-tier tests prove each piece; this proves the production chain. Per `.claude/rules/testing.md`, production-wired behavior ships with an integration test asserting at a seam — never `wait_for_status(Completed)` alone.

**Files:**
- Create or extend: a scenario under `tests/integration/`
- Modify: `tests/AGENTS.md` (binding — see Step 5)

**Interfaces:**
- Consumes: everything from Tasks 1-8, especially the builder's `advertised_context_window`.
- Produces: no production code, one regression test.

**Context you need:** The behavior to pin is that a run whose model advertises a *small* window compacts earlier than the 128k default would. That is what is broken today, and it is why a small-window model can never converge: `executor/model.rs:475-481` responds to a provider overflow by forcing compaction against a ceiling the prompt already satisfies, so the retry shrinks nothing and the run aborts on the second overflow (`recovery.rs:489-505`).

The harness shape is always `build → submit_turn → assert`, ~3-12 lines, no nested structs in the body. Script one entry per model call: a plain reply turn is one `RebornScriptedReply::text(..)`.

- [ ] **Step 1: Find the nearest existing scenario**

Run: `rg -ln "compaction" tests/integration/`
Known hits include `tests/integration/model_recovery.rs`. **Extend the nearest existing compaction scenario if one fits**; only add a new file if none does, and say why in the PR.

- [ ] **Step 2: Write the failing test**

Two assertions are required, and the second is the one that matters. A compaction-only assertion passes while the request-sizing consumer is still on the default — the exact split this plan exists to prevent.

**Assertion A — the request actually shrinks.** `TraceLlm::captured_requests() -> Vec<Vec<ChatMessage>>` (`tests/support/trace_llm.rs:548`) records the messages of every model call, and the harness already reads it throughout `tests/integration/support/assertions.rs` (e.g. `:587`, `:629`) via `self.scripted_llm.captured_requests()`. Compare the same transcript sent under a small advertised window against one sent with none:

```rust
#[tokio::test]
async fn small_advertised_window_shrinks_the_outbound_request() {
    let narrow = RebornIntegrationHarness::test_default()
        .advertised_context_window(40_000)
        .script([RebornScriptedReply::text("done")])
        .build().await.expect("harness builds");
    narrow.submit_turn(&large_transcript_prompt()).await.expect("turn completes");

    let wide = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("done")])
        .build().await.expect("harness builds");
    wide.submit_turn(&large_transcript_prompt()).await.expect("turn completes");

    let narrow_messages = narrow.captured_request_messages(0);
    let wide_messages = wide.captured_request_messages(0);
    assert!(
        narrow_messages < wide_messages,
        "a 40k-window model must be sent fewer transcript messages than an \
         unadvertised one; got {narrow_messages} vs {wide_messages}"
    );
}
```

`captured_request_messages` and `large_transcript_prompt` do **not** exist — both are yours to write. Add `captured_request_messages(&self, index: usize) -> usize` to `tests/integration/support/assertions.rs`, following the `self.scripted_llm.captured_requests()` idiom already used there at `:587` and `:629`; the underlying `TraceLlm::captured_requests() -> Vec<Vec<ChatMessage>>` is real (`tests/support/trace_llm.rs:548`). `loop_milestones()` and `milestone_len()`, used by Assertion B, **do** exist.

**Assertion B — compaction fires earlier.** The seam is the milestone stream, not a bespoke helper: `LoopHostMilestoneKind::CompactionStarted` / `CompactionCompleted` (`ironclaw_loop_contracts/src/milestones.rs:150-156`), read through `self.loop_milestones()` with a `milestone_len()` baseline — the `_since` pattern `assert_compaction_failed_since` uses (`assertions.rs:1480`). Assert compaction started for the narrow-window run and did not for the unadvertised one.

**Sizing the transcript.** The transcript must fall *between* the two ceilings, so express both in the same unit — characters — because the token figures happen to collide numerically and are easy to mix up:

| Run | visible budget (tokens) | at chars/4 |
|---|---|---|
| advertised 40,000 → limit 36,000, reserve `min(20_000, 9_000)` = 9,000 | 27,000 | ≈ **108,000 chars** |
| unadvertised → limit 128,000, reserve 20,000 | 108,000 | ≈ **432,000 chars** |

Note the trap: 108,000 is the *character* target for the narrow run and the *token* budget for the default run. Aim for roughly 150,000–250,000 characters of transcript — comfortably over the narrow ceiling, comfortably under the default one. Seed it by submitting prior turns with large text bodies, following whichever existing scenario Step 1 found; script one `RebornScriptedReply::text(..)` per seeding turn or the FIFO over-runs.

- [ ] **Step 3: Run the test to verify it fails for the right reason**

Run: `cargo test --test reborn_integration_<name>`
Expected: FAIL because compaction did not trigger — **not** because of a harness, script-FIFO, or wiring error. If it fails for any other reason, fix the test before touching anything else. Temporarily reverting Task 4 and confirming the test still fails the same way is a cheap way to prove it is pinning the right thing.

- [ ] **Step 4: Confirm it passes against the implementation**

Run the same command.
Expected: PASS. Tasks 1-8 are the implementation; no production code should need changing here. If it does, that is a real gap — fix it in the owning task's file and say so.

- [ ] **Step 5: Update the scenario coverage map**

`tests/integration/AGENTS.md` opens with a binding rule: adding, removing, renaming, or materially re-scoping a test here means updating `tests/AGENTS.md` **in the same commit**. Add or amend the row describing, in plain English, what this scenario proves.

- [ ] **Step 6: Run the architecture tests**

Run: `cargo test -p ironclaw_architecture_tests`
Expected: PASS. Proves `ironclaw_agent_loop` still takes contracts-layer dependencies only.

- [ ] **Step 7: Run the full gate**

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test
```
Expected: clean. Postgres legs self-provision testcontainers and are skipped without Docker; note in the PR if they were skipped.

- [ ] **Step 8: Commit**

```bash
git add tests/integration/ tests/AGENTS.md
git commit -m "test(integration): pin model-derived context budget through a real turn"
```

---

## Not in this plan

`ThreadBackedLoopModelGateway::issue_host_prompt_bundle` (`ironclaw_loop_host/src/model_gateway.rs:311-326`) also builds a context port with no budget and keeps the 128k default. It is deliberately left alone: `ThreadBackedLoopModelGateway::new` is instantiated only by `ironclaw_loop_host/tests/llm_gateway.rs`, never by composition, which wires `ThreadResolvingLoopModelGateway`. Wiring an unreachable path would add a call site with no production meaning. If it is ever composed, it must be wired in that change. Do not confuse it with `ThreadResolvingLoopModelGateway`, which Task 6 does wire.

Populating `ModelMetadata.context_length` per provider is the deliberate follow-on slice, specified in the spec's "Follow-on slice" section. Today only `gemini_oauth.rs:2158` populates it; every other provider returns `None` and therefore keeps today's exact behavior after this plan lands. Do not start it here — one provider at a time, behaviorally isolated.

`error.rs:437-465` already parses the provider's authoritative limit into `LlmError::ContextLengthExceeded { used, limit }`, and `model_gateway.rs:2692` discards it. Feeding that back is a later option, not this plan.
