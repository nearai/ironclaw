# Reborn Harness Engineering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Reborn agent-operable by adding repo-local maps, mechanical contract guardrails, deterministic local feedback loops, compatibility replay, and agent-readable observability inspired by OpenAI's Harness Engineering article.

**Architecture:** This plan keeps #2987 as the Reborn substrate/cutover tracker, #3020 as the blocking compatibility gate, and #3031 as the product-surface migration tracker. It does not duplicate substrate work. It adds the harness around Reborn so agents can discover contracts, validate boundary invariants, boot isolated environments, inspect runtime evidence, and execute product-surface migration safely.

**Tech Stack:** Rust workspace, Python/shell CI guardrail scripts, GitHub Actions, existing replay/insta/nextest fixtures, Playwright E2E, Reborn contract docs under `docs/reborn/contracts/`.

**Source article:** <https://openai.com/index/harness-engineering/>

**Companion gist:** <https://gist.github.com/serrrfirat/ab14c227f4dceab7e375d509d3eb5bdd>

---

## Non-goals

- Do not replace #2987's substrate/cutover plan.
- Do not replace #3020's compatibility gate.
- Do not replace #3031's product-surface migration tracker.
- Do not rebuild v1 as a parallel architecture.
- Do not introduce a second privileged path around `CapabilityHost`, Reborn kernel policy, approvals, leases, resources, secrets, network policy, or durable events.
- Do not make `FEATURE_PARITY.md` the source of truth for Reborn product-surface migration. Use actual v1 surfaces and the compatibility/product-surface manifest introduced below.

## Current anchors

- Parent Reborn substrate tracker: <https://github.com/nearai/ironclaw/issues/2987>
- Blocking compatibility gate: <https://github.com/nearai/ironclaw/issues/3020>
- Product-surface migration tracker: <https://github.com/nearai/ironclaw/issues/3031>
- Contract freeze index: `docs/reborn/contracts/_contract-freeze-index.md`
- Current architecture map: `docs/reborn/2026-04-25-current-architecture-map.md`

## Harness principles to encode

1. Humans steer through issues, contracts, plans, compatibility manifests, and acceptance criteria.
2. Agents execute through isolated worktrees, deterministic fake services, replay traces, E2E artifacts, and mechanical checks.
3. `AGENTS.md` remains a quick-start map. Reborn details live in linked, versioned `docs/reborn/` sources of truth.
4. Architectural boundaries are enforced mechanically with remediation text, not tribal review memory.
5. All authority, persistence, events, network, secrets, filesystem, memory, approvals, or runtime execution changes require caller-level tests.
6. User-facing compatibility evidence is captured through #3020 and #3031 artifacts before final cutover.

## File structure to add or modify

### Documentation and planning

- Create `docs/reborn/README.md`
  - Agent-facing table of contents for Reborn.
  - Links to contracts, current architecture map, active plans, quality grades, compatibility manifest, and local harness docs.
- Create `docs/reborn/harness/local-dev.md`
  - Documents per-worktree Reborn boot commands and artifacts.
- Create `docs/reborn/harness/replay.md`
  - Documents Reborn replay fixture ownership and update flow.
- Create `docs/reborn/harness/observability.md`
  - Documents logs/events/audit/process/debug bundle conventions.
- Create `docs/reborn/plans/active/2026-04-28-reborn-harness-engineering.md`
  - This plan.
- Create `docs/reborn/plans/completed/.gitkeep`
  - Holds completed plan archive directory in git.
- Create `docs/reborn/plans/debt/.gitkeep`
  - Holds debt plan directory in git.
- Create `docs/reborn/quality/domain-grades.md`
  - Domain/layer quality dashboard for Reborn harness readiness.
- Create `docs/reborn/quality/known-drift.md`
  - Drift ledger for repeated agent-generated inconsistencies.
- Create `docs/reborn/compatibility/product-surface.yml`
  - Machine-readable map from v1 product surfaces to Reborn contracts/tests/fixtures.
- Create `docs/reborn/reviews/security-review.md`
  - Specialized security/authority reviewer checklist.
- Create `docs/reborn/reviews/compatibility-review.md`
  - Specialized #3020/#3031 compatibility reviewer checklist.
