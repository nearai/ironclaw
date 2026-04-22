# CodeAct Host-Backed Pythonic Shims

**Date:** 2026-04-21  
**Status:** Proposed / Phase 0  
**Branch:** `feat/codeact-host-shims-phase1`

## Context

Engine v2 currently gives CodeAct Python access to host capabilities by exposing action names into Monty's name lookup. The runtime contract is intentionally narrow:

- unknown function calls in Python suspend the VM
- the host resolves them through capability leases + policy checks
- the `EffectExecutor` performs the canonical action
- the result is returned to Python

This is the correct security boundary, but the current surface is still too raw for good agent UX. The model has to remember bespoke action names, parameter schemas, and result shapes instead of writing the kind of Python it was pretrained on.

Relevant repo anchors:

- `crates/ironclaw_engine/CLAUDE.md:132` — unknown function calls suspend the VM, then flow through lease/policy/`EffectExecutor`
- `crates/ironclaw_engine/src/executor/scripting.rs` — Monty runtime integration, name lookup, OS denial, action result marshalling
- `crates/ironclaw_engine/prompts/codeact_preamble.md` — current prompt contract for the embedded Python environment
- `docs/plans/2026-03-20-engine-v2-architecture.md:359-395` — too many tool schemas degrade agent performance; skills/knowledge-bearing definitions are the preferred pattern
- `tests/engine_v2_skill_codeact.rs` — end-to-end CodeAct execution tests
- `tests/e2e_metrics_test.rs` + `tests/support/metrics.rs` — run comparison support for tokens, turns, tool calls, and wall time
- `tests/e2e_live.rs` + `tests/support/LIVE_TESTING.md` — live-vs-replay trace capture for real-model evaluation

## Why this exists

We want to make CodeAct feel more like Python **without**:

- giving Monty direct OS/filesystem/network access
- bypassing leases, approvals, policies, or the SafetyLayer
- exploding the tool/action list
- turning the prompt into a giant cookbook

The intended shape is:

- keep raw canonical actions available underneath
- add a small, Pythonic shim layer on top
- teach the model to prefer the shim layer for common workflows
- preserve the raw actions as escape hatches when the shim is insufficient

## Two framing questions

### 1. Can't the model already use the underlying tools from Python?

Yes, but only in the current engine-specific sense.

Today the model can call host actions from Python because the runtime resolves callable names during Monty name lookup. That means the model can write `await http(...)`, `await shell(...)`, etc. However, those are **not** native Python stdlib affordances. They are raw effect endpoints with repo-specific names, repo-specific argument shapes, and repo-specific result payloads.

So the current system already supports host actions in Python, but it still forces the model to solve three avoidable problems:

1. **Tool selection:** which raw action name should I call?
2. **Schema recall:** what exact parameters does it want?
3. **Result normalization:** what keys come back and how should I branch on them?

A shim layer keeps the same host authority while compressing those choices into a smaller, more pretrained-compatible interface.

### 2. Won't prescribing shim APIs reduce the model's creativity?

It can if we overdo it. That is **not** the plan.

The goal is to reduce wasted creativity on schema reconstruction, not reduce useful creativity in planning.

Bad prescription would mean:

- hiding all raw tools
- forcing one rigid recipe for every task
- teaching a large decision tree the model must obey mechanically

Good prescription means:

- provide a few default ergonomic affordances (`read_text`, `write_text`, `run`, `http_get`)
- keep raw actions available for edge cases
- explicitly tell the model: prefer shims for common cases, drop to raw actions when needed

This should increase effective creativity because the model spends less effort remembering tool minutiae and more effort on decomposition, verification, and recovery.

## Design principles

1. **Canonical truth stays in the host action layer.**
   Shims are facades, not new privileged capabilities.
2. **Raw actions remain available.**
   We are adding defaults, not removing escape hatches.
3. **Small API surface first.**
   Start with a few high-ROI shims before inventing a fake stdlib.
