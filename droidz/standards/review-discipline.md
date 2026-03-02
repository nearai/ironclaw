# Review Discipline Standard (AI Agents)

## Core principle

Fix the pattern, not just one instance.

When a defect pattern is identified, scan for and address equivalent occurrences across the codebase where in scope.

## Required pre-finish checks

Run mechanical checks on affected areas:

- `.unwrap()` / `.expect(` in production paths
- `super::` imports in production paths (prefer `crate::`)
- repeated instances of the exact bug pattern you fixed

## Architectural propagation

If a core abstraction changes (resource ownership, state model, backend API), update dependent satellite types accordingly.

## Schema translation discipline

For database/backend changes, verify:

- indexes
- seed data
- behavioral equivalence/documented differences

## Review output

Summaries should explicitly state:

- what pattern was fixed
- where else it was checked
- what verification commands were run
