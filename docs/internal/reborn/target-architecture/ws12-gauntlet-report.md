# WS12 Gauntlet Report — rows 3 and 4 (full gauntlet + backend parity)

- **Date:** 2026-08-05
- **Tree measured:** `0c6c0cfb9d853d941df78f48b273a1eba47a52ec` (the assembled `program-closure` batch tip = `origin/main` `b2023bc8fa` + the three Round-1 folds + coordinator fixups; verified equal to both local and `origin/program-closure` at run start)
- **Runner:** the `closure/ws12-gauntlet` agent (Round 2, final closure batch), branch `closure/ws12-gauntlet-work`
- **Toolchain:** rustc/cargo 1.96.0 (stable), Docker 29.6.0 (colima; socket `unix://$HOME/.colima/default/docker.sock`), node v24.18.0 + corepack pnpm@11.7.0 (frontend pin), Python 3.14.6, macOS 26.5.2 (darwin arm64)
- **Concurrency note:** the sibling `closure/ws12-security` agent ran on the same machine throughout; compile and wall-clock times below are contended, not representative.

## Method

Every suite ran with output captured to a file and the exit code read separately
(never piped through `head`/`tail`/`grep`), via a small runner script
(`target/run-bar.sh`, git-ignored). CI-mirroring env was applied per lane from
the workflows that own it (`reborn-tests.yml`, `code_style.yml`,
`platform-and-compat.yml`, `reborn-e2e.yml`): `IRONCLAW_DISABLE_OS_KEYCHAIN=1`,
`TZ=UTC`, `LANG=C.UTF-8`, empty LLM keys, `RUST_MIN_STACK=67108864` for the
root/group-bearing runs, `IRONCLAW_GENERATED_SEQUENCE_DEPTH=2`,
`PROPTEST_CASES=256`, and `DOCKER_HOST` pointed at the colima socket for every
testcontainers-consuming run. PostgreSQL for the URL-gated parity legs came from
a dedicated container started for this run (`pgvector/pgvector:pg16`, the same
image the hooks CI lane uses, on `127.0.0.1:15442`) and removed afterwards.

Classification: every non-zero exit is **REAL** (code/test wrong on this tree)
or **ENVIRONMENTAL** (infra: Docker socket, browser, sandbox, CPU-saturation
flake) — environmental only with the attempted-remedy evidence.

## Setup deviation (recorded)

The worktree bootstrap had cut this agent's worktree from `origin/main`
(`b2023bc8fa`), not the batch branch. The batch commit `0c6c0cfb9d…` was present
locally (tip of both `program-closure` and `origin/program-closure`);
`closure/ws12-gauntlet-work` was created directly at that SHA and
`git rev-parse HEAD` verified before any suite ran. No other deviation.

## Command table

Durations are contended wall-clock (sibling agent compiling throughout);
log-line counts from the captured per-bar logs. Every exit code was read from
the runner's separately-written `.exit` file, never a pipe.

