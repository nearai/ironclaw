---
paths:
  - "crates/**/*.rs"
  - "src/**/*.rs"
---
# Architecture Discipline — Stop the Sprawl Before It Ships

This rule exists because the engine refactor of 2026-05 found a class
of slow-burn architectural decay that no existing rule catches. The
individual symptoms look reasonable in isolation (one extra Arc, one
extra method arg, one `with_*` builder, one `#[allow(...)]`). The
class is recognizable only when you grep for the smoke alarms across
the crate: 11 `#[allow(clippy::too_many_arguments)]` annotations in
`crates/ironclaw_engine/`, a 7,933-line `runtime/mission.rs`, two
parallel action-dispatch pipelines, and the same six Arcs threaded
through three layers without ever being given a name.

The rule is: **listen to the language. When the compiler or clippy
complains, the answer is almost never `#[allow]`.**

## Five smells, with grep-able patterns

### 1. Argument creep — `#[allow(clippy::too_many_arguments)]` is a smoke alarm

clippy's default is 7 args. Reaching it means the function has more
inputs than a reader can hold in their head. Allowing it once is a
trade — allowing it eleven times is a refactor someone declined to
do.

**Required pattern** when introducing the allow:

```rust
// arch-exempt: too_many_args, <one-line reason>, plan #NNNN
#[allow(clippy::too_many_arguments)]
fn execute_orchestrator(...)
```

The annotation must name *what aggregation is missing* (a context
struct, a service bundle, a config object) and link a tracking
issue or plan. "Refactor needed" is not a reason; "needs `EngineServices`
bundle, plan #2800" is.

**Review flag:** any added `#[allow(clippy::too_many_arguments)]`
without an `arch-exempt` annotation on the line above it.

### 2. Optional Arcs that are required in production

```rust
struct Foo {
    a: Arc<dyn Bar>,
    b: Option<Arc<dyn Baz>>,  // <- smell
}

impl Foo {
    fn with_baz(mut self, b: Arc<dyn Baz>) -> Self { ... }
}
```

If production wires `with_baz` every time and only test code skips
it, the type system is lying. The `is_some()` branches that result
become dead paths in production and the favourite home of bugs that
only one user trips.

**Rule:** `Option<Arc<…>>` on a runtime struct is allowed only when
the dependency is *genuinely* optional (e.g., a feature-flagged
component that the binary may legitimately ship without). If the
production wiring always sets it, either:

- Make it required and have tests construct it with a fake.
- Or move the conditional behavior into a separate type (split the
  struct).

**Review flag:** `Option<Arc<` added on a struct field, paired with a
`with_<name>` builder that the production call site always invokes.

### 3. Re-derived identity / duplicated state

A field that already lives on a primary entity (`Thread.id`,
`Thread.user_id`, `Thread.project_id`) cannot be re-declared on a
context struct that the same code path constructs from that entity.

```rust
// Bad: ThreadExecutionContext re-declares thread_id, project_id, user_id
//      that already exist on Thread, and the constructor copies them.
struct ThreadExecutionContext {
    thread_id: ThreadId,
    project_id: ProjectId,
    user_id: String,
    // ... step-scoped fields
}

// Good: pass the source-of-truth entity by reference; only carry
//       step-scoped data on the side struct.
struct StepFrame<'t> {
    thread: &'t Thread,
    step_id: StepId,
    current_call_id: Option<String>,
}
```

