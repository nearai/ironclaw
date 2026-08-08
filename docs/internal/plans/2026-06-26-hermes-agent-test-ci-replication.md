# Hermes-Agent Test & CI Investigation — What to Replicate in IronClaw

**Date:** 2026-06-26
**Source:** `github.com/NousResearch/hermes-agent` (local: `/Users/henry/Code/hermes-agent`)
**Goal:** Inventory how hermes-agent structures tests + CI, then identify concrete patterns worth replicating in the IronClaw repo/CI.

> hermes-agent is a Python project (`uv` + setuptools, ~850 test files, ~17k tests). IronClaw is Rust (+ JS frontend, Python/Playwright e2e). Stack differs, but the *test-infra philosophy* is highly transferable. IronClaw's CI is already more mature than hermes in most dimensions (feature-matrix, snapshot gates, coverage, hermetic keychain disable, mold linker, change-scoped selection). This doc focuses on the **delta** — what hermes does that IronClaw does *not*, and whether it's worth adopting.

---

## 1. What Hermes Does

### 1.1 Test suite

- **Framework:** pytest 9.0.2 + pytest-asyncio. Single marker discipline: `integration` (excluded by default via `addopts = "-m 'not integration'"`), plus a couple of opt-out markers.
- **Layout:** ~850 `test_*.py` files across 40+ dirs mirroring source modules (`tests/agent/`, `tests/gateway/`, `tests/tools/`, `tests/hermes_cli/`, …). `tests/integration/`, `tests/e2e/`, `tests/docker/`, `tests/stress/` are special-cased.
- **No VCR/cassettes.** LLM calls mocked with plain `unittest.mock` / `AsyncMock`. Retry backoff collapsed to zero via an autouse fixture.
- **Custom fakes:** in-process aiohttp fake Home Assistant server (`tests/fakes/fake_ha_server.py`); a stdlib JSON-RPC mock LSP server selectable by env var (`clean`/`errors`/`crash`/`slow`).
- **Property testing without Hypothesis:** `random`-driven 500-sequence fuzzing of a Kanban SQLite DB asserting 9 invariants after each op.

### 1.2 The three autouse hermetic fixtures (root `conftest.py`) — the standout

1. **`_hermetic_environment`** — blanks *every* credential-shaped env var (by suffix `_API_KEY`/`_TOKEN`/`_SECRET` + 40 explicit names + 50 `HERMES_*` behavioral vars), redirects `HERMES_HOME` to a tmp dir, pins `TZ=UTC LANG=C.UTF-8 PYTHONHASHSEED=0`, disables AWS IMDS lookups (saves ~2s/test stall).
2. **`_live_system_guard`** — intercepts 9 process/kill primitives (`os.kill`, `os.killpg`, all `subprocess.*`, `os.system`, `pty.spawn`, `asyncio.create_subprocess_*`) so a buggy test can't kill the developer's *live running gateway*. Motivated by a real incident: 5+ accidental gateway kills in 3 days. Allows signals to the test's own subtree via psutil; opt-out marker for PTY tests.
3. **`_ensure_current_event_loop`** — fresh loop per sync test on 3.11+.

### 1.3 Custom parallel test runner (`scripts/run_tests_parallel.py`) — the other standout

- **Per-file subprocess isolation instead of pytest-xdist.** One `python -m pytest <file>` per file, bounded by `ThreadPoolExecutor(cpu*2)`. Kills cross-file module-level state leakage (cached singletons, ContextVars, registries) that xdist's loadscope doesn't fully solve.
- **140s per-file timeout**, SIGKILLs the whole process group on timeout (captures pgid before leader exits).
- **`--slice I/N` with LPT (Longest-Processing-Time) bin-packing** using a `test_durations.json` cache for balanced CI shard makespan. Duration cache written **only on main** (no PR-to-PR cache poisoning). CI runs 6 slices.
- Live per-file progress, inline failure output with copy-paste repro command.
- `scripts/run_tests.sh` wraps it in `env -i` for belt-and-suspenders hermeticity.

### 1.4 CI architecture

