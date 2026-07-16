# Live Canary Signal Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Make live-canary reports distinguish contract health, behavioral quality, and infrastructure incidents while removing invalid or hermetic work from the live tier.

**Architecture:** Keep the existing structured ProbeResult schema and teach the reporter to aggregate tier-specific totals. Harden the Reborn live-QA harness at its boundaries: an isolated temporary agent workspace, a Slack indexing preflight before the global recall question, and durable evidence as the creation oracle. Move deterministic and mock-backed jobs to their owning CI/release workflows without changing their local runner entrypoints.

**Tech Stack:** Python 3.12 unittest/pytest, GitHub Actions YAML, Bash, Rust recorded-behavior tests, Playwright-backed Reborn WebUI v2 harness.

## Global Constraints

- Live model answers are one-shot; do not add whole-case answer retries.
- Missing result-tier metadata fails closed as a blocking contract.
- Wrong-destination, duplicate-delivery, authorization, and provider-readback assertions remain blocking.
- Live LLM results remain supplemental; deterministic tests own regression gating.
- The agent process cannot use the repository, artifact directory, or canary logs as its workspace.
- Preserve existing local lane names in scripts/live-canary/run.sh for developer compatibility.
- Do not change production persistence schemas, APIs, authentication, or authorization behavior.
- Stage only files named by this plan; leave .agents/ and docs/reborn/memory-rd/ untouched.

---

### Task 1: Tier-specific canary aggregation and rendering

**Files:**
- Modify: scripts/live-canary/notify_slack.py
- Modify: scripts/live-canary/test_notify_slack.py

**Interfaces:**
- Consumes: _normalize_result_classification(entry) -> _ResultClassification.
- Produces: LaneReport.contract_passed, contract_total, behavioral_passed, behavioral_total; _tier_health_lines(report) -> list[str].

- [ ] **Step 1: Write the failing reporter tests**

Add a fixture containing two successful contracts, one successful behavioral case, and two failed nonblocking behavioral cases:

~~~python
def test_report_separates_contract_health_from_behavioral_warnings(self):
    with tempfile.TemporaryDirectory() as tmpdir:
        lane_dir = Path(tmpdir) / "reborn-webui-v2-live-qa" / "reborn-webui-v2" / "run"
        lane_dir.mkdir(parents=True)
        entries = [
            result_entry("contract_a", True, "contract", True),
            result_entry("contract_b", True, "contract", True),
            result_entry("behavior_a", True, "behavioral", False),
            result_entry("behavior_b", False, "behavioral", False),
            result_entry("behavior_c", False, "behavioral", False),
        ]
        (lane_dir / "results.json").write_text(json.dumps({"results": entries}))
        report = notify.collect_lane(lane_dir)

    self.assertEqual((report.contract_passed, report.contract_total), (2, 2))
    self.assertEqual((report.behavioral_passed, report.behavioral_total), (1, 3))
    self.assertEqual(report.status, "warn")
    rendered = json.dumps(notify.slack_payload([report], None, None))
    self.assertIn("Contracts: 2/2 passed", rendered)
    self.assertIn("Behavioral quality: 1/3 passed, 2 warnings", rendered)
    self.assertNotIn("3/5 passed", rendered)
~~~

Add separate tests proving infrastructure results increment neither product-failure count and older untyped failures increment contract_total and fail closed.

- [ ] **Step 2: Run the new tests and verify RED**

Run:

~~~bash
python3 -m pytest scripts/live-canary/test_notify_slack.py \
  -k 'separates_contract_health or tier_totals' -v
~~~

Expected: FAIL because LaneReport has no tier counters and the payload has no tier health lines.

- [ ] **Step 3: Implement counters at parse time**

Extend LaneReport:

~~~python
contract_passed: int = 0
contract_total: int = 0
behavioral_passed: int = 0
behavioral_total: int = 0
~~~

In parse_results_json, normalize every entry before the success branch:

~~~python
classification = _normalize_result_classification(entry)
if classification.tier == "behavioral":
    report.behavioral_total += 1
    if entry.get("success"):
        report.behavioral_passed += 1
else:
    report.contract_total += 1
    if entry.get("success"):
        report.contract_passed += 1
~~~

Treat JUnit-only tests as contracts so legacy and mock reports retain a meaningful denominator.

- [ ] **Step 4: Implement tier-specific rendering**

Add:

~~~python
def _tier_health_lines(report: LaneReport) -> list[str]:
    lines: list[str] = []
    if report.contract_total:
        lines.append(
            f"Contracts: {report.contract_passed}/{report.contract_total} passed"
        )
    if report.behavioral_total:
        warnings = report.behavioral_total - report.behavioral_passed
        suffix = (
            f", {warnings} warning{'s' if warnings != 1 else ''}"
            if warnings
            else ""
        )
        lines.append(
            "Behavioral quality: "
            f"{report.behavioral_passed}/{report.behavioral_total} passed{suffix}"
        )
    if report.inconclusive:
        lines.append(
            f"Infrastructure/preconditions: {report.inconclusive} inconclusive"
        )
    return lines
~~~

Render these lines in Slack and GitHub output for structured Reborn QA reports. Keep combined execution totals only in secondary detail rows.

- [ ] **Step 5: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/live-canary/test_notify_slack.py -v
git add scripts/live-canary/notify_slack.py scripts/live-canary/test_notify_slack.py
git commit -m "fix(canary): separate contract and behavioral health"
~~~

Expected: all reporter tests PASS.

### Task 2: Isolate the live agent workspace

**Files:**
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py

**Interfaces:**
- Produces: isolated_agent_workspace(repository_root: Path, output_dir: Path) -> ContextManager[Path].
- Changes: start_reborn_server(binary, reborn_home, output_dir, workspace_dir, extra_env=None).

- [ ] **Step 1: Write failing boundary tests**

~~~python
def test_isolated_agent_workspace_is_outside_repo_and_artifacts(self):
    with tempfile.TemporaryDirectory() as tmpdir:
        repo = Path(tmpdir) / "checkout"
        output = repo / "artifacts" / "live-canary"
        output.mkdir(parents=True)
        with run_live_qa.isolated_agent_workspace(repo, output) as workspace:
            self.assertFalse(workspace.is_relative_to(repo))
            self.assertFalse(workspace.is_relative_to(output))
            self.assertTrue(workspace.is_dir())
        self.assertFalse(workspace.exists())
~~~

Add test_start_reborn_server_uses_explicit_agent_workspace by patching subprocess.Popen and wait_for_ready, then asserting Popen receives cwd=workspace. Update existing fake server functions to accept the workspace argument.

- [ ] **Step 2: Verify RED**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'isolated_agent_workspace or explicit_agent_workspace' -v
~~~

Expected: FAIL because the allocator is absent and startup derives output_dir/workspace.

- [ ] **Step 3: Implement the allocator**

~~~python
@contextmanager
def isolated_agent_workspace(
    repository_root: Path,
    output_dir: Path,
) -> Iterator[Path]:
    repository_root = repository_root.resolve()
    output_dir = output_dir.resolve()
    with tempfile.TemporaryDirectory(
        prefix="ironclaw-reborn-live-qa-workspace-"
    ) as raw:
        workspace = Path(raw).resolve()
        for forbidden in (repository_root, output_dir):
            if workspace == forbidden or workspace.is_relative_to(forbidden):
                raise LiveQaError(
                    f"agent workspace {workspace} must be outside {forbidden}"
                )
        yield workspace
~~~

- [ ] **Step 4: Thread the workspace through process lifetime**

Remove the derived workspace from start_reborn_server and pass the explicit path to subprocess.Popen. In run_cases, enter isolated_agent_workspace before startup and keep stop_process plus trace export inside that context so cleanup happens after shutdown.

- [ ] **Step 5: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py -v
git add scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
git commit -m "fix(canary): isolate live agent workspace"
~~~

