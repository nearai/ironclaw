# Code Style Standard (AI Agents)

## Required

- Use `crate::` imports in production code.
- Avoid `pub use` re-exports unless intentionally exposing downstream API surface.
- Prefer strong types (enums/newtypes) over ad-hoc stringly-typed values.
- Keep functions focused; extract helpers when logic is reused.
- Add comments only for non-obvious intent.
- Run formatting with `cargo fmt`.

## Rust module conventions

- Preserve existing project structure and naming patterns.
- Match surrounding code style when editing an existing file.
- Avoid introducing new crates unless already used or explicitly required.

## Prohibited

- Large unrelated refactors in the same change.
- Style-only churn in files unrelated to the task.