- **Single-gate branch protection.** `ci.yml` orchestrator → `detect-changes` composite action → fans out to 9 `workflow_call` sub-workflows → one terminal `all-checks-pass` job aggregates results via inline Python (`if: always()`, skipped=success). Branch protection requires only that one check, so adding a workflow never requires editing the required-check list.
- **Change classifier** (`scripts/ci/classify_changes.py`): outputs 7 boolean lanes from the changed-file list (using *frozen* base/head SHAs from the event payload, not the mutable PR-files endpoint). **Fail-open:** empty diff or any `.github/` change runs everything.
- **Lint is deliberately near-empty:** `ruff` enforces exactly one rule (`PLW1514` — bare `open()` without `encoding=`, a Windows-corruption footgun). All other ruff/`ty` output is **advisory** — posted as a sticky PR comment diff vs merge-base, exit-zero. Plus a blocking custom `check-windows-footguns.py`.
- **`retry` composite action** wraps every flaky network install (`npm ci`, `uv sync`, `pip install`); command passed via env var (injection-safe), 3 attempts / 10s delay.
- **Supply-chain scanner** with ruthless scope discipline (only `.pth` files, same-line `base64`+`exec/eval`, obfuscated `subprocess`, root install-hooks) — header documents which patterns were *removed* because they trained reviewers to ignore the check.
- **`history-check.yml`** — fails PRs with no common ancestor with main (`git merge-base origin/main HEAD` empty), preventing orphan-branch grafts. Motivated by a real incident that collapsed `git blame` on ~1500 files.
- **`uv-lockfile-check`** runs `uv lock --check` against the *merged* state.
- Exact-pinned deps (`==X.Y.Z`) after a real supply-chain attack; dependabot enabled *only* for github-actions.
- Watchdog workflows: skills-index freshness probe every 4h, opens/appends a single GitHub issue rather than spamming.

---

## 2. What IronClaw Already Has (no action needed)

IronClaw matches or exceeds hermes here:

| Capability | Hermes | IronClaw |
|---|---|---|
| Single-gate branch protection | `all-checks-pass` | `code-style`, `run-tests`, `reborn-tests` roll-up gates |
| Change-scoped test selection | `classify_changes.py` | `scripts/ci/classify-test-scope.sh` + diff regex guards |
| Test sharding | LPT 6-slice (custom runner) | nextest `hash:N/M`, reborn 4-way partition, ~60 per-crate jobs, 7 e2e groups |
| Hermetic credential isolation | autouse env-blanking fixture | `IRONCLAW_DISABLE_OS_KEYCHAIN=1`, secrets to 0600 temp files |
| Snapshot/golden gates | none | `cargo insta --check`, rejects committed `.snap.new` |
| Coverage | **none** | `cargo-llvm-cov` + Codecov OIDC + e2e instrumented |
| Build cache discipline | uv cache, arm64 registry cache | `Swatinem/rust-cache` with `save-if` main-only |
| Network-flake mitigation | `retry` action | `.cargo/config.toml` retries + mold linker + `timeout --kill-after` |
| Lint footgun checks | `check-windows-footguns.py` | `pre-commit-safety.sh` (10 checks), `check_no_panics.py`, boundary checks |
| Orphan-history guard | `history-check.yml` | — (gap, see below) |

---

## 3. What's Worth Replicating (the delta)

Ranked by value/effort.

### Tier 1 — High value, low effort