Expected: the complete harness unit suite PASS.

### Task 3: Separate Slack indexing freshness from model quality

**Files:**
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py

**Interfaces:**
- Produces: _wait_for_slack_search_index(ctx, marker, timeout, poll_interval=5.0) -> dict[str, object].
- Preserves exactly one model journey after the nonce is indexed.

- [ ] **Step 1: Write failing freshness tests**

~~~python
def test_global_last_sent_skips_model_when_slack_index_is_stale(self):
    with (
        patch.object(
            run_live_qa,
            "_wait_for_slack_search_index",
            new=AsyncMock(
                return_value={"indexed": False, "attempts": 3, "latency_ms": 20}
            ),
        ),
        patch.object(
            run_live_qa,
            "_slack_correctness_chat_reply",
            new=AsyncMock(),
        ) as chat,
    ):
        result = asyncio.run(
            run_live_qa.case_qa_10g_slack_last_message_sent_global(
                self._dummy_ctx()
            )
        )
    self.assertFalse(result.success)
    self.assertTrue(result.details["inconclusive"])
    self.assertEqual(result.details["failure_class"], "infrastructure")
    self.assertEqual(
        result.details["failure_category"], "slack_search_index_stale"
    )
    chat.assert_not_awaited()
~~~

Patch token/channel/seeding helpers as existing QA10 tests do. Add a polling test with two empty observations followed by a hit, and a test that indexed data plus a wrong answer yields failure_class=model_quality.

- [ ] **Step 2: Verify RED**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'slack_search_index or global_last_sent' -v
~~~

Expected: FAIL because the polling helper and infrastructure classification are absent.

- [ ] **Step 3: Implement bounded indexing polling**

~~~python
async def _wait_for_slack_search_index(
    ctx: LiveQaContext,
    *,
    marker: str,
    timeout: float,
    poll_interval: float = 5.0,
) -> dict[str, object]:
    started = time.monotonic()
    attempts = 0
    last_observation: dict[str, object] = {}
    while True:
        attempts += 1
        last_observation = await _slack_search_marker_hits(ctx, marker=marker)
        hits = last_observation.get("hits")
        if last_observation.get("ok") and isinstance(hits, list) and hits:
            return {
                "indexed": True,
                "attempts": attempts,
                "latency_ms": int((time.monotonic() - started) * 1000),
            }
        if time.monotonic() - started >= timeout:
            return {
                "indexed": False,
                "attempts": attempts,
                "latency_ms": int((time.monotonic() - started) * 1000),
                "last_error": last_observation.get("error"),
            }
        await asyncio.sleep(poll_interval)
~~~

- [ ] **Step 4: Gate the global journey**

After seeding, poll using REBORN_WEBUI_V2_LIVE_QA_SLACK_INDEX_TIMEOUT_SECONDS with a 90-second default. If the nonce is absent, return a nonblocking infrastructure/inconclusive result and do not call chat. If indexed but absent from the model answer, return behavioral model_quality/answer_mismatch. Persist only latency, attempts, indexed state, and bounded error text.

- [ ] **Step 5: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'slack_search_index or global_last_sent or last_message_sent' -v
cargo test --test reborn_qa_recorded_behavior --features libsql \
  contract_slack_recent_message_reads_the_synthetic_conversation -- --nocapture
git add scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
git commit -m "fix(canary): preflight Slack search freshness"
~~~

Expected: selected Python and Rust tests PASS; scoped QA10G still forbids indexed search.

### Task 4: Make durable evidence authoritative

**Files:**
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
- Modify: tests/reborn_qa_recorded_behavior.rs
- Modify only if required: tests/fixtures/llm_traces/reborn_qa/slack_entity_hygiene.json

**Interfaces:**
- _routine_creation_case treats creation markers and routine/trigger wording as diagnostic but still requires a finalized reply and new durable trigger record.
- QA5C uses capability evidence instead of the substring .md.