**Why:** identity confusion has shipped four times in `types.md`
(PRs #2561, #2473, #2512, #2574). Duplicating identity onto a side
struct *adds another copy* of the value the compiler cannot enforce
agreement on.

**Review flag:** a field on type `B` whose name and type match a
field on type `A` when `B` is constructed from `A` in the same file.

### 4. Duplicate dispatch pipelines

When the same downstream call (`effects.execute_action`,
`safety_layer.scan_*`, `dispatcher.dispatch`) is invoked from two
places that each implement their own pre-checks (lease, policy,
sanitization), they are one pipeline written twice. Every
safety/policy change must then land in both — and one always lags.

This mirrors the rule in `tools.md` ("Everything Goes Through Tools")
and `safety-and-sandbox.md` ("Every New Ingress Scans Before Storage
or LLM"). The pattern: identify the converging downstream call,
extract a single gateway, route both sides through it.

**Review flag:** a new call site to a registry/executor/dispatcher
trait method that re-implements pre-checks (lookup, policy, lease,
scan) already implemented at another call site.

### 5. File size budget

A `.rs` file > 1,500 lines is a refactor the codebase has been
postponing. A file > 3,000 lines is a refactor that is already
costing review time. There is no hard cap, but:

- New files: aim for < 800 lines.
- Existing files between 1,500 and 3,000: every PR that touches them
  should leave them shorter, not longer, unless the PR is explicitly
  expanding a feature that has nowhere else to live.
- Existing files > 3,000: file a tracking issue for decomposition.
  PRs that *add* > 200 lines need an inline justification.

This is not a mechanical check — it's a culture norm. The
mechanical check is "does this file have a tracking issue."

## Where each rule is enforced

The rules above are deliberately split across enforcement layers
because each layer catches a different failure mode.

| Smell | Pre-commit script | CI / clippy | Code review | Agent-facing (this file) |
|---|---|---|---|---|
| 1. `too_many_arguments` allow | yes — count + annotation grep | clippy default already fires | required | yes |
| 2. `Option<Arc<…>>` + `with_*` | yes — paired-pattern grep | — | required | yes |
| 3. Re-derived identity | — (heuristic) | — | required | yes |
| 4. Duplicate dispatch | partial — known-method grep | — | required | yes |
| 5. File size | yes — `wc -l` on staged | — | informational | yes |

### Why this split

- **Pre-commit catches the mechanical patterns.** A regex on staged
  diffs is enough for #1 (annotation grep), #2 (paired patterns),
  and #5 (line count). These are cheap, deterministic, and run on
  every commit. Add to `scripts/pre-commit-safety.sh` as Check #10
  (`ARCH-SPRAWL`) following the existing format.
- **CI / clippy catches what compilers can express.** clippy already
  emits `too_many_arguments`. The rule is "don't silence it without
  a plan link." No new CI check needed; existing default works once
  the annotation discipline lands.
- **Code review catches semantic patterns.** #3 and #4 require
  reading the code, not the diff. The annotation `// arch-exempt:
  <category>, <reason>, plan #NNNN` puts the burden on the proposer
  to name the aggregation that is missing — reviewers reject
  exempts without a plan link.
- **This rule file is the agent-facing summary.** Loaded into context
  whenever an agent edits `crates/**/*.rs` or `src/**/*.rs`. The
  agent's job: *don't be the one who adds the twelfth `#[allow]`.*

## Annotation format (consistent with other rules)

```rust
// arch-exempt: <category>, <reason>, plan #NNNN
```

Categories:

- `too_many_args` — function signature grew past clippy default.
- `optional_arc` — `Option<Arc<…>>` field on a runtime struct.
- `parallel_dispatch` — second call site to a converging downstream.
- `large_file` — file size growing past 1,500 lines.

Each must name a tracking issue or plan that owns the cleanup. An
exempt without a plan link is a violation, not an exception.

## What this rule does NOT cover

- **Trait shape.** Trait method signatures are part of the public
  contract; their argument count is governed by API design, not
  this rule.
- **Test code.** Tests are allowed to construct things with the
  full Arc bag explicitly when that makes the test clearer.
  `#[cfg(test)]` blocks are skipped by the pre-commit check.
- **Generated code.** WIT bindings, `serde` derive output, and
  similar machine-emitted code are exempt (they are not what a
  reader maintains).
- **One-off scripts under `scripts/`.** Architectural sprawl in a
  shell script or migration helper is a different conversation.

## References

- The diagnosis that motivated this rule:
  `docs/plans/2026-05-02-engine-architecture-simplification.md`.
- Adjacent rules with the same shape (extract a single gateway,
  route everything through it): `tools.md`, `safety-and-sandbox.md`,
  `gateway-events.md`.
- Annotation discipline reference: `gateway-events.md` —
  `// projection-exempt: <category>, <detail>` is the canonical
  shape this rule borrows.