**A. Live-system guard fixture (Rust equivalent).**
Hermes's biggest pragmatic win: a test can't kill your live dev process. IronClaw runs a real gateway/daemon (`service.rs`, launchd/systemd) during local dev. A buggy test that calls `pkill`, `systemctl`, or shells out to `ironclaw service` could nuke it.
- *Replicate as:* a shared test guard (in a `tests/common/` helper or a `#[cfg(test)]` harness module) that refuses `Command::new("pkill"/"systemctl"/"kill")` and `service::*` stop/restart paths unless an explicit opt-in marker/env is set. Lower urgency than in hermes (Rust tests don't `os.kill` arbitrary PIDs as casually), but the `service.rs` install/uninstall tests are the real risk surface. Audit those first.
- *Cheaper interim:* extend `scripts/pre-commit-safety.sh` to flag new test lines that shell out to `pkill`/`systemctl`/`launchctl` without a `// test-system-safe:` annotation.

**B. Orphan-history guard.**
IronClaw has no equivalent of `history-check.yml`. The hermes incident (orphan branch grafted a 2nd root, collapsed blame on 1500 files) is stack-agnostic and IronClaw is just as exposed.
- *Replicate as:* a tiny PR-only job — `git merge-base origin/main HEAD` must be non-empty, else fail. ~15 lines of YAML. Pure upside.

**C. Advisory-lint sticky PR comment (clippy/`ty` analog).**
Hermes enforces a *minimal* blocking lint and surfaces everything else as a non-blocking diff comment vs merge-base. IronClaw enforces `-D warnings` (good), but has no surfaced *advisory* layer for pedantic/nursery clippy lints that the team doesn't want blocking.
- *Replicate as:* optional `clippy::pedantic`/`nursery` run → diff vs merge-base → sticky PR comment (marker-based update, exit-zero). IronClaw already has `scripts/ci/delta_lint.sh`; wire its output into a sticky comment instead of pre-push only. Medium value — only if the team wants pedantic signal without the block.

### Tier 2 — Medium value

**D. Per-file/per-crate timeout with process-group kill.**
IronClaw wraps tests in `timeout --kill-after=30s`, but at the *job* level. Hermes times out per *file* (140s) and kills the process group, so one hung test doesn't burn the whole job's budget and the failure points at the exact file. IronClaw's per-crate reborn jobs already approximate this. Worth tightening for the legacy `cargo test` matrix only if hangs are observed. nextest's `slow-timeout = { terminate-after = 3 }` (already in `.config/nextest.toml`) covers most of this — **largely already done.**

**E. Duration-cached LPT sharding.**
Hermes balances 6 shards by historical file runtime. IronClaw shards by hash partition (even by count, not by time). If reborn/e2e shard makespan is imbalanced (one shard consistently the long pole), adopt nextest's built-in `--partition` is hash-based; a time-balanced variant would need a custom runner like hermes's. **Only worth it if shard skew is measurably hurting wall-clock.** Measure first.

**F. Hermetic time/locale/hash pinning in the test harness.**
Hermes pins `TZ=UTC LANG=C.UTF-8 PYTHONHASHSEED=0` for every test. IronClaw disables keychain but doesn't uniformly pin TZ/locale. Given IronClaw's runtime-context time-slice work (per memory: fingerprint = model-visible rendering, tz handling), timezone-dependent test flakiness is a real risk.
- *Replicate as:* set `TZ=UTC LANG=C.UTF-8` in CI test job `env:` and document a local-dev expectation. Low effort, prevents a class of flakes.

### Tier 3 — Adopt the philosophy, not the mechanism

**G. Lint scope discipline / "don't train reviewers to ignore checks."**
Hermes's supply-chain scanner header documenting *removed* patterns is a cultural artifact worth importing: every blocking check should have a near-zero false-positive rate or it gets ignored. Audit `pre-commit-safety.sh`'s 10 checks for false-positive rate; demote noisy ones to advisory.

**H. Fail-open change classification.**
IronClaw's `classify-test-scope.sh` should (verify it does) treat `.github/` changes and empty/ambiguous diffs as "run everything." Confirm the fail-open contract matches hermes's — a fail-*closed* classifier silently skips tests on edge cases.

**I. Watchdog issue de-duplication.**
IronClaw's nightly/canary workflows open a GitHub issue on failure. Confirm they *append to an existing open issue* rather than spamming new ones each run (hermes's skills-index-freshness pattern). Check `nightly-deep-ci.yml` / `nightly-e2e.yml` issue-creation logic.

---

## 4. Explicitly NOT Worth Replicating

- **Per-file subprocess runner replacing xdist.** This solves a *Python* problem (module-level singleton/ContextVar leakage across xdist workers). Rust integration tests are already separate binaries (per-`tests/*.rs`-file process isolation is the cargo default). IronClaw gets this for free. The custom 860-line runner would be pure complexity.
- **Property fuzzing via `random`.** If IronClaw wants property tests, use `proptest`/`quickcheck` (real shrinking) — don't hand-roll hermes's `random`-loop.
- **uv lockfile merged-state check.** IronClaw uses `Cargo.lock` + `--locked`; `cargo-deny` + release-plz cover the equivalent ground.
- **PyPI OIDC / Sigstore publishing.** N/A — IronClaw ships Docker images, already via `docker.yml`.

---

## 5. Recommended Next Actions

1. **Ship Tier-1B (orphan-history guard)** — ~15 lines, pure upside, do it now.
2. **Audit `service.rs` tests + add Tier-1A guard** (pre-commit annotation flag first, full fixture if the audit finds real risk).
3. **Pin `TZ=UTC LANG=C.UTF-8` in CI test jobs (Tier-2F)** — one-line env change, prevents tz flakes given the runtime-context time-slice work.
4. **Decide on advisory clippy comment (Tier-1C)** — needs a team preference call (block vs surface pedantic lints). Wire `delta_lint.sh` → sticky comment if yes.
5. **Verify fail-open classification + watchdog dedup (Tier-3H/I)** — read-only audits, confirm existing behavior matches the hermes contract; fix if not.
6. **Measure shard skew before touching sharding (Tier-2E)** — don't build a custom LPT runner on a hunch.

Items 1–3 are concrete and independently shippable. 4 needs a decision. 5 is verification. 6 is gated on data.