- [ ] **Step 1: Write failing routine tests**

~~~python
def test_routine_creation_accepts_reformatted_reply_when_trigger_is_durable(self):
    async def fake_chat(_ctx, **kwargs):
        self.assertFalse(kwargs["enforce_marker"])
        self.assertEqual(kwargs["required_text"], [])
        return run_live_qa.ProbeResult(
            provider="test",
            mode="live:routine",
            success=True,
            latency_ms=1,
            details={"text_excerpt": "Trigger created successfully."},
        )
    # Patch count to zero and wait helper to return one record; assert success.
~~~

Retain the existing no-trigger failure test. Add a QA5C delegation test that .md is absent from forbidden_text while Google Docs capability requirements remain.

- [ ] **Step 2: Verify RED**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'routine_creation or strategy_doc_knowledge' -v
~~~

Expected: FAIL because creation still enforces markers and literal prose.

- [ ] **Step 3: Make creation prose diagnostic**

Pass required_text=[] and enforce_marker=False from _routine_creation_case. Store the original marker and expected terms in details for debugging. Continue waiting for _wait_for_trigger_record_after_count and fail when no new record appears. Do not alter delivery marker, wrong-channel, duplicate, schedule, or delivery-target assertions.

- [ ] **Step 4: Replace QA5C broad substring matching**

Remove .md from forbidden_text. Require terminal evidence for google-docs.create_document and google-docs.read_content using the existing current-turn capability evidence helper. Retain explicit authentication, authorization, and local-workspace fallback phrases.

- [ ] **Step 5: Strengthen synthetic entity replay**

In contract_slack_entity_hygiene_humanizes_the_chained_user_id, assert the final reply includes Canary User and excludes U0CANARY and D0CANARY. Prove RED by temporarily asserting against a raw-ID synthetic string, run the targeted test, then apply the real assertion and restore the scrubbed fixture.

- [ ] **Step 6: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'routine_creation or strategy_doc_knowledge' -v
cargo test --test reborn_qa_recorded_behavior --features libsql \
  contract_slack_entity_hygiene_humanizes_the_chained_user_id -- --nocapture
scripts/ci/check-reborn-qa-fixtures.sh
git add scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  tests/reborn_qa_recorded_behavior.rs
git commit -m "fix(canary): prefer durable routine evidence"
~~~

Stage the fixture separately only if it actually changed.

### Task 5: Consolidate repeated scheduled connections

**Files:**
- Modify: scripts/reborn_webui_v2_live_qa/run_live_qa.py
- Modify: scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
- Modify: .github/workflows/live-canary.yml

**Interfaces:**
- Produces SCHEDULED_REDUNDANT_CONNECTION_CASES and _scheduled_non_telegram_case_names().
- Manual --non-telegram-qa-cases continues selecting all 47 cases.

- [ ] **Step 1: Write the failing scheduled-selection test**

~~~python
def test_scheduled_suite_keeps_one_connect_journey_per_integration(self):
    scheduled = run_live_qa._scheduled_non_telegram_case_names()
    retained = {
        "qa_2a_gmail_connect",
        "qa_2b_calendar_connect",
        "qa_2c_drive_connect",
        "qa_3a_slack_connect",
        "qa_4b_github_connect",
        "qa_6b_sheets_connect",
    }
    removed = {
        "qa_4a_gmail_connect",
        "qa_5a_slack_connect",
        "qa_5b_drive_connect",
        "qa_6a_gmail_connect",
        "qa_7a_slack_product_channel_connect",
        "qa_7b_sheets_connect",
        "qa_8a_slack_connect",
        "qa_9a_slack_connect",
    }
    self.assertTrue(retained.issubset(scheduled))
    self.assertTrue(removed.isdisjoint(scheduled))
    self.assertEqual(len(scheduled), 39)
~~~

Change the workflow-shard coverage test to compare the matrix against this scheduled helper, while the manual suite test remains 47.