4. **Prefer familiar object shapes.**
   Return `resp.ok`, `resp.status`, `proc.stdout`, etc. when possible.
5. **Test through the caller.**
   Validate behavior through the CodeAct runtime, not only helper functions.
6. **Measure agent performance, not wrapper microbenchmarks.**
   The main wins should be fewer turns, fewer tokens, fewer failed tool calls, and better task completion.
7. **Prompt lightly.**
   Teach the preferred shims concisely; do not bury the model in documentation.

## Non-goals

- Replacing Monty with CPython
- Recreating the Python standard library
- Letting Python call OS/filesystem/network directly
- Adding a second security or approval system
- Hiding raw tools from advanced workflows

## Proposed runtime shape

### Canonical raw actions (unchanged)

Examples:

- `read_file`
- `write_file`
- `http`
- `shell`
- `list_dir`
- `apply_patch`

These remain the actions that leases, policies, approvals, logging, and metrics understand.

### Preferred shim functions (new)

Phase 1 targets:

- `read_text(path)`
- `write_text(path, text)`
- `http_get(url, headers=None)`
- `run(command, timeout=None)`

Likely follow-ons:

- `append_text(path, text)`
- `exists(path)`
- `list_dir(path=".")`
- `glob(pattern, path=".")`
- `read_json(path)`
- `write_json(path, obj)`
- `http_request(method, url, headers=None, body=None, json=None)`

### Result objects

Phase 1 can ship with simple host-backed objects or normalized dict-like shapes. The ideal target is a small set of predictable result types:

- `HttpResponse`
  - `ok`
  - `status`
  - `text`
  - `headers`
  - `json()` or parsed `json_body`
- `CompletedProcess`
  - `ok`
  - `exit_code`
  - `stdout`
  - `stderr`
  - `duration_ms`

## Host mapping contract

Every shim must map to one or more existing actions. Example mapping:

- `read_text(path)` → canonical file-read action
- `write_text(path, text)` → canonical file-write action
- `http_get(url, headers=None)` → canonical `http` action with `method="GET"`
- `run(command, timeout=None)` → canonical `shell` action

Important invariant:

> The shim name is never the trust boundary. The underlying action remains the trust boundary.

That means:

- approvals still trigger based on canonical action policy
- leases still count canonical action usage
- logs/events still describe canonical effects
- the shim layer is purely ergonomic and semantic normalization

## Phased implementation plan

## Phase 0 — planning + guardrails

Deliverables:

- this plan doc
- isolated worktree + branch
- explicit rule that raw actions stay available
- explicit rule that shims are thin facades over canonical actions
- choose a narrow first slice

Acceptance criteria:

- clear phase ordering
- testing strategy agreed before writing runtime code

## Phase 1 — minimal ergonomic shims

Scope:

- `read_text(path)`
- `write_text(path, text)`
- `http_get(url, headers=None)`
- `run(command, timeout=None)`

Runtime work:

- extend the scripting runtime to resolve these names as built-in shim functions
- intercept shim invocation before or alongside generic action dispatch
- translate shim arguments to canonical action parameters
- normalize results into predictable Python-friendly values
- keep raw action names working exactly as before

Prompt work:

- update `crates/ironclaw_engine/prompts/codeact_preamble.md`
- add a compact “preferred shims” section with 1-2 examples
- explicitly say raw actions still exist for advanced cases

Docs work:

- update `crates/ironclaw_engine/CLAUDE.md` or `MONTY.md` if the runtime contract meaningfully changes
- note the canonical action mapping and why it preserves policy/approval semantics

Tests (TDD-first):

- add failing end-to-end CodeAct tests that call the new shim names
- verify the thread completes successfully through the full runtime
- verify the canonical underlying action was invoked
- verify result normalization shape is as documented
- verify approval/policy behavior is unchanged relative to canonical action use

Likely files:

- `crates/ironclaw_engine/src/executor/scripting.rs`
- `crates/ironclaw_engine/prompts/codeact_preamble.md`
- `tests/engine_v2_skill_codeact.rs` or a new neighboring test file dedicated to shims

Acceptance criteria:

- all four shims work end-to-end in CodeAct
- raw actions still work unchanged
- tests prove dispatch still flows through canonical actions

## Phase 2 — high-frequency filesystem and JSON helpers

Scope:

- `append_text`
- `exists`
- `list_dir`
- `glob`
- `read_json`
- `write_json`

Goal:

- reduce common multi-call boilerplate for repo editing and inspection tasks

Tests:

- repo-inspection flows
- read/modify/write loops
- JSON roundtrip behavior
- error normalization for missing files / invalid JSON / non-zero shell exits

Acceptance criteria:

- common repo workflows require fewer raw action invocations and less parsing boilerplate

## Phase 3 — richer objects

Scope:

- evaluate host-backed dataclass/object support for `HttpResponse`, `CompletedProcess`, maybe `Path`
- if host objects are awkward in Monty, keep the simpler normalized dict-like contract

Goal:

- make Python feel more idiomatic without introducing interpreter fragility

Tests:

- attribute access (`resp.ok`, `proc.stdout`)
- JSON parsing ergonomics
- object behavior in multiple CodeAct blocks where relevant

Acceptance criteria:

- result types improve ergonomics without introducing brittle Monty behavior

## Phase 4 — scenario evaluation and A/B metrics

Use the existing repo infrastructure rather than inventing a new benchmark harness.

### Deterministic scenario evaluation

Re-use:

- `tests/e2e_metrics_test.rs`
- `tests/support/metrics.rs`

Track deltas for:

- pass rate
- wall-clock time
- LLM calls
- total tokens
- tool call count
- failed tool calls
- turn count

Suggested scenario set:

1. read → summarize → write
2. grep/list/read/edit flow
3. HTTP fetch → inspect JSON → summarize
4. shell run → inspect stderr → retry/fix
5. multi-step repo inspection task

### Live model evaluation

Re-use:

- `tests/e2e_live.rs`
- `tests/support/LIVE_TESTING.md`

Plan:

- record representative live traces in baseline mode
- record the same scenarios with shims enabled / preferred
- replay both deterministically from fixtures
- compare completion behavior and metrics

Acceptance criteria:

At least one of the following should improve without hurting pass rate:

- fewer LLM calls
- lower total token usage
- fewer failed tool calls
- fewer total turns
- better scenario success rate

## Feature flag / rollout suggestion

Introduce a temporary feature gate if needed so we can A/B test safely. Options:

- engine config flag
- thread config flag
- test-rig option for shim preference / enablement

The goal is not permanent user-facing complexity. It is controlled evaluation while the API stabilizes.

## Testing strategy

### 1. Runtime correctness tests

Primary rule: test through the CodeAct caller, not only shim helper functions.

Good tests:

- scripted LLM returns code that calls a shim
- runtime resolves and executes it
- canonical action is recorded underneath
- final answer contains the expected result

### 2. Regression tests for raw actions

Add explicit coverage that existing raw `http` / shell / file action behavior is unchanged when shims are present.

### 3. Metrics comparison tests

Build `RunResult` snapshots before/after and compare with `compare_runs()`.

### 4. Live/replay tests

Use live tests to confirm real-model adoption and replay traces to make the result deterministic and CI-friendly.

## Risks and mitigations

### Risk: over-prescribing the model
Mitigation:

- keep prompt additions short
- keep raw actions available
- describe shims as preferred defaults, not the only legal path

### Risk: security bypass through shim shortcuts
Mitigation:

- every shim must delegate to canonical actions
- approvals, leases, policies, and logging must remain keyed to canonical actions
- no direct OS/filesystem/network from Monty

### Risk: prompt bloat
Mitigation:

- document only the small starter set in the preamble
- move deeper details into docs, not the hot prompt path

### Risk: Monty object-model fragility
Mitigation:

- start with simple normalized values
- only add richer objects if tests show they behave reliably

### Risk: wrapper overhead is mistaken for regression
Mitigation:

- evaluate agent-level metrics, not only per-call runtime overhead

## Concrete first slice

The first implementation slice should be intentionally boring:

1. Add failing tests for `read_text`, `write_text`, `http_get`, and `run`
2. Implement the thinnest mapping in `scripting.rs`
3. Normalize result shapes just enough for useful branching
4. Add a small prompt section recommending those shims
5. Run targeted engine tests
6. If stable, add one replay/live comparison scenario

## Acceptance criteria for the branch

This branch is a success if it produces all of the following:

- a reviewable plan doc
- isolated worktree-based implementation with no impact on the dirty main worktree
- Phase 1 shims implemented behind the existing engine security model
- tests proving shim calls still route through canonical actions
- a clear next step for Phase 2 and A/B evaluation

## Updated next-shim shortlist (2026-04-22 benchmark pass)

The live/replay A/B suite now shows a clear split:

- **Huge wins** happen on multi-file, path-sensitive, read/transform/write workflows (`package_json_edit`, `monorepo_package_migration`, `js_codemod_use_strict`, `yaml_workflow_update`, `cargo_toml_rust_version_sync`).
- **Small or neutral wins** happen on already-clean single- or dual-file flows (`read_json`, `tsconfig_*`, `append_text`, `find_files`, `mixed_config_sync`).

That means the next shims should target the *remaining multi-step friction*, not add more one-call sugar.

### Priority 1 — path-safe file discovery

Candidate shapes:

- `find_paths(pattern, path=".")`
- or `find_files(..., absolute=True)`

Why first:

- the biggest remaining failures in the high-friction scenarios were path-joining mistakes after discovery (`read_file(ci.yml)`, `read_file(sub/d.js)`, etc.)
- this is the same category of ergonomics bug that `list_entries()` normalization already proved valuable

### Priority 2 — literal text patch helper

Candidate shapes:

- `replace_in_file(path, old, new, count=None)`
- `insert_after(path, anchor, text)`

Why second:

- the large wins in YAML / JS / Cargo.toml scenarios still required the model to hand-roll read/replace/write loops
- a small literal patch helper should reduce both code volume and accidental whitespace drift without introducing a broad new capability class

### Priority 3 — TOML-aware structured helpers

Candidate shapes:

- `read_toml(path)`
- `write_toml(path, value)`

Why third:

- `cargo_toml_rust_version_sync` showed strong shim value even with only text helpers
- that is a strong sign TOML is a real remaining pain point, especially in this Rust repo

### Priority 4 — YAML-aware structured helpers

Candidate shapes:

- `read_yaml(path)`
- `write_yaml(path, value)`

Why fourth:

- `yaml_workflow_update` still improved substantially with current text shims, but the model had to do brittle plain-text edits
- structured YAML helpers likely help workflow, compose, and CI config tasks, but comment/order preservation needs careful design

### Priority 5 — targeted batch-edit helper

Candidate shapes:

- `replace_in_files(pattern, old, new, path=".")`
- `edit_many(paths, op=...)`

Why fifth:

- current wins increasingly come from reducing repeated multi-file orchestration, not single-file convenience
- only pursue this after smaller path/text helpers prove stable, since batch helpers increase surface area quickly

### Explicitly de-prioritized for now

- more one-shot wrappers equivalent to current `append_text` / `find_files` / `read_json` gains
- broad fake-stdlib additions (`pathlib`, `os.path`, large file object emulation)
- JSON-specific patch helpers until they show bigger wins than the current JSON shims already deliver

## Notes for implementation

- Do not turn this into a fake `pathlib` clone immediately.
- Do not remove the existing raw action names.
- Do not add a second action registry for shims.
- Prefer the smallest diff that proves the architecture.
- If a result object shape becomes hard to support in Monty, ship a simpler normalized value first and iterate later.
