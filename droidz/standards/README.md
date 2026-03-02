# AI Agent Standards Index

`droidz/standards/` is the canonical source of implementation standards for AI agents operating in this repository.

## How to use this directory

1. Identify the work area (error handling, database, tool implementation, etc.).
2. Read the corresponding standard file before making changes.
3. Apply required checks from `review-discipline.md` before finishing work.
4. Run the required verification commands and review checks from these standards before opening or updating a PR.

## Standards files

- `code-style.md` — Rust style and module conventions
- `error-handling.md` — error modeling and panic avoidance
- `async-patterns.md` — async/concurrency rules
- `tool-implementation.md` — Tool trait and capability conventions
- `database.md` — dual-backend persistence requirements
- `testing.md` — test expectations and feature-matrix checks
- `security.md` — safety, secret, and sanitization requirements
- `feature-parity.md` — when to update `FEATURE_PARITY.md`
- `review-discipline.md` — mechanical checks and pattern-wide fixes
- `commits-and-prs.md` — commit/PR quality standards
- `adding-features.md` — architecture-first rules for feature additions

## Enforcement

Standards are enforced through code review and required project verification commands (format, lint, and tests).

If standards and implementation conventions disagree, update the standards in the same branch as the implementation change.
