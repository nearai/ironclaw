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
- **Persistence = both backends.** PostgreSQL + libSQL parity where production-facing; the model is the hooks trio (`hooks_postgres`/`hooks_libsql` + the `hooks_parity` adversarial equivalence crate). Feature-gate integration tests (`check-boundaries.sh` enforces the gating for root `tests/`).
- **The integration tier is NOT a PR gate unless the workflow says so** (re-verify: `grep -n integration .github/workflows/platform-and-compat.yml`): full `--features integration`/Postgres coverage may run post-merge or nightly. A green PR does not prove the integration tier ran; run it locally when your change is DB/runtime-shaped: `cargo test --features integration`.
- **Never add a silent self-skip to PR-gated tests.** `if docker_missing { return }` hides the suite from CI. Existing Docker sandbox canaries still need migration; new tests should skip loudly via feature gates or explicit env opt-outs.
- **Safe summaries and terminal errors have their own test shape** — capability handlers must route recoverable failures to `Ok(CapabilityOutcome::Failed(..))`, not `Err` (which kills the whole run), and safe summaries must survive the real validator; see `.claude/rules/agent-loop-capabilities.md` for the two invariants and how to test them past the helper.
- **A contract doc change needs its test named.** The house pattern: `docs/reborn/contracts/conversation-binding.md` names its test file + run command; `scripts/reborn-e2e-rust.sh` is the machine-readable contract→test map. If you implement contract behavior, wire both.

## Verify

`cargo test -p <crate>` → `cargo test --test reborn_<harness>` (offline) → `cargo test -p ironclaw_architecture` if edges changed → `cargo test --features integration` locally for DB-shaped changes → `bash scripts/reborn-e2e-rust.sh` when touching contract behavior.

**Exemplar tests to open and imitate, per tier**: [references/exemplar-tests.md](references/exemplar-tests.md) — the living copies; update as the suite evolves.