- [ ] **Step 2: Verify RED**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'scheduled_suite or workflow_shards_cover' -v
~~~

Expected: FAIL because the helper is absent and the workflow contains all repeated connection cases.

- [ ] **Step 3: Add the scheduled helper**

Define the exact removed set above and return default non-Telegram cases except that set. Do not change _selected_case_names.

- [ ] **Step 4: Update the shard matrix**

Remove the eight repeated IDs from matrix.include, ALL_SHARD_CASES, and REBORN_WEBUI_V2_GOOGLE_CASES. Keep all non-connect cases in their shards and rename labels that claim removed rows, such as QA 5C-5D and QA 9B-9D.

- [ ] **Step 5: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/reborn_webui_v2_live_qa/test_run_live_qa.py \
  -k 'non_telegram_qa_suite or scheduled_suite or workflow_shards_cover' -v
git add .github/workflows/live-canary.yml \
  scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
git commit -m "test(canary): consolidate live connection journeys"
~~~

Expected: manual selection is 47 and scheduled workflow coverage is 39 unique cases.

### Task 6: Move hermetic and compatibility jobs

**Files:**
- Create: .github/workflows/upgrade-compatibility.yml
- Create: scripts/live-canary/test_workflow_tiering.py
- Modify: .github/workflows/live-canary.yml
- Modify: .github/workflows/reborn-tests.yml

**Interfaces:**
- Live Canary exposes only live-dependent lanes.
- Tests (Reborn) owns mock-auth-e2e and workflow-hermetic-e2e and includes them in its roll-up.
- Upgrade Compatibility owns scripts/live-canary/upgrade-canary.sh.
- replay-gate.yml remains the GitHub owner of deterministic replay.

- [ ] **Step 1: Write failing workflow ownership tests**

Create a standard-library unittest with exact sets:

~~~python
LIVE_LANES = {
    "public-smoke",
    "persona-rotating",
    "private-oauth",
    "provider-matrix",
    "release-public-full",
    "auth-live-seeded",
    "auth-browser-consent",
    "reborn-webui-v2-live-qa",
}
NON_LIVE_LANES = {
    "deterministic-replay",
    "auth-smoke",
    "auth-full",
    "auth-channels",
    "workflow-canary",
    "upgrade-canary",
}

def test_live_dispatch_choices_are_live_only(self):
    self.assertEqual(
        set(dispatch_lane_options(LIVE_WORKFLOW_TEXT)) - {"all"},
        LIVE_LANES,
    )
    for lane in NON_LIVE_LANES:
        self.assertNotRegex(
            LIVE_WORKFLOW_TEXT,
            rf"(?m)^  {re.escape(lane)}:$",
        )

def test_reborn_ci_owns_mock_suites(self):
    self.assertIn("  mock-auth-e2e:", REBORN_WORKFLOW_TEXT)
    self.assertIn("  workflow-hermetic-e2e:", REBORN_WORKFLOW_TEXT)
    self.assertIn("- mock-auth-e2e", REBORN_WORKFLOW_TEXT)
    self.assertIn("- workflow-hermetic-e2e", REBORN_WORKFLOW_TEXT)

def test_upgrade_has_dedicated_manual_workflow(self):
    self.assertIn("name: Upgrade Compatibility", UPGRADE_WORKFLOW_TEXT)
    self.assertIn("workflow_dispatch:", UPGRADE_WORKFLOW_TEXT)
    self.assertIn(
        "scripts/live-canary/upgrade-canary.sh",
        UPGRADE_WORKFLOW_TEXT,
    )
~~~

- [ ] **Step 2: Verify RED**

~~~bash
python3 -m pytest scripts/live-canary/test_workflow_tiering.py -v
~~~

Expected: FAIL because non-live jobs remain in live-canary.yml and no upgrade workflow exists.

- [ ] **Step 3: Remove non-live jobs and choices**

