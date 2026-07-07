# Goal: remove obsolete v1 architecture only where Reborn replacement is proven

Source page: https://app.notion.com/p/36e29a6526bf805fb2e9ff4da450683d

Read `../COMMON.md` first. It is part of this goal.

## Stage 0 - Build to spec (inner loop)

Write `spec.md` as a deletion ledger before deleting code. Every proposed deletion must name:

- The v1 module, route, service, config key, binary path, or test being removed.
- The Reborn owner or replacement path.
- The tests that prove replacement behavior.
- Runtime compatibility and rollback risk.
- Docs or parity files that must change.

No deletion is eligible until its replacement evidence is written down and executable tests exist.

## Target (outer loop)

Optimize deletion safety score:

- 40% no remaining production references to deleted v1 surfaces.
- 30% Reborn equivalent behavior is covered by caller-level or integration tests.
- 20% docs, setup, and feature parity references are updated.
- 10% ambiguous entrypoints, config, or duplicated service paths are measurably reduced.

Hard gates: zero broken references, zero removed behavior without replacement evidence, and no new feature behavior added to legacy `src/`. Bar: 1.00 hard-gate pass and at least 0.90 weighted holdout score.

## Eval design

Create a deletion matrix with at least 30 dev and 80 holdout checks. Inputs are deleted symbols, configs, commands, routes, or docs references. Answers are expected absence, replacement owner, replacement test, and docs status.

Eval must include code references, docs references, command-line entrypoints, configuration precedence, setup/onboarding references, and feature parity claims.

## Harness design

`harness/score.sh` must run:

- Static dependency checks for deleted symbols and imports.
- Build and test commands for affected Reborn crates.
- Grep/rg checks for retired entrypoints, config names, and docs references where graph tools cannot see text.
- Architecture tests or custom dependency lints banning new Reborn dependencies on legacy paths.
- Replacement behavior tests for each deletion group.

Calibrate with one known-bad deletion that removes tests or leaves references and confirm the scorer fails.

## Constraints

- Wall-clock budget: 10 hours.
- Spend ceilings: $5 LLM/API spend; no external service calls.
- Surface allowlist: legacy `src/` only for deletion or reference cleanup; Reborn crates only for replacement tests and minimal compatibility fixes; docs and parity files needed by the deletion ledger.
- Capacity caps: shared caps; deletion allowlist capped at the ledger entries only.
- Do not delete tests merely because they fail after deletion. Replace or retarget them when behavior still matters.

## Cycle protocol

Follow the common cycle protocol. Each cycle deletes or retires one coherent surface and updates the ledger. Before commit, run the targeted replacement tests and the deleted-symbol lint.

## Entropy rules

- Every third cycle must target a different artifact class: code, config, docs, tests, or binary/entrypoint references.
- If a deletion uncovers hidden replacement gaps, pause deletion and add Reborn replacement coverage before continuing.
- If the score improves only by shrinking the ledger, the next cycle must add missing checks instead of more deletion.

## Cheat audit

Lane-specific cheap wins to block:

1. Delete failing tests instead of obsolete behavior; scorer checks replacement coverage.
2. Move dead code to a new module; deleted-symbol lint follows aliases and imports.
3. Remove docs references without updating replacement docs; docs score requires replacement.
4. Leave config keys accepted silently; config checks assert retirement or migration.
5. Break v1 behavior still needed in production; ledger requires owner signoff and replacement test.
6. Add new behavior to `src/` while cleaning; lint rejects new legacy feature paths.
7. Mark parity complete without tests; `FEATURE_PARITY.md` changes require evidence.
8. Hide references in generated files; text checks include scripts and configs.
9. Delete only easy leaf files; ambiguous entrypoint metric requires entrypoint reduction.
10. Claim cleanup from line-count reduction; score is behavioral and reference-based.

## Stop conditions

Stop when all eligible ledger entries pass hard gates and weighted holdout is at least 0.90, budget is exhausted, score is flat for 3 cycles, or a deletion threatens unreplaced production behavior.

