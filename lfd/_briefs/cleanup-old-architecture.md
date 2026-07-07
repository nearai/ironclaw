# LFD Brief: cleanup-old-architecture — Clean up old architecture

**State**: migration tooling built (`ironclaw_reborn_migration`, dry-run,
MigrationReport with explicit gap set) but not wired into the CLI; six data
categories unmigrated; v1 + engine-v2 still in tree. **Bar**: 0.90 holdout
(migration fidelity). **Profile**: `migration`.

**Scope decision (deliberate)**: the scored target is MIGRATION
COMPLETENESS + CLI wiring, NOT code deletion. Deleting src/ is a stretch
goal listed in goal.md as unscored follow-up gated on full-suite green —
scoring deletion invites reckless removal; scoring fidelity does not.

## Outcome

`ironclaw-reborn migrate` (a real `Command::Migrate` subcommand) converts a
populated v1 database into Reborn substrate with verified fidelity across
ALL categories, closing the six documented gaps: non-cron triggers (Event/
Webhook/Manual), routine_runs history, settings KV, agent_jobs +
job_actions + job_events, memory_document_versions, and extension manifest
fidelity + WASM binary. Dry-run reports match reality; re-running is
idempotent.

## Spec sources

- `crates/ironclaw_reborn_migration/` (+ its MigrationReport gap set),
  `tests/migration_roundtrip.rs` (asserts the CURRENT exact gap set — the
  spec REDEFINES this test's expectation to shrink as gaps close)
- `docs/reborn/contracts/migration-compatibility.md`,
  `docs/plans/2026-06-25-cas-migration.md`
- v1 schemas: `src/db/` (+ its CLAUDE.md), `src/history/`
- LLM-data retention invariant: migration must never drop LLM output rows.

## Stage 0 inner suite

`migration_roundtrip` (updated per spec, red→green per category) +
`ironclaw_reborn_migration` crate tests + `Command::Migrate` CLI test.
Libsql backend for portability (Docker-free); Postgres variant where the
harness supports it.

## Eval shape (differs from other features)

Cases = SEEDED v1 DATABASES. `setup.profile_extra.seed` describes a v1
dataset (generator writes a seeding helper in the pinned profile support:
declarative seed spec → v1 libsql DB). The runner executes the migration
CLI path, then `state_queries` project the Reborn stores; contracts diff
them against sealed expected projections.

Dev ~30 / holdout ~10:
1. Per-category fidelity (14): one-category-focused seeds (conversations,
   cron triggers, non-cron triggers, routine_runs, settings KV, agent_jobs
   chain, memory docs + versions, secrets re-encryption, identities,
   extensions incl. WASM binary bytes) → projected Reborn state matches
   sealed expectation (state_eq on normalized projections; secrets assert
   metadata + decryptability, never plaintext echo).
2. Mixed realistic seeds (6): all categories together, cross-references
   intact (job→conversation links, trigger→routine identity).
3. Edge cases (5): unicode, oversized rows, dangling FKs, legacy enum
   values, empty DB → migrate cleanly or report typed per-row skips
   (forbidden: silent drop — report must enumerate every skipped row; a
   fail-loud contract).
4. Idempotency + dry-run honesty (5): dry-run report categories == wet-run
   result (state_eq between report and post-state); second run adds
   nothing (state_eq before/after).

Holdout seeds are structurally different datasets (new shapes, adversarial
edge mixes, one category-combination absent from dev).

## Feature-specific cheats → fences

- **Hardcode expected Reborn state from dev seeds** → holdout seeds differ
  structurally; probe shuffles ids/timestamps/contents via the map (seeds
  are inputs — the map transforms them AND the sealed expectations
  consistently).
- **Edit MigrationReport claims** → scorer never reads the report as
  truth; contracts diff ACTUAL stores; report honesty is itself a
  contract (theme 4).
- **Mark hard categories as skipped** → skips are enumerated per row and
  priced: category-fidelity contracts REQUIRE migrated content.
- **Relax roundtrip test to pass** → Stage-0 test-weakening caps
  (#[ignore]=0, deletions=0) + the test is named in goal.md as
  spec-governed: its expected gap set may only SHRINK.
- **Retention violation** (drop LLM rows to simplify) → forbidden matchers
  on missing conversation/job event rows in projections; retention
  invariant contracts on counts ≥ seed counts.

## caps.json extras

Dev seed literal values in `crates/**` diff: max 0. `DELETE FROM` /
`DROP TABLE` statement additions targeting v1 stores in migration code:
max 0 (migration reads v1, never mutates it — state_eq on v1 DB
before/after is also a contract in every case).

## Live mode

None. This loop is fully deterministic — no live cases; the $100 ceiling
is nominally unused (goal.md states spend 0 expected).