| # | Command / lane | Exit | Duration | Log lines | Classification |
|---|---|---|---|---|---|
| 1 | `cargo fmt --all --check` | 0 | 11s | 0 | green |
| 2 | `cargo clippy --all --tests --examples -- -D warnings` | 0 | 106s | 699 | green |
| 2b | `cargo clippy --all --lib --bins -- -D warnings` (the #7119 push-lane shape) | 0 | 94s | 228 | green |
| 3 | `cargo clippy --all --tests --examples --all-features -- -D warnings` | 0 | 78s | ~700 | green |
| 4 | `cargo test --workspace --no-fail-fast` | 0 | ~42min | 25k+ | green — 495 targets / 15,203 passed / 63 ignored / 0 failed |
| 4b | crate-feature supplement (assistant/webui/host_runtime/hooks `--features test-support --all-targets`) | 0 | ~11min | — | green — 67 targets / 2,950 passed |
| 5 | `cargo test -p ironclaw_architecture_tests --no-fail-fast` | 0 | ~7min | — | green — 39 targets / 285 passed |
| 6 | `cargo test -p ironclaw_integration_tests --features integration --no-fail-fast` | 0 | ~35min | — | green — 100 targets / 1,665 passed / 53 ignored |
| 7 | QA recorded-fixture lane (scrub + guards + `reborn_qa_recorded_behavior`) | 0 | ~6min | — | green — 61 fixtures clean; 41 passed / 12 ignored |
| 8a | frontend `pnpm lint` (conventions + tsc) | 0 | 3s | 6 | green (after ENVIRONMENTAL pnpm-shadow retry, see §8) |
| 8b | frontend `pnpm test` (vitest, all suites) | 0 | ~3min | — | green — 126 files / 1,088 passed |
| 8c | frontend `pnpm build` (+ bundle budgets) | 0 | 1s* | 93 | green (*warm vite cache; real dist emitted, budgets checked) |
| 9a | e2e binaries (default + test-support SSO + openai-compat stamp) | 0 | ~6min | — | green |
| 9b | e2e browser block (smoke + sso + custom_mcp), hermetic wrapper | 0 | 107s | — | green — 50 passed |
| 9c | e2e responses block (21 manifest node IDs), hermetic wrapper | 0 | 6s | — | green — 21 passed |
| 9d | e2e blackbox smoke, hermetic wrapper | 0 | 8s | — | green — 5 passed |
| 10 | scripts/ci self-test battery (41 entries) | 0 | ~9min | — | green — 38 first-pass + 2 ENVIRONMENTAL-retried-green (bash 3.2) + 1 battery invocation artifact |
| P1 | fabric parity (`ironclaw_filesystem --all-targets`) | 0 | ~4min | — | green — 297 passed; 57 pg + 81 libsql legs ran |
| P2 | triggers parity (`repository_contract`, REQUIRE_POSTGRES) | 0 | ~3min | — | green — 52 passed |
| P3 | hooks parity (5 targets, `integration,test-support`, REQUIRE_POSTGRES) | 0 | ~5min | — | green — 58+ passed across 5 binaries, all 3 backends |
| P4 | composition parity (`test-support,memory-mem0 --all-targets`) | 0 | ~18min | — | green — 896 passed / 1 ignored |
| P5 | processes journal parity (2 contracts) | 0 | ~2min | — | green — 49 passed |
| P6 | event-store parity (2 contracts) | 101 | 3 runs | — | **REAL (pre-existing test-isolation defect, Postgres leg)** — see Row 4 §P6 |
| P7 | extension-registry installations (virgin DB) | 0 | ~2min | — | green — 31 passed |
| P8 | assistant ledger parity (virgin DB) | 101 | 4 runs | — | **REAL (same defect class as P6)** — see Row 4 §P8 |
| P9 | host-runtime libSQL restart lane | 0 | ~2min | — | green — 4 passed |
| P10 | integration backend matrix | 0 | ~4min | — | green — 19 passed |

## Row 3 — per-suite results

### 1. `cargo fmt --all --check`
EXIT=0, 11s, empty output. Clean.

### 2. `cargo clippy --all --tests --examples -- -D warnings` (default lane)
EXIT=0, 106s (warm shared deps), 699 log lines, `Finished` with zero warnings.

### 3. `cargo clippy --all --tests --examples --all-features -- -D warnings`
EXIT=0, 78s, zero warnings. WebUI frontend deps were provisioned first
(`corepack pnpm install --frozen-lockfile`, lockfile already satisfied), matching
the CI lane's ordering (`code_style.yml` installs frontend deps before the
all-features clippy because `ironclaw_webui`'s build script compiles the SPA).

### 4. `cargo test --workspace --no-fail-fast` (workspace test set)
EXIT=0 — **495 test targets, 15,203 passed, 63 ignored, 0 failed** (aggregated
from the per-target `test result:` lines; zero `test result: FAILED`).
Env: CI-mirrored (see Method) + `DOCKER_HOST` at the colima socket so the
testcontainers-based suites could provision PostgreSQL.
The characterized CPU-saturation flake
(`smoke::onboard_login_link_then_bearer_authorizes_a_protected_request`,
documented at `crates/app/ironclaw_cli/tests/smoke.rs:3132`) **passed on the
first run** despite the sibling agent compiling concurrently — no retry needed.
Note: URL-gated per-domain Postgres legs (`IRONCLAW_*_POSTGRES_URL` families)
skip-return inside this bar by design; they are forced and verified in the
Row-4 parity bars below.

### 5. `cargo test -p ironclaw_architecture_tests --no-fail-fast` (full arch suite)
EXIT=0 — **39 binaries, 285 tests passed, 0 ignored, 0 failed.**

### 8. Frontend suites (`crates/product/ironclaw_webui/frontend`, CI = `code_style.yml` webui-v2-js-lint job)
Provisioning: corepack-pinned `pnpm@11.7.0` with a worktree-local shim dir
(local equivalent of CI's `corepack enable pnpm`; first attempt failed
ENVIRONMENTALLY because a system pnpm 11.8.0 shadowed the pin for inner `pnpm`
invocations — resolved by the shim dir, recorded below).
- `pnpm lint` (lint:conventions + `tsc --noEmit`): EXIT=0. Typecheck verified
  real via `--extendedDiagnostics`: 1,588 files / 101,159 LoC TS checked.
- `pnpm test` (vitest, ALL suites): EXIT=0 — **126 test files, 1,088 tests, all passed.**
- `pnpm build` (vite build + `check-bundle-budgets`): EXIT=0 — bundle budgets
  passed (login 130.0 KB gzip, /chat 215.4 KB gzip, largest chunk 435.6 KB raw,
  all inside headroom).
Note: local node is v24.18.0 vs the frontend's `engines` want of `>=22.22 <23`
(warn-only; CI's vitest lane runs node 22, the reborn-e2e smoke job runs node 24).

### 6. Integration lanes — `cargo test -p ironclaw_integration_tests --features integration --no-fail-fast`
EXIT=0 — **100 test targets (all flat/int-tier, group, auth-folder, root parity
and QA bins of the workspace-root package), 1,665 passed, 53 ignored, 0
failed.** Two readings of "integration lanes" both hold on this tree:
(a) the root package's `integration` cargo feature is declared `integration = []`
(root `Cargo.toml:233`) with **zero** `required-features`/`cfg` consumers under
`tests/` — the root `CLAUDE.md` line "`cargo test --features integration` # +
PostgreSQL tests" is stale vocabulary for what are now testcontainers-backed
crate suites (Row 4); and (b) this `-p`-selected run is *not* redundant with
bar 4: resolver-v2 feature unification for a single selected package matches
the shape CI's per-suite integration lanes use
(`cargo test -p ironclaw_integration_tests --test <name>`,
`scripts/ci/reborn-coverage-lane-run.sh`), and the dependency closure was
recompiled and re-run green under that narrower feature set.

### 7. Recorded-fixture QA lane (CI = `reborn-tests.yml` qa-recorded-fixtures job, mirrored step-for-step)
EXIT=0 across all four stages:
- scrub self-test (`test-check-reborn-qa-fixtures.sh`): pass
- real fixture scrub (`check-reborn-qa-fixtures.sh`): **61 fixture files, clean**
- promotion guards (`python3 -m unittest scripts/ci/test-check-regression-promotions.py`): OK
- replay: `cargo test -p ironclaw_integration_tests --test reborn_qa_recorded_behavior -- --nocapture`
  → **41 passed, 12 ignored, 0 failed** (the 12 ignored are the suite's
  documented live-only canaries).

### 4b. Crate-feature supplement (CI crate-bucket flag parity)
`cargo test --workspace` (bar 4) builds every crate at workspace-unified
features; CI's crate buckets additionally pin per-package flags
(`scripts/ci/package-feature-flags.sh`). The four packages whose CI flags add
surface beyond bar 4 were re-run exactly as CI does —
`ironclaw_assistant`, `ironclaw_webui`, `ironclaw_host_runtime`,
`ironclaw_hooks`, each `--features test-support --all-targets`
(`ironclaw_composition`'s `test-support,memory-mem0` runs as parity bar P4;
`ironclaw_hooks`' `integration` lane as P3): EXIT=0 —
**67 targets, 2,950 passed, 0 ignored, 0 failed.**

### 10. `scripts/ci` self-test battery (every self-test + the two named gates)

41 entries run (the union of every `scripts/ci/test-*.sh` / `test_*.py` /
`test-*.py`, the CI-mirrored companions from `code_style.yml` and
`reborn-tests.yml` — `check_no_panics.py --self-test`/`--reborn-baseline`,
`test_run_live_qa` (204-test module), `.github/scripts/test-pr-labeler.sh`,
`tests/test_smoke_release_binary.py`, `scripts/test_dev_metrics.py`,
`check-wasm-artifact-freshness.py`, `check-include-str-paths.sh`,
`reborn_changed_coverage.py --validate-manifest-only`,
`check-test-suite-boundaries.sh` — and the two mission-named gates
`bash scripts/ci/check-composition-budget.sh` and
`python3 scripts/ci/check-target-tree.py`).

**Final state: all green.** Detail:

- 38/41 EXIT=0 on the first pass (including both named gates —
  `check-target-tree.py` reports the §5 steady-state equality:
  64 members / 64 documented / 1 exclusion / 0 exceptions).
- 2 first-pass failures were **ENVIRONMENTAL (macOS bash 3.2)** and **pass on
  retry under bash 5**, the CI shape (ubuntu): `check-test-suite-boundaries.sh`
  (EXIT=127, `line 54: mapfile: command not found`) and
  `test-reborn-coverage.sh` (EXIT=1: its C-section comment-script cases exited
  127 because `scripts/ci/reborn-coverage-comment.sh:66` uses `mapfile`).
  Remedy evidence: installed bash 5.3.15 via homebrew, reran both →
  `check-test-suite-boundaries.sh` EXIT=0; `test-reborn-coverage.sh`
  **194 of 194 cases passed**, EXIT=0. Recorded as passes per the retry rule.
  *Observation for the coordinator (not a REAL failure — CI's contract is
  ubuntu/bash 5):* these two scripts are the only ones in the battery not
  runnable under macOS /bin/bash 3.2; `reborn-coverage-int-tier-tests.sh`
  documents the repo's macOS-portability intent ("macOS dev machines ship
  bash 3.2") that these two predate or missed.
- 1 first-pass 127 was **an invocation artifact of this battery**, not a suite
  failure: `test-reborn-coverage-ratchet-cases.sh` is a *sourced case library*
  (its `assert_*`/`capture` helpers live in the driver), not a standalone
  self-test; its R-section cases all run and pass inside
  `test-reborn-coverage.sh`'s 194.

## Row 4 — backend parity per fabric-routed domain

### How the parity surface was derived

Per `crates/substrates/ironclaw_filesystem/CLAUDE.md` (one trait, one fabric;
backends `PostgresRootFilesystem` / `LibSqlRootFilesystem` / `InMemoryBackend` /
`DiskFilesystem` / `HsmBackend`) and `.claude/rules/database.md` (shared
conformance suites where a domain supports multiple durable backends), the
tree's actual dual-durable-backend surface was enumerated by inspection —
every crate test using `LibSql*`/`Postgres*` types or a `*_POSTGRES_URL` /
`DATABASE_URL` gate (the complete env inventory on this tree:
`IRONCLAW_FILESYSTEM_POSTGRES_URL`, `IRONCLAW_HOOKS_POSTGRES_URL`,
`IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL`,
`IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL`, plus the production config keys
`IRONCLAW_REBORN_POSTGRES_URL`/`IRONCLAW_REBORN_CUSTOM_POSTGRES_URL` which are
not test gates). Domains whose stores are backend-neutral by design delegate
backend correctness to the fabric contract (P1) — that architecture is stated
in the suites themselves (e.g.
`ironclaw_identity/tests/project_repository_contract.rs` header: "backend
correctness (Postgres / libSQL / JSONL)" lives at the fabric;
`ironclaw_outbound/tests/outbound_state_store_contract.rs:442`: the legacy
per-backend stores were deleted in the fabric convergence).

Postgres for the URL-gated legs: dedicated `pgvector/pgvector:pg16` container
(the hooks CI lane's image) at `127.0.0.1:15442`, database `ironclaw_test`;
testcontainers-based legs self-provision `postgres:16-alpine` through the
colima socket. Every parity bar ran with `-- --nocapture` so a silent
skip-return would be VISIBLE in the log; each Postgres-side verdict below
includes the count of postgres-named tests that actually ran, and
`IRONCLAW_REQUIRE_POSTGRES=1` was set for the two suites that honor it
(triggers, hooks — their ADRs added the hard-require switch precisely because
these legs once skipped green).

### Parity runs

**P1 — the fabric itself (`ironclaw_filesystem`), the parity holder for every
fabric-delegated domain.** `cargo test -p ironclaw_filesystem --all-targets`
under the parity env: EXIT=0 — 6 targets, **297 passed, 0 failed, 0 skip
lines**; **57 postgres-leg tests ran** (contract + `postgres_delete_if_version_race`
+ `concurrent_cas_storm` legs) and **81 libsql-leg tests ran**.

**P2 — triggers (`ironclaw_triggers`, ADR 0003's permanent hand-written-SQL
exception).** `cargo test -p ironclaw_triggers --test repository_contract` with
`IRONCLAW_REQUIRE_POSTGRES=1` (the ADR's hard-require switch, honored by this
suite) + testcontainers via the colima socket: EXIT=0 — **52 passed, 0 failed,
0 skip lines**; the postgres/libsql aggregate parity drivers
(`postgres_repository_contract_parity`, `libsql_repository_contract_parity`,
`assert_durable_fire_claim_contract` bundles) all ran. With REQUIRE set, a
skipped Postgres leg is a hard failure by construction — it wasn't.

**P3 — hooks (`ironclaw_hooks`, ADR 0004's staged dual backends).** The
`platform-and-compat.yml` hooks-parity invocation verbatim
(`--features integration,test-support`, five `--test` targets) with
`DATABASE_URL` at the dedicated pgvector/pg16 container and
`IRONCLAW_REQUIRE_POSTGRES=1`: EXIT=0, 0 skip lines —
`parity_matrix` 6 (in-memory × libSQL × Postgres cross-asserted against the
hand-computed oracle), `multi_host_adversarial` 12 (6 `postgres_cluster::*` +
6 libsql, all visibly ran), `predicate_state_postgres_contract` 12,
`predicate_state_postgres_adversarial` 7, `predicate_state_libsql_contract`
21+ (its per-test lines and its own "all predicate_state_contract cases ok"
summary are in the log; the libtest summary line itself was lost to
`--nocapture` interleaving — cargo's overall exit 0 requires that binary to
have exited 0, so this is a log artifact, not a gap).

**P4 — composition substrate acceptance (`ironclaw_composition`,
`--features test-support,memory-mem0 --all-targets`, the CI crate-bucket
flags).** EXIT=0 — **34 targets, 896 passed, 1 ignored, 0 failed**; the
substrate-acceptance legs visibly ran: 10 postgres-named
(`postgres_substrate.rs` + friends, testcontainers-provisioned), 23
libsql-named (`libsql_substrate.rs`, `resource_governor_libsql_contract.rs`),
9 mem0-named (the memory-mem0 factory/swap tests).

**P5 — processes journal (`ironclaw_processes`).**
`--test process_journal_store_contract --test legacy_migration_backend_contract`:
EXIT=0 — **49 passed (47 + 2), 0 failed, 0 skip lines**; the postgres leg
(legacy-migration backend contract over `PostgresRootFilesystem`) and libsql
legs both ran. The journal contract proper runs over the fabric
(backend-neutral); its Postgres correctness is held by P1.

**P6 — event store (`ironclaw_event_store`) — THE FIRST OF THIS GAUNTLET'S
TWO REAL FAILURES, ONE DEFECT CLASS (pre-existing, test-design, not a store
defect; the second instance is P8).**
`--test durable_event_store_contract --test profile_contract` under the parity
env: `profile_contract` 8/8 ok; `durable_event_store_contract` **11 passed,
2 FAILED** — `postgres_replay_advances_next_cursor_past_trailing_filtered_records`
and `postgres_runtime_and_audit_logs_survive_rebuild_with_filtered_cursor_semantics`.
Full diagnosis, each step evidenced in the captured logs:
- First run (shared parity DB): asserts failed `EventCursor(39)`/`EventCursor(38)`
  vs expected `1` — residue-shaped.
- Retry on a **virgin database**: still red — `2` vs `1`, `5` vs `3` — the two
  postgres tests interleave on the ONE global cursor sequence under libtest's
  default parallelism.
- Serial (`--test-threads=1`) on a recreated virgin database: **first postgres
  test passes; second still red (`3` vs `1`)** — it inherits the first's rows.
- Each failing test run **alone** on a virgin database: **passes** (1 passed).
Verdict: the Postgres store's cursor semantics are **correct** (every
assertion failure is exactly the other test's appends); the suite's Postgres
leg is **not self-isolating** — the tests assert *absolute* global cursor
values against the single database named by
`IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL` (the unique scope suffixes isolate
record *filtering* but not the global cursor), while the jsonl/libsql twins
isolate per-test (temp files / per-test DBs) and pass. Classification:
**REAL — a defect in the test suite's Postgres-leg isolation**, with three
mitigating provenance facts: (1) the file is **byte-identical to
`origin/main`** (`git diff origin/main -- …durable_event_store_contract.rs`
empty; last touch WS7's text-only family move) — not a batch regression;
(2) **no CI lane sets `IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL`** (repo-wide
grep of `.github/` + `scripts/`), so the leg has no executor in CI and skips
in every other bar of this gauntlet; (3) parity *semantics* for the domain are
individually proven per-test. Fix shape (owner's call, not applied here — the
verify-only mandate): per-test isolated databases (the pattern
`ironclaw_filesystem`'s contract already uses) or baseline-relative cursor
assertions.

**P7 — extension registry installations (`ironclaw_extension_registry`,
`--test installations_contract`), on its own virgin database.** EXIT=0 —
**31 passed, 0 failed, 0 skip lines**; the postgres-backed and libsql-backed
durable legs both visibly ran.

**P8 — assistant durable ledger (`ironclaw_assistant --features test-support
--test durable_ledger_contract`) — SECOND INSTANCE OF THE SAME REAL
(pre-existing) TEST-ISOLATION DEFECT CLASS AS P6.** Under the parity env on a
virgin database: **18 passed, 2 FAILED** —
`postgres_settled_entry_limit_prunes_oldest_when_configured` and
`postgres_settled_prune_interval_defers_until_interval_when_configured`
(assert "oldest was pruned and can reserve again" →
`IdempotencyDecision::New`). Evidence chain:
- 8 postgres-leg tests ran (no skips); the two prune tests fail while their
  libsql twins pass (libsql legs get per-test temp databases).
- Whole suite **serial** (`--test-threads=1`) on a **virgin** database: same 2
  failures — accumulated-state, not parallelism (sibling tests' settled
  entries land in the shared database the prune bookkeeping then sees).
- **Each failing test alone on a virgin database: passes** (both re-proven).
- File **identical to `origin/main`** (empty `git diff --stat`); **no CI lane
  sets `IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL`** (repo-wide grep) — latent,
  no executor, not a batch regression.
Classification: **REAL — Postgres-leg test isolation** (suite-level), ledger
Postgres semantics individually proven per-test. Same fix shape as P6.

**P9 — host-runtime durable restart (libSQL; the `code_style.yml`
reborn-cli-smoke named lane).** `cargo test -p ironclaw_host_runtime
--features test-support --test reborn_durable_restart_integration`: EXIT=0 —
**4 passed, 0 failed.**

**P10 — integration-tier backend matrix
(`reborn_integration_backend_matrix`, whole-turn behavior parity InMemory ×
libSQL).** EXIT=0 — **19 passed, 0 failed.**

### Per-domain × backend table

Legend: **green** = suite ran and passed with the leg's tests demonstrably
executed (counted from `--nocapture` logs; skip messages would be visible and
none appeared, and REQUIRE_POSTGRES hard-fails skips where honored).
**fabric-delegated** = the domain's store is backend-neutral by design; its
Postgres/libSQL correctness is held by the fabric contract (P1) — the
documented convergence architecture, not a silent skip.

| Domain (crate) | Suite | libSQL | PostgreSQL | Note |
|---|---|---|---|---|
| storage fabric (`ironclaw_filesystem`) | `db_root_filesystem_contract` + `postgres_delete_if_version_race` + `concurrent_cas_storm` | green (81 tests) | green (57 tests) | P1 — the parity holder for every fabric-delegated domain |
| triggers (`ironclaw_triggers`, ADR 0003) | `repository_contract` (51-case shared conformance) | green | green (REQUIRE enforced) | P2 |
| hooks (`ironclaw_hooks`, ADR 0004) | `parity_matrix` + `multi_host_adversarial` + 3 predicate-state contracts | green | green (REQUIRE enforced) | P3 — in-memory × libSQL × Postgres cross-asserted vs oracle |
| composition substrate (`ironclaw_composition`) | `libsql_substrate` / `postgres_substrate` / `profile_acceptance` / `resource_governor_libsql_contract` | green (23) | green (10, testcontainers) | P4 (+ mem0 factory 9) |
| processes journal (`ironclaw_processes`) | `process_journal_store_contract` + `legacy_migration_backend_contract` | green | green (legacy-migration leg; journal proper is fabric-delegated) | P5 |
| event store (`ironclaw_event_store`) | `durable_event_store_contract` + `profile_contract` | green | **RED as a suite** — 2 tests pass only individually (REAL pre-existing isolation defect, see P6) | P6 — the row-4 blocker |
| extension installations (`ironclaw_extension_registry`) | `installations_contract` | green | green (virgin DB) | P7 |
| assistant durable ledger (`ironclaw_assistant`) | `durable_ledger_contract` | green | **RED as a suite** — 2 prune tests pass only individually (same defect class, see P8) | P8 — the row-4 blocker (2nd instance) |
| host-runtime restart (`ironclaw_host_runtime`) | `reborn_durable_restart_integration` | green | n/a (libSQL-only lane by design; CI-named) | P9 |
| whole-turn behavior (`tests/integration/backend_matrix.rs`) | `reborn_integration_backend_matrix` | green | n/a (InMemory × libSQL matrix by design) | P10 |
| threads (`ironclaw_threads`) | `filesystem_session_thread_contract` | green (in bars 4/6) | fabric-delegated (P1) | backend-neutral store |
| identity (`ironclaw_identity`) | `project_repository_contract` (header: backend correctness lives at the fabric) | in-memory | fabric-delegated (P1) | backend-neutral store |
| outbound (`ironclaw_outbound`) | `outbound_state_store_contract` (legacy per-backend stores deleted, `:442`) | via fabric | fabric-delegated (P1) | convergence documented in-suite |
| secrets (`ironclaw_secrets`) | `secret_store_contract` + `boundary_contract` | via fabric | fabric-delegated (P1) | backend-neutral store |
| conversations (`ironclaw_conversations`) | `conversation_state_store_contract` + `inbound_contract` | via fabric | fabric-delegated (P1) | backend-neutral store |
| approvals (`ironclaw_approvals`) | 5 contract suites (store/resolution/gate-record/boundary) | via fabric | fabric-delegated (P1) | backend-neutral store |
| memory (`ironclaw_memory`) | crate tests + `group_memory` scenarios | via fabric | fabric-delegated (P1) | backend-neutral store (+ mem0 lane in P4) |
| auth (`ironclaw_auth`) | `auth_product_contract` + `auth_engine_contract` + `test_support::conformance` | via fabric | fabric-delegated (P1) | backend-neutral store |

Reported gap (not a silent skip): the fabric-delegated rows have **no
domain-named Postgres-side suite** — by the tree's own architecture (one
fabric, backend-neutral domains), their Postgres correctness rests entirely on
P1 plus the domain contract over the fabric interface. That is the designed
coverage shape (`.claude/rules/database.md` + the convergence notes in the
suites themselves); it is listed here so the row's reviewer sees exactly which
domains rest on the P1 keystone rather than on a suite of their own.

### 9. e2e smoke (CI = `reborn-e2e.yml` browser lane, mirrored)

Provisioning: `tests/e2e/.venv` (Python 3.14.6) + `pip install -e .` +
`playwright install chromium`; binaries built exactly as CI's smoke job
(default `target/debug/ironclaw`, `--features test-support` copy at
`target/e2e-sso/debug/ironclaw`, `.ironclaw-reborn-openai-compat.stamp`).
All three pytest blocks ran through the repo's own hermetic wrapper
(`run-hermetic-deterministic-suite.sh command`, `PLAYWRIGHT_BROWSERS_PATH`
pinned), exactly like the CI browser lane:
- smoke + sso + custom_mcp: **50 passed** (106.57s)
- responses manifest (21 node IDs from `reborn_responses_e2e_tests.txt`): **21 passed**
- blackbox smoke: **5 passed**
The Emulate-backed provider lanes are outside the smoke subset (they require
the pinned `serrrfirat/emulate` build via `IRONCLAW_EMULATE_CLI`) and were not
run — the mission's smoke scope is the browser lane, which is fully green.

## REAL vs ENVIRONMENTAL summary

**REAL (2 findings — one defect class, two instances; both Row 4, both
pre-existing on `origin/main`, both CI-unreachable today):**
1. `ironclaw_event_store/tests/durable_event_store_contract.rs` — the two
   `postgres_*` tests assert absolute global-cursor values against the single
   shared `IRONCLAW_REBORN_EVENT_STORE_POSTGRES_URL` database; the suite
   cannot pass as one invocation (parallel or serial, even on a virgin
   database). Each test passes alone on a virgin database — store semantics
   correct, test isolation defective. File byte-identical to `origin/main`;
   no CI lane sets the env var.
2. `ironclaw_assistant/tests/durable_ledger_contract.rs` — the two
   `postgres_settled_*prune*` tests fail from sibling tests' accumulated
   state in the shared `IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL` database
   (fails serially on a virgin DB too); each passes alone on a virgin
   database. Same provenance: identical to `origin/main`, no CI executor.

Consequence: **Row 3 ticks** (every named gauntlet suite green; the two REAL
findings live in Postgres legs that no Row-3 lane executes). **Row 4 stays
open** on these two suites — parity semantics are individually proven for
both domains, but the row demands green *suites* on both backends and these
two cannot go green as-written. Fix shape is small and test-only (per-test
isolated databases, the `ironclaw_filesystem` contract's existing pattern, or
baseline-relative assertions) — the owner's call, out of scope for this
verify-only pass.

**ENVIRONMENTAL (all resolved with retry evidence, recorded passes):**
- Frontend first `pnpm lint` attempt: system pnpm 11.8.0 shadowed the
  corepack-pinned 11.7.0 for inner `pnpm` invocations (macOS PATH artifact;
  CI's global `corepack enable` doesn't split versions). Fixed with a
  worktree-local corepack shim dir; all three frontend gates green.
- `check-test-suite-boundaries.sh` + `test-reborn-coverage.sh` under macOS
  `/bin/bash` 3.2: `mapfile` is bash-4+ (`check-test-suite-boundaries.sh:54`,
  `reborn-coverage-comment.sh:66`). Both green under bash 5.3 (the CI shape):
  boundaries EXIT=0, coverage harness 194/194.
- P6's *first* failure shape (cursors 39/38) additionally reflected this
  run's shared parity database; eliminated by virgin-database retries, which
  is what isolated the REAL finding above.

**Characterized flake:** not hit —
`smoke::onboard_login_link_then_bearer_authorizes_a_protected_request`
(`crates/app/ironclaw_cli/tests/smoke.rs:3132`) passed on the first attempt
despite sibling-agent CPU contention.

**Not run (scope):** the Emulate-backed e2e provider lanes (outside the smoke
subset; they require the pinned `serrrfirat/emulate` build).

**Run beyond the mission's list:** `cargo clippy --all --lib --bins -- -D
warnings` (the #7119 no-dev-deps production-target shape `code_style.yml`
requires on push): EXIT=0, 94s — green.

## Cleanup

The dedicated Postgres container (`ws12-gauntlet-pg`) and its databases were
removed after the run. Logs live only under the git-ignored
`target/gauntlet-logs/` of the run worktree; nothing outside the worktree was
modified except the homebrew `bash` formula installation (additive).