- Create `docs/reborn/reviews/boundary-review.md`
  - Specialized architecture-boundary reviewer checklist.
- Create `docs/reborn/reviews/persistence-review.md`
  - PostgreSQL/libSQL/storage reviewer checklist.
- Create `docs/reborn/reviews/product-surface-review.md`
  - v1 surface parity reviewer checklist.

### Guardrail scripts

- Create `scripts/check-reborn-docs.py`
  - Validates Reborn docs/plans/contract metadata and links.
- Create `scripts/check-reborn-boundaries.py`
  - Enforces Reborn dependency and call-path invariants.
- Create `scripts/check-reborn-compatibility-manifest.py`
  - Validates `docs/reborn/compatibility/product-surface.yml`.
- Create `scripts/check-reborn-plans.py`
  - Validates active/completed/debt plan schema and required references.
- Create `scripts/check-reborn-golden-rules.py`
  - Encodes high-level Reborn golden-principle checks that do not belong in narrower scripts.
- Create `scripts/reborn-event-coverage.sh`
  - Reports Reborn durable event/projection coverage in snapshots.
- Create `scripts/reborn-dev`
  - Per-worktree Reborn local environment runner.
- Create `scripts/reborn-ui-snap`
  - Captures Reborn UI screenshots/DOM/console/network artifacts for named scenarios.
- Create `scripts/reborn-ui-video`
  - Captures before/after Reborn UI videos for named scenarios.

### Tests and fixtures

- Create `tests/fixtures/reborn_traces/README.md`
  - Reborn replay trace format and ownership notes.
- Create `tests/reborn_conformance/README.md`
  - Conformance suite contract and backend matrix.
- Create `tests/e2e/scenarios/reborn/README.md`
  - E2E scenario ownership and artifact expectations.
- Create `crates/ironclaw_reborn_testkit/`
  - Deterministic fake LLM, network, MCP, OAuth, runtime, event, process, and fixture helpers.
- Modify `Cargo.toml`
  - Add `crates/ironclaw_reborn_testkit` as a workspace member when the testkit crate is created.
- Modify `.github/workflows/code_style.yml`
  - Add Reborn guardrail scripts to the style/architecture roll-up after scripts are implemented.
- Create `.github/workflows/reborn-replay-gate.yml`
  - Runs Reborn replay snapshots and Reborn event coverage after fixtures exist.

---

## Task 1: Add the Reborn documentation map

**Files:**
- Create: `docs/reborn/README.md`
- Create: `docs/reborn/harness/local-dev.md`
- Create: `docs/reborn/harness/replay.md`
- Create: `docs/reborn/harness/observability.md`

- [ ] **Step 1: Create `docs/reborn/README.md` with progressive-disclosure navigation**

Use this exact initial content:

```markdown
# Reborn Architecture Map

Reborn is IronClaw's host/runtime redesign. This page is the agent-facing map; it is not the full spec.

## Start here

1. Read the contract freeze index: `docs/reborn/contracts/_contract-freeze-index.md`.
2. Read the current architecture map: `docs/reborn/2026-04-25-current-architecture-map.md`.
3. For product-surface migration, read `docs/reborn/compatibility/product-surface.yml` and issue #3031.
4. For compatibility gate work, read issue #3020 and the Reborn replay fixtures.
5. For substrate/cutover work, read issue #2987 and the relevant contract docs.

## Contracts

Contracts live under `docs/reborn/contracts/`. A contract answers:

- which crate/service owns the domain;
- which crates must not depend on it;
- where durable state lives;
- which scope fields must flow through the call;
- which side effects happen and in what order;
- which failures are fail-closed vs best-effort;
- which errors/events must be redacted;
- which tests prove the contract.

## Harness docs

- Local worktree environment: `docs/reborn/harness/local-dev.md`
- Replay and compatibility fixtures: `docs/reborn/harness/replay.md`
- Observability and debug bundles: `docs/reborn/harness/observability.md`

## Plans and quality

- Active plans: `docs/reborn/plans/active/`
- Completed plans: `docs/reborn/plans/completed/`
- Debt plans: `docs/reborn/plans/debt/`
- Quality dashboard: `docs/reborn/quality/domain-grades.md`
- Drift ledger: `docs/reborn/quality/known-drift.md`

## Review checklists

- Security/authority: `docs/reborn/reviews/security-review.md`
- Compatibility: `docs/reborn/reviews/compatibility-review.md`
- Boundaries: `docs/reborn/reviews/boundary-review.md`
- Persistence: `docs/reborn/reviews/persistence-review.md`
- Product surface: `docs/reborn/reviews/product-surface-review.md`
```

