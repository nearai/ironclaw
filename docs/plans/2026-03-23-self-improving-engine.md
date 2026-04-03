# Self-Improving Engine: Automated Debugging and Evolution

**Date:** 2026-03-23
**Status:** Design
**Context:** The last debugging session revealed a clear pattern: trace → human reads trace → human identifies root cause → human edits code → rebuild. Every step of this loop is something the engine can already do. This plan designs a system where the engine debugs and improves itself.

---

## The Pattern We Observed

5 consecutive fixes followed the same loop:

| Trace symptom | Root cause | Fix location | Fix type |
|---|---|---|---|
| `Tool web_search not found` | Hyphen/underscore mismatch | `effect_adapter.rs` | Code (name conversion) |
| `TypeError: str indices must be integers` | JSON double-serialization | `effect_adapter.rs` | Code (parse before wrap) |
| `NameError: result not defined` | No variable persistence | `loop_engine.rs` | Code (state dict) |
| `byte index 80 is not a char boundary` | Unsafe UTF-8 slicing | `thread.rs`, `loop_engine.rs`, `scripting.rs` | Code (chars() not bytes) |
| Model calls `web_fetch` (doesn't exist) | Wrong example in prompt | `codeact_preamble.md` | Prompt edit |

Each fix used the same tools the engine has access to: `read_file`, `apply_patch`, `shell` (cargo test), and file writing.

---

## Three Levels of Self-Improvement

### Level 1: Prompt Evolution (low risk)

The engine modifies its own prompt templates based on accumulated experience.

**What it changes:** `crates/ironclaw_engine/prompts/*.md` files

**Examples:**
- Adds "NEVER call web_fetch — use http() or llm_context()" to rules section
- Adds "freshness parameter: 'pd'=past day, 'pw'=past week, 'pm'=past month" to tool hints
- Adds "Always access previous step data via state['tool_name']" after repeated NameErrors
- Removes examples that reference nonexistent tools

**Safety:** Low risk. Prompt changes only affect LLM behavior, not engine logic. Easy to review diff. Easy to revert (git checkout).

**Trigger:** After every thread with issues detected by trace analysis.

**Validation:** None needed beyond human review of diff.

### Level 2: Configuration Tuning (medium risk)

The engine adjusts its own defaults and mappings.

**What it changes:**
- `ThreadConfig` defaults (max_iterations, truncation limits, compaction thresholds)
- Tool name alias mappings
- Output truncation sizes
- Resource limits

**Examples:**
- After repeated `freshness` errors: add parameter hints to tool descriptions
- After repeated truncation issues: adjust `OUTPUT_TRUNCATE_LEN`
- After excessive step counts: lower `max_iterations` default

**Safety:** Medium risk. Config changes affect execution behavior. Should be bounded (e.g., max_iterations can go 30-100 but not 1 or 10000).

**Trigger:** After N threads with similar patterns (not on first occurrence).

**Validation:** Run existing test suite (`cargo test -p ironclaw_engine`). Only apply if tests pass.

### Level 3: Code Patching (high risk, high value)

The engine proposes Rust code changes to fix bugs it detects in itself.

**What it changes:** Any file in `crates/ironclaw_engine/` or `src/bridge/`

**Examples:**
- Fix unsafe byte slicing (detected by panics in traces)
- Add missing type conversions (detected by tool errors)
- Fix missing match arms (detected by unhandled response types)
- Add error recovery paths (detected by repeated failures)

**Safety:** High risk. Wrong patches can break the engine, introduce security issues, or cause data loss.

**Guardrails:**
1. Always work in a git branch (`self-improve/{timestamp}`)
2. Run full test suite (`cargo test -p ironclaw_engine`)
3. Run clippy (`cargo clippy -p ironclaw_engine --all-targets -- -D warnings`)
4. Never modify files outside `crates/ironclaw_engine/` and `src/bridge/` without human approval
5. Max patch size: 50 lines changed
6. Generate a PR (not direct commit) with trace evidence
7. Human approves or rejects the PR

**Trigger:** After a pattern appears in 3+ traces.

**Validation:** Full test suite + clippy + human review.

---

## Architecture

### Self-Improvement Mission

A `Mission` with `MissionCadence::OnEvent` that triggers after each thread completion:

```
Thread completes
    → Trace analysis (existing, automatic)
    → If issues detected:
        → Spawn self-improvement thread (ThreadType::Reflection)
        → Thread has access to: shell, read_file, write_file, apply_patch
        → Thread reads the trace JSON
        → Thread reads relevant source files
        → Thread proposes a fix
        → Thread validates the fix (cargo test)
        → Thread either:
            a) Applies prompt edit directly (Level 1)
            b) Creates a git branch + PR (Level 2-3)
            c) Logs the proposal for human review
```

### The Self-Improvement Thread's Prompt

```
You are a debugging agent analyzing execution traces from the IronClaw engine.

## Your task
Read the trace file at {trace_path} and identify the root cause of any issues.
Then propose and validate a fix.

## Available information
- Trace JSON: full message history, events, tool results, issues detected
- Source code: read any file in the codebase
- Prompt templates: crates/ironclaw_engine/prompts/*.md
- Bridge adapters: src/bridge/*.rs
- Engine code: crates/ironclaw_engine/src/**/*.rs

## Fix levels
1. PROMPT EDIT: Modify prompts/*.md to prevent LLM mistakes
   → Apply directly, no approval needed
2. CONFIG CHANGE: Adjust defaults in engine code
   → Create git branch, run tests, propose PR
3. CODE PATCH: Fix Rust code bugs
   → Create git branch, run tests + clippy, propose PR

## Rules
- Always read the relevant source file before proposing a change
- Always run `cargo test -p ironclaw_engine` after making changes
- Never modify more than 50 lines in a single patch
- For Level 2-3: create a branch `self-improve/{issue}` and use git
- Explain your reasoning: what the trace shows, why the fix works
```

### Trace-to-Fix Pattern Database

Over time, the system builds a pattern database mapping trace symptoms to fix strategies:

| Trace pattern | Fix strategy | Location pattern |
|---|---|---|
| `Tool X not found` | Add name alias/conversion | `effect_adapter.rs` |
| `TypeError: str indices must be integers` | Parse JSON before wrapping | Where tool output is converted |
| `NameError: name 'X' not defined` | Add to state dict or prompt hint | `loop_engine.rs` or `prompts/*.md` |
| `byte index N is not a char boundary` | Replace `[..N]` with `chars().take(N)` | Grep for `[..` in relevant files |
| Model calls nonexistent tool | Fix prompt example or add alias | `prompts/*.md` or `effect_adapter.rs` |
| Model ignores tool results | Improve output metadata format | `loop_engine.rs` output building |
| Excessive steps (>5) for simple task | Add prompt rule or fix tool schema | `prompts/*.md` |

This database itself is a MemoryDoc that the self-improvement thread can read and extend.

### Feedback Loop

```
                    ┌──────────────────────────────────┐
                    │         User Message              │
                    └──────────────┬───────────────────┘
                                   │
                    ┌──────────────▼───────────────────┐
                    │     Thread Execution (CodeAct)    │
                    │     Using: evolved prompt +       │
                    │     learned rules + tool hints    │
                    └──────────────┬───────────────────┘
                                   │
                    ┌──────────────▼───────────────────┐
                    │     Trace + Reflection            │
                    │     Produces: Lesson, Issue,      │
                    │     Spec, Rule, Playbook docs     │
                    └──────────────┬───────────────────┘
                                   │
                         ┌─────────▼─────────┐
                         │  Issues detected?  │
                         └────┬──────────┬────┘
                              │ yes      │ no
                    ┌─────────▼────┐     └──→ done
                    │ Self-Improve │
                    │ Thread       │
                    ├──────────────┤
                    │ Read trace   │
                    │ Read source  │
                    │ Propose fix  │
                    │ Test fix     │
                    │ Apply/PR     │
                    └──────┬───────┘
                           │
              ┌────────────▼────────────┐
              │  Level 1: prompt edit   │──→ Apply directly
              │  Level 2: config change │──→ Branch + test + PR
              │  Level 3: code patch    │──→ Branch + test + clippy + PR
              └─────────────────────────┘
```

---

## Implementation Plan

### Phase A: Prompt Self-Evolution (Level 1)

**Effort:** Small. Uses existing infrastructure.

1. After reflection, if any `Spec` or `Lesson` docs reference prompt issues, spawn a Level 1 self-improvement thread
2. The thread reads `prompts/codeact_preamble.md` and the Lesson/Spec docs
3. It proposes an edit using `apply_patch` or `write_file`
4. No testing needed — prompt changes are safe
5. Next thread uses the updated prompt (loaded at runtime, not compile time)

**Prerequisite:** Prompts must be loaded at runtime from workspace, not via `include_str!`. Change `build_codeact_system_prompt` to read from store/file with `include_str!` as fallback.

### Phase B: Fix Pattern Database (Level 1-2)

**Effort:** Medium.

1. Create a `MemoryDoc` of type `Playbook` that maps trace symptoms to fix strategies
2. Seed it with the 8 patterns from our debugging session
3. The self-improvement thread reads this playbook before analyzing a trace
4. After successfully fixing an issue, it adds the new pattern to the playbook
5. The playbook grows over time — the system gets better at fixing itself

### Phase C: Automated Code Patches (Level 3)

**Effort:** Large. Requires careful safety design.

1. Self-improvement thread creates a git branch
2. Reads trace + source code + fix pattern database
3. Proposes a Rust code change using `apply_patch`
4. Runs `cargo test -p ironclaw_engine` and `cargo clippy`
5. If tests pass: creates a PR with trace evidence + reasoning
6. Human reviews and merges (or the system auto-merges after N successful self-fixes build trust)

### Phase D: Meta-Evaluation Loop

**Effort:** Large. This is the full autoresearch loop.

1. Periodically replay historical traces against the current code
2. Compare: did the fix actually reduce the failure pattern?
3. Score fixes by effectiveness
4. Revert ineffective fixes
5. Propose more targeted fixes for persistent issues

---

## Safety Model

| Level | What can change | Who approves | Revert mechanism |
|---|---|---|---|
| **1: Prompt** | `prompts/*.md` only | Auto (no approval) | `git checkout prompts/` |
| **2: Config** | Engine defaults, constants | Auto if tests pass | `git revert` |
| **3: Code** | Any `.rs` in engine/bridge | Human via PR review | `git revert` or PR rejection |

**Hard boundaries (never auto-modify):**
- Security-sensitive code (safety layer, policy engine, leak detection)
- Database schemas / migrations
- Files outside `crates/ironclaw_engine/` and `src/bridge/` (without human approval)
- Test files (never weaken tests to make a fix pass)

---

## What We Already Have vs What's New

| Component | Status | Used for |
|---|---|---|
| Trace recording | **Exists** | Input: execution data |
| Retrospective analysis | **Exists** | Detection: find issues |
| Reflection pipeline | **Exists** | Analysis: produce Lessons/Specs |
| RetrievalEngine | **Exists** | Context: inject learnings |
| CodeAct/Monty | **Exists** | Execution: write and run code |
| Tools: shell, read_file, apply_patch | **Exists** | Mechanics: read/edit files, run tests |
| Missions | **Exists** | Trigger: run after events |
| Self-improvement thread prompt | **NEW** | Brain: tells the agent how to debug itself |
| Fix pattern database | **NEW** | Knowledge: maps symptoms to strategies |
| Runtime prompt loading | **NEW** | Prerequisite: prompts editable at runtime |
| Git branch + PR creation | **NEW** | Safety: human review for code changes |
| Trace replay for validation | **NEW** | Quality: verify fixes actually help |

---

## First Concrete Step

The smallest thing that creates a real self-improvement loop:

1. Move prompt loading from `include_str!` to runtime file read (with compiled fallback)
2. After reflection produces a `Spec` doc about a prompt issue, spawn a thread that edits `prompts/codeact_preamble.md`
3. The edit is a simple append to the "Important rules" section
4. Next user message picks up the updated prompt

This is Level 1 prompt evolution with zero risk. One feature, one file change, immediate feedback loop.
