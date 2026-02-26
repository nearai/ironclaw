# Draft GitHub Issue: nearai/ironclaw

---

**Title:** proposal: Add clippy complexity lints to improve AI-assisted development quality

---

## Context

As AI coding agents (Claude Code, Cursor, Copilot, etc.) become a larger part of the development workflow, certain code quality problems recur because agents generate code sequentially without the holistic awareness a human maintainer develops over time. This is already acknowledged in issue #326 (crate isolation to reduce over-dependency between components).

Clippy ships with configurable lints that catch the most common agent-generated smells at compile time, before they reach review. Enabling them is zero-cost at runtime and a one-file change.

## Problem

A review of the current codebase found patterns that clippy's restriction lints would catch automatically:

- **49 functions exceed 80 lines**, including `main()` at 554 lines and `Agent::run()` at 366 lines. Long functions are harder to review, test, and reason about.
- **Cognitive complexity** is high in several hot paths (deeply nested `match`/`if let` chains with 4-5 levels of nesting in `find_code_regions()`, `step_channels()`, etc.).
- **10 structs have 10+ fields**, some mixing unrelated concerns (e.g., `GatewayState` bundles 21 fields from 15 subsystems into every HTTP handler).

These aren't bugs today, but they make the codebase harder to maintain and more prone to regressions -- especially when agents contribute changes without full project context.

## Suggestion

Add a `clippy.toml` at the project root with thresholds that prevent new violations while allowing time to address existing ones:

```toml
# Complexity guardrails
cognitive-complexity-threshold = 15    # default: 25
too-many-lines-threshold = 100         # default: 100 (make explicit)
too-many-arguments-threshold = 6       # default: 7
type-complexity-threshold = 200        # default: 250
```

And enable the restriction lints in `src/lib.rs`:

```rust
#![warn(clippy::cognitive_complexity)]
#![warn(clippy::too_many_lines)]
```

### Why these thresholds

- **`cognitive-complexity = 15`**: Still accommodates idiomatic Rust (match arms, error chains) but flags functions that mix multiple concerns. The default of 25 is too permissive to catch real issues.
- **`too-many-lines = 100`**: Matches the existing CLAUDE.md guidance. Functions above this tend to have multiple responsibilities.
- **`too-many-arguments = 6`**: Nudges toward builder patterns or config structs. Already close to the default (7).
- **`type-complexity = 200`**: Flags deeply nested `Arc<RwLock<HashMap<...>>>` types that should get a newtype wrapper.

### Rollout approach

These lints produce warnings, not errors, so they won't break existing code or CI immediately. The project can:

1. **Add `clippy.toml` + lint attributes** -- new code gets flagged
2. **Add `#[allow(...)]` to existing violations** if needed for a clean build
3. **Address violations incrementally** as modules are touched (aligns with the crate extraction in #326)

## Alternatives considered

- **Stricter thresholds** (e.g., `cognitive-complexity = 10`, `too-many-lines = 60`): Would require significant refactoring of existing code first. Can ratchet down later.
- **`#![deny(...)]` instead of `#![warn(...)]`**: Too aggressive for a project with existing violations. Start with warnings.
- **External tools** (Lizard, SonarQube): Add CI complexity. Clippy is already in the toolchain and runs in the existing `cargo clippy` step.

## Additional tools worth considering

Beyond clippy, two other tools would complement this well (though they're separate decisions):

- **`cargo-llvm-cov --fail-under-lines N`**: Enforces a minimum test coverage percentage in CI. Even a low threshold (e.g., 30%) prevents new modules from shipping with zero tests.
- **`cargo-mutants`**: Mutation testing that finds functions where no test actually verifies the behavior. Slower, better suited as a weekly CI job.

Happy to submit a PR for the `clippy.toml` if the team is interested.