Remove deterministic-replay, auth-smoke, auth-full, auth-channels, workflow-canary, and upgrade-canary from live workflow dispatch options, job blocks, and canary-report.needs. Keep every local Bash lane.

- [ ] **Step 4: Add Reborn CI jobs**

Add mock-auth-e2e with profile matrix smoke/full/channels using the prior checkout, Rust, Python 3.12, Playwright, WASM build, and cache steps. Run scripts/live-canary/run.sh with LANE matching each auth profile.

Add workflow-hermetic-e2e using the prior workflow-canary steps with LANE=workflow-canary, PROVIDER=mock, and PLAYWRIGHT_INSTALL=skip. Add both job IDs to the reborn-tests roll-up needs and explicit result validation.

- [ ] **Step 5: Create Upgrade Compatibility**

Create a manual workflow with previous_ref and current_ref inputs, tag-aware checkout, stable Rust/cache setup, and the existing run.sh upgrade lane. Upload scrubbed artifacts for 30 days.

- [ ] **Step 6: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/live-canary/test_workflow_tiering.py -v
ruby -e 'require "yaml"; ARGV.each { |f| YAML.load_file(f, aliases: true) }' \
  .github/workflows/live-canary.yml \
  .github/workflows/reborn-tests.yml \
  .github/workflows/upgrade-compatibility.yml
git add .github/workflows/live-canary.yml \
  .github/workflows/reborn-tests.yml \
  .github/workflows/upgrade-compatibility.yml \
  scripts/live-canary/test_workflow_tiering.py
git commit -m "ci: separate live and hermetic canary lanes"
~~~

Expected: ownership tests PASS and all YAML parses.

### Task 7: Report persona credential coverage

**Files:**
- Modify: scripts/live-canary/run.sh
- Modify: scripts/live-canary/test_run_dispatch.py

**Interfaces:**
- Adds presence-only persona_live_integrations and persona_stubbed_integrations lines to env-summary.txt.

- [ ] **Step 1: Write failing persona summary test**

Run persona-rotating with a temporary fake cargo executable, set only the GitHub and Slack credential variables, locate env-summary.txt, and assert:

~~~python
self.assertIn("persona_live_integrations=github,slack", summary)
self.assertIn(
    "persona_stubbed_integrations=google,telegram,composio",
    summary,
)
self.assertNotIn("secret-github-value", summary)
self.assertNotIn("secret-slack-value", summary)
~~~

- [ ] **Step 2: Verify RED**

~~~bash
python3 -m pytest scripts/live-canary/test_run_dispatch.py -k persona -v
~~~

Expected: FAIL because no presence summary exists.

- [ ] **Step 3: Implement presence-only reporting**

Add a write_persona_credential_summary helper that maps github, google, slack, telegram, and composio to their LIVE_CANARY environment names, appends names to live or stubbed arrays based only on non-empty presence, and prints comma-separated names. Call it from write_env_summary only for persona-rotating. Never print values.

- [ ] **Step 4: Verify GREEN and commit**

~~~bash
python3 -m pytest scripts/live-canary/test_run_dispatch.py -v
shellcheck scripts/live-canary/run.sh scripts/live-canary/upgrade-canary.sh
git add scripts/live-canary/run.sh scripts/live-canary/test_run_dispatch.py
git commit -m "chore(canary): report live persona integrations"
~~~

Expected: dispatcher tests PASS and ShellCheck is clean.

### Task 8: Update operator documentation

**Files:**
- Modify: scripts/live-canary/README.md
- Modify: docs/internal/live-canary.md
- Modify: CHANGELOG.md

**Interfaces:**
- Documents live-only lanes, CI/release owners, report tiers, Slack freshness, workspace isolation, and persona evidence.

- [ ] **Step 1: Update README lane ownership**

List only the eight GitHub live lanes. State that mock auth/workflow suites run in Tests (Reborn), deterministic replay runs in Replay Gate, and upgrade checks run in Upgrade Compatibility. Preserve local command examples for the stable run.sh lanes.

- [ ] **Step 2: Document report semantics and operations**

