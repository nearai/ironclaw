# Contributing

## Feature Parity Requirement

When your change affects a tracked capability, update `FEATURE_PARITY.md` in the same branch.

### Required before opening a PR

1. Review the relevant parity rows in `FEATURE_PARITY.md`.
2. Update status/notes if behavior changed.
3. Include the `FEATURE_PARITY.md` diff in your commit when applicable.

## Review Tracks

All PRs follow a risk-based review process:

| Track | Scope | Requirements |
|-------|-------|-------------|
| **A** | Docs, tests, chore, dependency bumps | 1 approval + CI green |
| **B** | Features, refactors, new tools/channels | 1 approval + CI green + test evidence |
| **C** | Security (`src/safety/`, `src/secrets/`), runtime (`src/agent/`, `src/worker/`), database schema, CI workflows | 2 approvals + rollback plan documented |

Select the appropriate track in the PR template based on what your changes touch.