- [ ] **Step 2: Create `docs/reborn/harness/local-dev.md`**

Document the target command surface:

````markdown
# Reborn Local Harness

The Reborn local harness must be bootable per git worktree so agents can validate changes without shared mutable state.

Target command surface:

```bash
scripts/reborn-dev up
scripts/reborn-dev down
scripts/reborn-dev reset
scripts/reborn-dev status
scripts/reborn-dev logs
scripts/reborn-dev seed
scripts/reborn-dev doctor
```

Per-worktree state lives under `.pi/reborn-dev/`:

```text
.pi/reborn-dev/
  db/
  logs/
  events/
  traces/
  artifacts/
  screenshots/
  config.toml
  tokens.json
```

`doctor` must emit a redacted bundle with config, logs, events, audit records, process tree, failed invocations, screenshots, and a replay command.
````

- [ ] **Step 3: Create `docs/reborn/harness/replay.md`**

Document fixture ownership:

```markdown
# Reborn Replay Harness

Reborn replay fixtures prove contract and product-surface behavior without live providers.

Required fixture families:

- chat/tool turn;
- approval required -> approve -> resume;
- approval denied;
- auth blocked;
- MCP auth flow;
- WASM capability call;
- script capability call;
- memory read/write/search;
- secret lease usage;
- network policy denial;
- background process spawn/status/result;
- SSE replay cursor;
- WebSocket reconnect;
- routine/job trigger;
- extension install/activate/remove;
- v1 product-surface compatibility cases from #3031;
- #3020 blocking compatibility cases.

Replay snapshots are reviewable compatibility artifacts. JSON fixtures drive the model/runtime; `.snap` files encode what Reborn did with them.
```

- [ ] **Step 4: Create `docs/reborn/harness/observability.md`**

Document common fields:

```markdown
# Reborn Observability Harness

Every Reborn log/event/span should include stable IDs where applicable:

- `tenant_id`
- `user_id`
- `project_id`
- `agent_id`
- `thread_id`
- `turn_id`
- `run_id`
- `invocation_id`
- `process_id`
- `extension_id`
- `capability_id`
- `runtime_kind`
- `approval_id`
- `lease_id`

The goal is agent-readable evidence. An agent debugging a failure should inspect structured logs, durable events, audit records, process state, and UI artifacts before proposing a fix.
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
git diff --check
git status --short
```

Expected: no whitespace errors and only the four new docs plus this plan if Task 1 is committed together.

Commit:

```bash
git add docs/reborn/README.md docs/reborn/harness/local-dev.md docs/reborn/harness/replay.md docs/reborn/harness/observability.md docs/reborn/plans/active/2026-04-28-reborn-harness-engineering.md
git commit -m "docs(reborn): add harness engineering plan and map"
```

---

## Task 2: Add plan, quality, compatibility, and review scaffolding

**Files:**
- Create: `docs/reborn/plans/completed/.gitkeep`
- Create: `docs/reborn/plans/debt/.gitkeep`
- Create: `docs/reborn/quality/domain-grades.md`
- Create: `docs/reborn/quality/known-drift.md`
- Create: `docs/reborn/compatibility/product-surface.yml`
- Create: `docs/reborn/reviews/security-review.md`
- Create: `docs/reborn/reviews/compatibility-review.md`
- Create: `docs/reborn/reviews/boundary-review.md`
- Create: `docs/reborn/reviews/persistence-review.md`
- Create: `docs/reborn/reviews/product-surface-review.md`

- [ ] **Step 1: Create empty plan archive directories**

Run:

```bash
mkdir -p docs/reborn/plans/completed docs/reborn/plans/debt
touch docs/reborn/plans/completed/.gitkeep docs/reborn/plans/debt/.gitkeep
```

