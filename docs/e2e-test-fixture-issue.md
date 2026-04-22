# Seed issue for the fix-issue live e2e test

The `tests/e2e_live_fix_issue_flow.rs` test drives the `/fix-issue`
workflow on the scratch repo `nearai/ironclaw-e2e-test`, which is a
clone of the main IronClaw codebase. The test targets issue number `1`
by default (see `ISSUE_NUMBER` in the test).

The seeded issue asks the agent to build the `thread_metadata_set`
builtin tool — the smallest self-contained piece of the worktree +
metadata-pills feature currently being shipped in
`nearai/ironclaw`. Because the fixture repo is an older snapshot
without that tool, the agent's resulting implementation can be
diff'd against the real one in this repo
(`src/tools/builtin/thread_metadata.rs`) as a quality signal.

## Fixture repo facts

- Default branch: `staging` (PRs target this, not `main`).
- The repo is a snapshot of the IronClaw codebase — any implementation
  task must follow the same conventions (`thiserror`, `async_trait`,
  `crate::` imports, no `unwrap` in production, etc.).

## Issue title

> Add `thread_metadata_set` builtin tool for per-thread metadata patches

## Issue body (paste verbatim into issue #1 on `nearai/ironclaw-e2e-test`)

```markdown
Add a new builtin tool, `thread_metadata_set`, that skills can call
to stash per-thread state (branch name, PR url, working-note flags,
anything JSON-shaped) under the current thread's `metadata` object.

The intended semantics is **replace-at-top-level-key**: each top-level
key in the patch overwrites the matching key in the thread's metadata
wholesale — not a deep merge. Skills namespace their state under a
single key (e.g. `dev`, `notes`) and the "patch" is that whole
namespace written at once. Deep merge lets two skills silently drop
each other's sub-keys; replace-at-top makes contention visible.

The engine wiring to actually apply the patch lives in follow-ups —
this issue is **just the tool** plus its registration and unit tests.

## Files

1. **New:** `src/tools/builtin/thread_metadata.rs`
   - Export a public struct `ThreadMetadataSetTool`.
   - Implement the `Tool` trait (`name`, `description`, `parameters_schema`,
     `execute`, `requires_sanitization`).
   - `name()` returns `"thread_metadata_set"`.
   - `parameters_schema()` requires a single object parameter `patch`.
   - `execute()` validates that `patch` is a JSON object and returns
     the serialized patch JSON as a `ToolOutput::text(...)` payload.
     The engine will read that output in a follow-up and apply the
     merge; this tool does **not** persist anything itself.
   - Cap the serialized patch at **8 KiB**; reject with
     `ToolError::InvalidParameters` above that so a runaway skill
     can't bloat context.
   - `requires_sanitization()` returns `false` (internal tool).

2. **Register** in `src/tools/builtin/mod.rs` (add the `mod` + `pub use`).

3. **Register** in `ToolRegistry::register_builtin_tools()` in
   `src/tools/registry.rs` next to the other base tools (`EchoTool`,
   `TimeTool`, `JsonTool`, `PlanUpdateTool`).

## Tests (required)

Under the same module in a `#[cfg(test)] mod tests {}` block, cover:

- **missing patch** → `ToolError::InvalidParameters`
- **non-object patch** (e.g. `"nope"`) → `ToolError::InvalidParameters`
- **oversized patch** (>8 KiB) → `ToolError::InvalidParameters`
- **happy path**: `{patch: {dev: {branch: "feature/x"}}}` → output
  payload parses back to the same JSON object.

## Acceptance criteria

- `cargo fmt` clean.
- `cargo clippy --lib --features libsql -- -D warnings` clean.
- `cargo test --lib thread_metadata` passes (4 tests).
- PR opened as a draft against `staging`.
- PR body includes the phrase "replace-at-top-level-key" and closes
  this issue (`Closes #<number>`).
```

## Diff signal

After the agent opens its PR on the fixture repo, compare its
`src/tools/builtin/thread_metadata.rs` against the same-named file in
`nearai/ironclaw`. Both should:

- Follow the same `Tool` trait implementation shape.
- Cap patch size at 8 KiB and reject oversized input.
- Return the patch JSON as `ToolOutput::text(...)` output.
- Carry tests for the same four failure/happy cases.

Deep drift between the two is a signal for either a skill-prompt gap
(the agent misread the spec) or a regression in how the `coding-repo`
/ `fix-issue` skills channel the agent through the implementation.
