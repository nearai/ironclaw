# Best-Effort sccache Installation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent transient `sccache` release-download failures from failing CI lanes before their tests run.

**Architecture:** The shared composite action continues to use the pinned upstream installer and its retries. Only installer failure is tolerated; later configuration runs exclusively after a successful install and retains its current strict validation.

**Tech Stack:** GitHub composite actions, Python `unittest` CI contract tests, repository CI validation scripts.

## Global Constraints

- `sccache` remains an optional performance optimization.
- Invalid cache credentials or configuration remain fatal after installation succeeds.
- Successful cache setup behavior and all callers remain unchanged.
- No new dependency or Cargo feature is introduced.

---

### Task 1: Pin the fail-open installer contract

**Files:**
- Modify: `scripts/ci/ws12_workflow_contracts.py`
- Modify: `scripts/ci/test_ws12_workflow_contracts.py`

**Interfaces:**
- Consumes: `.github/actions/setup-sccache-dist/action.yml` as checked-in text.
- Produces: `validate_sccache_setup_action(text: str) -> list[str]`, wired into the top-level workflow contract validation.

- [ ] **Step 1: Write sabotage tests for installer tolerance, configuration gating, and fallback warning**

Add tests that mutate each required behavior independently and assert that `validate_sccache_setup_action` reports the broken contract.

- [ ] **Step 2: Run the focused tests to verify RED**

Run: `python3 scripts/ci/test_ws12_workflow_contracts.py`

Expected: FAIL because `validate_sccache_setup_action` and the required action behavior do not yet exist.

- [ ] **Step 3: Add the minimal validator interface**

Load `.github/actions/setup-sccache-dist/action.yml` alongside workflow files, validate the three behavior markers, and return actionable errors through `validate_workflow_texts`.

- [ ] **Step 4: Re-run the focused tests**

Run: `python3 scripts/ci/test_ws12_workflow_contracts.py`

Expected: The checked-in-action test remains red until Task 2; sabotage tests exercise the new validator.

### Task 2: Make installation best-effort

**Files:**
- Modify: `.github/actions/setup-sccache-dist/action.yml`

**Interfaces:**
- Consumes: the existing cache-credential availability condition.
- Produces: step outcome `steps.install_sccache.outcome` for configuration and fallback routing.

- [ ] **Step 1: Mark the installer step with `id: install_sccache` and `continue-on-error: true`**

- [ ] **Step 2: Require `steps.install_sccache.outcome == 'success'` before configuration**

- [ ] **Step 3: Add a warning step for `steps.install_sccache.outcome == 'failure'` that states local compilation will be used**

- [ ] **Step 4: Run the focused tests to verify GREEN**

Run: `python3 scripts/ci/test_ws12_workflow_contracts.py`

Expected: PASS.

### Task 3: Verify and publish

**Files:**
- Verify the complete branch diff.

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces: a draft GitHub pull request against `main`.

- [ ] **Step 1: Run focused repository verification**

Run:

```bash
python3 scripts/ci/test_ws12_workflow_contracts.py
python3 scripts/ci/ws12_workflow_contracts.py
python3 scripts/ci/docs_publication_boundary.py
```

Expected: all commands exit 0.

- [ ] **Step 2: Run GitHub Actions syntax validation when available**

Run: `actionlint`

Expected: exit 0; if unavailable, report that limitation explicitly.

- [ ] **Step 3: Inspect the diff and working tree**

Run: `git diff --check && git status --short && git diff --stat && git diff`

Expected: only the composite action, its contract tests, and these internal design/plan files are changed.

- [ ] **Step 4: Commit, push, and open a draft PR**

Commit message: `ci: tolerate sccache install outages`

PR title: `ci: tolerate sccache install outages`

### Task 4: Classify the shared action in the Reborn PR planner

**Files:**
- Modify: `scripts/ci/reborn_pr_test_plan.py`
- Modify: `scripts/ci/test_reborn_pr_test_plan.py`

**Interfaces:**
- Consumes: changed path `.github/actions/setup-sccache-dist/**`.
- Produces: an exhaustive Reborn test plan for the shared action while other unmapped local actions remain fail-closed.

- [ ] **Step 1: Add a regression test that reproduces the PR failure**

Assert that the sccache action selects `mode=full` and that an unrelated local action still raises `unmapped test or CI path`.

- [ ] **Step 2: Run the focused test to verify RED**

Run: `python3 -m unittest scripts.ci.test_reborn_pr_test_plan.RebornPrTestPlanTests.test_shared_sccache_action_widens_to_exhaustive_plan`

Expected: FAIL with `unmapped test or CI path: .github/actions/setup-sccache-dist/action.yml`.

- [ ] **Step 3: Add the minimal path classification**

Recognize only `.github/actions/setup-sccache-dist/**`, finish validating the rest of the changed paths, and then return the exhaustive plan.

- [ ] **Step 4: Run the planner and workflow-contract suites**

Run:

```bash
python3 scripts/ci/test_reborn_pr_test_plan.py
python3 scripts/ci/test_ws12_workflow_contracts.py
```

Expected: both suites pass.