- [ ] **Step 2: Create `docs/reborn/quality/domain-grades.md`**

Use this starting table:

```markdown
# Reborn Quality Grades

Grades track agent-operability and product-readiness, not developer effort.

| Domain | Grade | Main gaps | Required gate |
| --- | --- | --- | --- |
| CapabilityHost | B+ | Product projections and migration callers incomplete | conformance + replay |
| Events/projections | C | Durable cursors/fanout/SSE/WS bridge incomplete | strict replay/event coverage |
| Memory/workspace | B | Production provider wiring and multi-scope product parity incomplete | PG/libSQL + replay |
| Secrets | B- | Production master-key/keychain wiring and product lease flows incomplete | redaction + lease tests |
| Web gateway | C+ | Durable event projection bridge and Reborn route parity incomplete | E2E + reconnect replay |
| Extensions lifecycle | C | Full state machine/product UI migration incomplete | lifecycle conformance |
| MCP lane | C | Auth/lifecycle productization incomplete | auth replay fixtures |
| Routines/jobs | C | Reborn ownership and scheduler/process integration incomplete | scheduler/process tests |
```

- [ ] **Step 3: Create `docs/reborn/quality/known-drift.md`**

Use this starting content:

```markdown
# Reborn Known Drift Ledger

Use this file for repeated agent-generated drift that should become a rule, lint, fixture, or refactor issue.

| Date | Drift pattern | Evidence | Planned correction | Owner issue |
| --- | --- | --- | --- | --- |
| 2026-04-28 | Reborn implementation prompts can omit caller-level tests for side-effect gates | Contract freeze checklist requires caller-level tests | Add plan/doc checker and boundary review checklist | #3031 |
```

- [ ] **Step 4: Create `docs/reborn/compatibility/product-surface.yml`**

Use this initial manifest:

```yaml
version: 1
source_issues:
  substrate_tracker: 2987
  compatibility_gate: 3020
  product_surface_migration: 3031
surfaces:
  web_chat:
    owner: crates/ironclaw_gateway
    v1_paths:
      - src/channels/web
      - tests/e2e/scenarios/test_chat.py
      - tests/e2e/scenarios/test_sse_reconnect.py
    reborn_contracts:
      - docs/reborn/contracts/turns-agent-loop.md
      - docs/reborn/contracts/events-projections.md
      - docs/reborn/contracts/migration-compatibility.md
    required_evidence:
      replay_fixture: tests/fixtures/reborn_traces/web_chat_basic.json
      e2e: tests/e2e/scenarios/reborn/test_reborn_chat.py
    status: planned
  approvals_and_tool_execution:
    owner: Reborn CapabilityHost / approvals / tool surface adapters
    v1_paths:
      - src/agent
      - src/tools
      - tests/e2e/scenarios/test_tool_approval.py
      - tests/e2e/scenarios/test_tool_execution.py
    reborn_contracts:
      - docs/reborn/contracts/capability-access.md
      - docs/reborn/contracts/approvals.md
      - docs/reborn/contracts/run-state.md
      - docs/reborn/contracts/turns-agent-loop.md
    required_evidence:
      replay_fixture: tests/fixtures/reborn_traces/approval_resume.json
      e2e: tests/e2e/scenarios/reborn/test_reborn_approval_resume.py
    status: planned
  memory_workspace:
    owner: ironclaw_memory
    v1_paths:
      - src/workspace
      - src/tools
      - tests/fixtures/llm_traces
    reborn_contracts:
      - docs/reborn/contracts/memory.md
      - docs/reborn/contracts/storage-placement.md
      - docs/reborn/contracts/migration-compatibility.md
    required_evidence:
      replay_fixture: tests/fixtures/reborn_traces/memory_read_write_search.json
      conformance: tests/reborn_conformance/memory.rs
    status: planned
```

- [ ] **Step 5: Create review checklists**

Each review file should contain a short checklist. Use these headings:

