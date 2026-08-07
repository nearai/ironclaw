---
name: ironclaw-reborn-testing
description: Use when adding or reviewing tests for Reborn behavior — choosing a test tier, covering a bug fix, testing model/tool-choice behavior, touching tests/integration or tests/fixtures/llm_traces, or when a test needs Postgres, Docker, or a live LLM.
---

# Reborn Testing

Pick the tier first; everything else follows. The repo's tier knowledge lives in `tests/integration/CLAUDE.md` (read it before writing harness tests); this skill is the decision layer plus the traps.

## Tier decision tree

1. **Pure logic, no gated side effect** → unit test in the crate (`mod tests` / crate `tests/`).
2. **A helper gates a side effect (HTTP, DB write, egress body, approval, dispatch)** → you also need a caller-path test driving the real entry point (`*_handler`, facade method, adapter, coordinator). Helper-only is insufficient — `.claude/rules/testing.md` has the bug catalog. Gold standard: `tests/integration/group_approvals/scenario_gate_then_approve.rs` asserts the approved write **exists on disk**.
3. **Whole-turn Reborn behavior (submit → runner → loop → reply), deterministic** → the in-process scripted-model harness (`tests/integration/`, run as `cargo test --test reborn_integration_<name>`, zero setup, offline). **Mock only at the vendor-SDK seam** (`TraceLlm`): the real `ironclaw_llm` decorator chain (retry/failover/circuit-breaker) must execute. Mocking at the gateway seam skips it — that's the gateway-seam replay tier's job (`RebornBinaryE2EHarness` / `RebornTraceReplayModelGateway`), not yours by default.
4. **Model tool-choice / request-shape is the behavior under test** → recorded QA fixtures (`tests/fixtures/llm_traces/reborn_qa/` + `tests/reborn_qa_recorded_behavior.rs`: ignored live recorder → hermetic contract assertions → hermetic replay). Fixtures must pass `scripts/ci/check-reborn-qa-fixtures.sh` (secret/PII scrub). Never commit unscrubbed traces.
5. **Browser-visible** → `tests/e2e/` Playwright (`reborn_v2_*` fixtures for WebChat v2). **Live LLM** → `#[ignore]` canary tier; supplemental only, never the PR gate.

## Repo-specific traps

- **Regression-per-fix is mechanically checked for conventionally marked fix/high-risk changes** (commit-msg hook + `regression-test-check.yml`). Escape hatch `[skip-regression-check]` exists — using it on a real fix will be questioned in review.
- **Consolidate, don't proliferate**: extend the existing test that already drives the path (a case, a scripted turn, an assertion) before standing up a new file. Say why an existing test couldn't absorb a genuinely new scenario.
- **Persistence = both backends.** PostgreSQL + libSQL parity where production-facing; the model is `ironclaw_hooks`' dual-backend shape (`crates/loop/ironclaw_hooks/src/postgres_backend/` + `crates/loop/ironclaw_hooks/src/libsql_backend/`, proved equivalent by `crates/loop/ironclaw_hooks/tests/parity_matrix.rs` and `crates/loop/ironclaw_hooks/tests/multi_host_adversarial.rs`). Feature-gate integration tests.
- **The backend-integration tier is NOT a PR gate unless the workflow says so** (re-verify: `grep -n integration .github/workflows/platform-and-compat.yml`): full Postgres coverage may run post-merge or nightly. A green PR does not prove the tier ran; run it locally when your change is DB/runtime-shaped — crate-level, e.g. `cargo test -p ironclaw_hooks --features integration,test-support` (the workspace-root `integration` feature is empty; a bare root `cargo test --features integration` runs nothing extra).
- **Never add a silent self-skip to PR-gated tests.** `if docker_missing { return }` hides the suite from CI. Existing Docker sandbox canaries still need migration; new tests should skip loudly via feature gates or explicit env opt-outs.
- **Capability results and terminal errors have their own test shape** — recoverable failures must remain model-visible outcomes, while host failures are terminal; test the caller against the capability-access contract and its real redaction/evidence validation.
- **A contract doc change needs its test named.** The house pattern: `docs/reborn/contracts/conversation-binding.md` names its test file + run command; `scripts/reborn-e2e-rust.sh` is the machine-readable contract→test map. If you implement contract behavior, wire both.

## Verify

`cargo test -p <crate>` → `cargo test --test reborn_<harness>` (offline) → `cargo test -p ironclaw_architecture_tests` if edges changed → the owning crate's feature-gated suite (`cargo test -p <crate> --features integration`) locally for DB-shaped changes → `bash scripts/reborn-e2e-rust.sh` when touching contract behavior.

**Exemplar tests to open and imitate, per tier**: [references/exemplar-tests.md](references/exemplar-tests.md) — the living copies; update as the suite evolves.