Add a concrete example:

~~~text
Contracts: 44/44 passed
Behavioral quality: 1/3 passed, 2 warnings
Infrastructure/preconditions: 0 inconclusive
~~~

Document live eligibility, search-index latency, isolated agent workspaces, dedicated upgrade dispatch, and persona live-versus-stubbed summaries.

- [ ] **Step 3: Add changelog entry**

Under Unreleased, add a testing/CI bullet covering separated health signals, workspace isolation, Slack freshness preflight, and moved mock/upgrade jobs.

- [ ] **Step 4: Verify and commit**

~~~bash
rg -n "deterministic-replay|auth-smoke|workflow-canary|upgrade-canary" \
  scripts/live-canary/README.md docs/internal/live-canary.md
git diff --check
git add scripts/live-canary/README.md docs/internal/live-canary.md CHANGELOG.md
git commit -m "docs: clarify live canary test tiers"
~~~

Expected: old lane names appear only as local commands or new workflow ownership.

### Task 9: Full verification, review, and draft PR

**Files:**
- Review every file changed since origin/main.

**Interfaces:**
- Produces a verified draft PR against main.

- [ ] **Step 1: Run affected Python suites**

~~~bash
python3 -m pytest \
  scripts/live-canary/test_notify_slack.py \
  scripts/live-canary/test_run_dispatch.py \
  scripts/live-canary/test_workflow_tiering.py \
  scripts/reborn_webui_v2_live_qa/test_run_live_qa.py -v
~~~

Expected: all tests PASS.

- [ ] **Step 2: Run deterministic Reborn QA and fixture checks**

~~~bash
cargo test --test reborn_qa_recorded_behavior --features libsql -- --nocapture
scripts/ci/check-reborn-qa-fixtures.sh
~~~

Expected: Rust tests and fixture validation PASS.

- [ ] **Step 3: Run workflow, shell, formatting, and safety checks**

~~~bash
ruby -e 'require "yaml"; ARGV.each { |f| YAML.load_file(f, aliases: true) }' \
  .github/workflows/live-canary.yml \
  .github/workflows/reborn-tests.yml \
  .github/workflows/upgrade-compatibility.yml
shellcheck scripts/live-canary/run.sh scripts/live-canary/upgrade-canary.sh
cargo fmt --all -- --check
git diff --check
scripts/pre-commit-safety.sh
~~~

Expected: every command exits zero.

- [ ] **Step 4: Run required workspace-wide Clippy**

Read .claude/rules/review-discipline.md immediately before running its documented workspace-wide command. Require zero warnings. If an unrelated baseline failure occurs, record the exact evidence in the PR instead of claiming success.

- [ ] **Step 5: Audit scope and risks**

~~~bash
git diff origin/main...HEAD --stat
git diff origin/main...HEAD -- .github scripts tests docs CHANGELOG.md
git status --short
rg -n '\.unwrap\(\)|\.expect\(' \
  scripts/reborn_webui_v2_live_qa/run_live_qa.py \
  scripts/live-canary/notify_slack.py
~~~

Confirm no user-owned files are staged, no live identifiers or secrets entered fixtures, no answer retry was added, wrong-channel and duplicate delivery remain blocking, workflow needs lists match moved jobs, and rollback/compatibility are described.

- [ ] **Step 6: Request review and address findings**

Use the requesting-code-review skill with BASE_SHA=origin/main and HEAD_SHA=HEAD. Fix all Critical and Important findings and rerun affected checks.

- [ ] **Step 7: Push and open the draft PR**

~~~bash
git push -u origin codex/canary-signal-reliability
gh pr create --draft \
  --base main \
  --head codex/canary-signal-reliability \
  --title "fix(canary): make live QA signals trustworthy" \
  --body-file /tmp/ironclaw-canary-pr-body.md
~~~

The PR body must include root causes, a tier-ownership table, compatibility and rollback, exact checks, any blocked check, and an exact-head live validation request.