```markdown
# Reborn <Area> Review Checklist

- [ ] Does the change cite the relevant Reborn contract docs?
- [ ] Does the change preserve #2987/#3020/#3031 ownership boundaries where applicable?
- [ ] Does the change include caller-level tests for side-effect gates?
- [ ] Does the change preserve tenant/user/project/agent scope propagation where applicable?
- [ ] Does the change avoid raw secret, host path, approval reason, lease, and backend error leaks?
- [ ] Does the change update replay/E2E/conformance evidence where product behavior changed?
```

Use a specific title for each file:

- `# Reborn Security Review Checklist`
- `# Reborn Compatibility Review Checklist`
- `# Reborn Boundary Review Checklist`
- `# Reborn Persistence Review Checklist`
- `# Reborn Product Surface Review Checklist`

- [ ] **Step 6: Verify and commit**

Run:

```bash
git diff --check
git status --short
```

Commit:

```bash
git add docs/reborn/plans/completed/.gitkeep docs/reborn/plans/debt/.gitkeep docs/reborn/quality/domain-grades.md docs/reborn/quality/known-drift.md docs/reborn/compatibility/product-surface.yml docs/reborn/reviews/security-review.md docs/reborn/reviews/compatibility-review.md docs/reborn/reviews/boundary-review.md docs/reborn/reviews/persistence-review.md docs/reborn/reviews/product-surface-review.md
git commit -m "docs(reborn): scaffold harness quality and compatibility maps"
```

---

## Task 3: Add Reborn document and plan ratchets

**Files:**
- Create: `scripts/check-reborn-docs.py`
- Create: `scripts/check-reborn-plans.py`
- Modify: `.github/workflows/code_style.yml`

- [ ] **Step 1: Add `scripts/check-reborn-docs.py`**

Implement a Python checker that verifies:

- `docs/reborn/README.md` exists;
- every markdown file under `docs/reborn/contracts/` contains at least one ownership cue such as `Owner`, `Source of truth`, `Fail-closed`, `Best-effort`, `Redaction`, or `Tests`;
- every link from `docs/reborn/README.md` resolves to an existing repo path when it is a local markdown/path link;
- `docs/reborn/compatibility/product-surface.yml` exists.

The script should print remediation text that points to `docs/reborn/contracts/_contract-freeze-index.md`.

- [ ] **Step 2: Add `scripts/check-reborn-plans.py`**

Implement a Python checker that verifies:

- active plans live under `docs/reborn/plans/active/`;
- active plan filenames match `YYYY-MM-DD-*.md`;
- active plans include `Goal`, `Non-goals`, `Current anchors`, `File structure`, `Acceptance criteria`, and `Verification commands` headings;
- completed plans include a `Verification evidence` heading;
- debt plans include an `Owner issue` heading.

The checker should fail with clear remediation instructions.

- [ ] **Step 3: Add CI wiring**

Modify `.github/workflows/code_style.yml` by adding a job after `gateway-boundaries`:

```yaml
  reborn-docs:
    name: Reborn docs and plan ratchets
    needs: changes
    if: needs.changes.outputs.has_code == 'true'
    runs-on: ubuntu-latest
    steps:
    - name: Checkout repository
      uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd # v6
      with:
        persist-credentials: false
    - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065 # v5
      with:
        python-version: "3.12"
    - name: Check Reborn docs
      run: python3 scripts/check-reborn-docs.py
    - name: Check Reborn plans
      run: python3 scripts/check-reborn-plans.py
```

Then add `reborn-docs` to the roll-up `needs` list and always-required job list.

- [ ] **Step 4: Add tests for the checker behavior**

Each checker should support a `test` subcommand using `unittest`, mirroring the style of `scripts/check_gateway_boundaries.py test`.

Run:

```bash
python3 scripts/check-reborn-docs.py test
python3 scripts/check-reborn-plans.py test
python3 scripts/check-reborn-docs.py
python3 scripts/check-reborn-plans.py
```

- [ ] **Step 5: Commit**

```bash
git add scripts/check-reborn-docs.py scripts/check-reborn-plans.py .github/workflows/code_style.yml
git commit -m "ci(reborn): add docs and plan ratchets"
```

---

## Task 4: Add Reborn boundary and golden-rule guardrails

**Files:**
- Create: `scripts/check-reborn-boundaries.py`
- Create: `scripts/check-reborn-golden-rules.py`
- Modify: `.github/workflows/code_style.yml`

- [ ] **Step 1: Add `scripts/check-reborn-boundaries.py`**

The first version should check for these violations with explicit allowlists:

1. Product/web/channel code calling `RuntimeDispatcher` directly once Reborn dispatcher types exist in the main workspace.
2. Dispatcher code importing web, channels, tools, approvals UI, or product workflow modules.
3. Filesystem crate code importing memory-domain path grammar.
4. Memory/provider code using raw `reqwest` outside an approved `ironclaw_network` boundary.
5. Secret values formatted into logs/events/errors.

The script must print a contract link for each violation:

- `docs/reborn/contracts/dispatcher.md`
- `docs/reborn/contracts/capability-access.md`
- `docs/reborn/contracts/filesystem.md`
- `docs/reborn/contracts/memory.md`
- `docs/reborn/contracts/network.md`
- `docs/reborn/contracts/secrets.md`

- [ ] **Step 2: Add `scripts/check-reborn-golden-rules.py`**

Encode these first mechanical rules:

- Reborn plans must not say `FEATURE_PARITY.md` is the product-surface source of truth.
- Reborn compatibility work must cite #3020.
- Reborn product-surface migration work must cite #3031.
- Any Reborn plan touching secrets/network/approvals/events/processes must include `Redaction/no-leak` or `redaction` text.
- Any Reborn plan touching PostgreSQL/libSQL persistence must include `PostgreSQL/libSQL parity` text.

- [ ] **Step 3: Add self-tests**

Both scripts should support `test` subcommands with positive and negative fixture strings. Tests must run without modifying the repo.

Run:

```bash
python3 scripts/check-reborn-boundaries.py test
python3 scripts/check-reborn-golden-rules.py test
python3 scripts/check-reborn-boundaries.py
python3 scripts/check-reborn-golden-rules.py
```

- [ ] **Step 4: Wire CI**

Add a `reborn-boundaries` job to `.github/workflows/code_style.yml` and the roll-up. Keep the first version scoped and allowlist-backed so it catches new violations without blocking known migration work.

- [ ] **Step 5: Commit**

```bash
git add scripts/check-reborn-boundaries.py scripts/check-reborn-golden-rules.py .github/workflows/code_style.yml
git commit -m "ci(reborn): add boundary and golden-rule guardrails"
```

---

## Task 5: Add Reborn compatibility manifest validation

**Files:**
- Create: `scripts/check-reborn-compatibility-manifest.py`
- Modify: `.github/workflows/code_style.yml`
- Modify: `docs/reborn/compatibility/product-surface.yml`

- [ ] **Step 1: Implement manifest checker**

`check-reborn-compatibility-manifest.py` should parse YAML or use Python stdlib plus a simple conservative parser if PyYAML is unavailable. It must verify every surface has:

- `owner`;
- `v1_paths`;
- `reborn_contracts`;
- `required_evidence`;
- `status` in `planned`, `partial`, `covered`, or `blocked`;
- existing contract paths;
- source issues `2987`, `3020`, and `3031`.

- [ ] **Step 2: Add self-tests**

Use `unittest` fixtures for:

- valid minimal manifest;
- missing source issue;
- missing owner;
- missing contract path;
- invalid status.

Run:

```bash
python3 scripts/check-reborn-compatibility-manifest.py test
python3 scripts/check-reborn-compatibility-manifest.py
```

- [ ] **Step 3: Wire CI**

Add the manifest checker to the `reborn-docs` job from Task 3 or a separate `reborn-compatibility` job.

- [ ] **Step 4: Commit**

```bash
git add scripts/check-reborn-compatibility-manifest.py docs/reborn/compatibility/product-surface.yml .github/workflows/code_style.yml
git commit -m "ci(reborn): validate product surface compatibility manifest"
```

---

## Task 6: Add Reborn replay fixture scaffolding and event coverage

**Files:**
- Create: `tests/fixtures/reborn_traces/README.md`
- Create: `scripts/reborn-event-coverage.sh`
- Create: `.github/workflows/reborn-replay-gate.yml`
- Create or modify: `tests/snapshots/` as fixtures are added in follow-up work

- [ ] **Step 1: Create `tests/fixtures/reborn_traces/README.md`**

Document the fixture families listed in `docs/reborn/harness/replay.md`. Include ownership rules:

- JSON fixture describes input/model/runtime script.
- Snapshot describes observed Reborn behavior.
- Fixture changes without snapshot changes are suspicious.
- Snapshot changes without contract/product-surface explanation require review.

- [ ] **Step 2: Add `scripts/reborn-event-coverage.sh`**

Model it after `scripts/trace-coverage.sh`. First version should:

- find Reborn event enum/type definitions once present;
- scan `tests/snapshots/*.snap` for event names;
- run soft by default;
- support `--strict`.

If Reborn event definitions are not yet present in the current workspace, the script should print a clear skip message and exit 0.

- [ ] **Step 3: Add replay workflow**

Create `.github/workflows/reborn-replay-gate.yml` with path filters for:

```yaml
- 'docs/reborn/**'
- 'tests/fixtures/reborn_traces/**'
- 'tests/snapshots/**'
- 'scripts/reborn-event-coverage.sh'
- '.github/workflows/reborn-replay-gate.yml'
```

Start with docs/coverage validation only. Add `cargo insta test` targets after the first Reborn replay tests are implemented.

- [ ] **Step 4: Commit**

```bash
git add tests/fixtures/reborn_traces/README.md scripts/reborn-event-coverage.sh .github/workflows/reborn-replay-gate.yml
git commit -m "test(reborn): scaffold replay compatibility gate"
```

---

## Task 7: Add deterministic Reborn testkit crate

**Files:**
- Create: `crates/ironclaw_reborn_testkit/Cargo.toml`
- Create: `crates/ironclaw_reborn_testkit/src/lib.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_events.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_llm.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_mcp.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_network.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_oauth.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_process.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fake_runtime.rs`
- Create: `crates/ironclaw_reborn_testkit/src/fixtures.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Create crate skeleton**

Use package name `ironclaw_reborn_testkit`, edition `2024`, publish false, and dependencies on `serde`, `serde_json`, `tokio`, `uuid`, and `chrono` as needed from workspace/root conventions.

- [ ] **Step 2: Add minimal deterministic fakes**

Each fake module should expose a small struct that records requests and returns configured deterministic responses. Do not connect to external services.

Required first structs:

- `FakeEventSink`
- `FakeLlmProvider`
- `FakeMcpServer`
- `FakeNetworkClient`
- `FakeOAuthProvider`
- `FakeProcessExecutor`
- `FakeRuntimeAdapter`

- [ ] **Step 3: Add unit tests**

Each fake should have a unit test proving request recording and deterministic response behavior.

Run:

```bash
cargo test -p ironclaw_reborn_testkit
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/ironclaw_reborn_testkit
git commit -m "test(reborn): add deterministic harness testkit"
```

---

## Task 8: Add per-worktree local harness scripts

**Files:**
- Create: `scripts/reborn-dev`
- Create: `scripts/reborn-ui-snap`
- Create: `scripts/reborn-ui-video`
- Modify: `docs/reborn/harness/local-dev.md`
- Modify: `docs/reborn/harness/observability.md`

- [ ] **Step 1: Implement `scripts/reborn-dev` command parser**

Support commands:

```bash
scripts/reborn-dev up
scripts/reborn-dev down
scripts/reborn-dev reset
scripts/reborn-dev status
scripts/reborn-dev logs
scripts/reborn-dev seed
scripts/reborn-dev doctor
```

First version may use JSONL files and deterministic fixture output before full Reborn services are wired.

- [ ] **Step 2: Implement `.pi/reborn-dev/` state layout**

`up` and `seed` must create:

```text
.pi/reborn-dev/db/
.pi/reborn-dev/logs/
.pi/reborn-dev/events/
.pi/reborn-dev/traces/
.pi/reborn-dev/artifacts/
.pi/reborn-dev/screenshots/
.pi/reborn-dev/config.toml
.pi/reborn-dev/tokens.json
```

`tokens.json` must contain fake or redacted local-only tokens only.

- [ ] **Step 3: Implement `doctor --bundle`**

`doctor` should write:

```text
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/config-redacted.json
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/logs.jsonl
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/events.jsonl
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/audit.jsonl
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/process-tree.json
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/failed-invocations.json
.pi/reborn-dev/artifacts/reborn-debug/<timestamp>/replay-command.txt
```

- [ ] **Step 4: Implement UI artifact script stubs**

`scripts/reborn-ui-snap` and `scripts/reborn-ui-video` should validate scenario names and create artifact directories. Add real Playwright/CDP integration in a follow-up once Reborn routes are wired.

- [ ] **Step 5: Verify scripts**

Run:

```bash
bash scripts/reborn-dev reset
bash scripts/reborn-dev up
bash scripts/reborn-dev status
bash scripts/reborn-dev doctor
bash scripts/reborn-ui-snap chat-basic
bash scripts/reborn-ui-video approval-resume
```

Expected: commands complete, create only `.pi/reborn-dev/` artifacts, and do not require live external services.

- [ ] **Step 6: Commit**

```bash
git add scripts/reborn-dev scripts/reborn-ui-snap scripts/reborn-ui-video docs/reborn/harness/local-dev.md docs/reborn/harness/observability.md
git commit -m "tooling(reborn): add local harness script scaffolding"
```

---

## Acceptance criteria

- `docs/reborn/README.md` gives agents a short map into Reborn contracts, plans, harness docs, quality grades, compatibility manifest, and reviews.
- Reborn docs/plans/compatibility manifests have mechanical checkers with self-tests.
- Reborn boundary/golden-rule checks exist and produce remediation text with contract links.
- `docs/reborn/compatibility/product-surface.yml` names #2987, #3020, and #3031 and maps initial product surfaces to contracts/evidence.
- Reborn replay scaffolding exists and is ready for #3020/#3031 fixtures.
- A deterministic `ironclaw_reborn_testkit` crate exists for fake provider/runtime/service tests.
- Reborn local harness scripts create per-worktree state and redacted debug bundles without external services.
- CI runs the Reborn docs/plan/boundary/compatibility guardrails once implemented.

## Verification commands

Run these after the full plan is implemented:

```bash
python3 scripts/check-reborn-docs.py test
python3 scripts/check-reborn-plans.py test
python3 scripts/check-reborn-boundaries.py test
python3 scripts/check-reborn-golden-rules.py test
python3 scripts/check-reborn-compatibility-manifest.py test
python3 scripts/check-reborn-docs.py
python3 scripts/check-reborn-plans.py
python3 scripts/check-reborn-boundaries.py
python3 scripts/check-reborn-golden-rules.py
python3 scripts/check-reborn-compatibility-manifest.py
bash scripts/reborn-event-coverage.sh
bash scripts/reborn-dev reset
bash scripts/reborn-dev up
bash scripts/reborn-dev status
bash scripts/reborn-dev doctor
cargo test -p ironclaw_reborn_testkit
cargo fmt --all -- --check
cargo clippy --all --tests --examples --all-features -- -D warnings
```

For this plan-only branch, run:

```bash
git diff --check
git status --short
```

## Rollback notes

- Documentation-only tasks can be reverted by deleting the added `docs/reborn/` harness/plans/quality/compatibility/reviews files.
- CI guardrail tasks can be rolled back by removing their workflow jobs and scripts; keep docs unless they became misleading.
- `scripts/reborn-dev` creates `.pi/reborn-dev/` local artifacts only; `.pi/` is already local state and must not be committed.
- `ironclaw_reborn_testkit` is test-only; if it destabilizes workspace builds, remove the crate and the `Cargo.toml` workspace member in one revert.

## Progress log

- 2026-04-28: Plan branch created from `origin/staging`.
- 2026-04-28: Companion gist created at <https://gist.github.com/serrrfirat/ab14c227f4dceab7e375d509d3eb5bdd>.

## Decision log

- Keep #2987 as substrate/cutover authority.
- Keep #3020 as blocking compatibility authority.
- Keep #3031 as product-surface migration authority.
- Use `docs/reborn/plans/active/` instead of `docs/superpowers/plans/` because this is a Reborn architecture plan that should be discoverable from the Reborn docs map.
- Add harness scaffolding before broad implementation so future agent tasks have repo-local maps and mechanical feedback.
