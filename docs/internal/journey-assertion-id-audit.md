# Journey assertion ID audit

Epic #6524, workstream 3: "Use provider-issued IDs for assertions instead of
searching by possibly duplicated names."

## Method

Grepped `tests/e2e/scenarios/*.py` for equality/membership checks against a
`title` / `name` / `subject` / `summary` field on a *provider* resource
(GitHub issue/PR/branch/release, Google Calendar event / Drive file / Docs /
Sheets, Slack channel/message), then read each site in context to judge
whether a same-named collision is actually reachable: is the name a fixture
constant, does it include a random/unique suffix, does the surrounding test
run alone against a fresh provider world, or does it run in a shared/module-
scoped world alongside other journeys that could create a same-named
resource. IronClaw-internal resources (chat threads, installed-extension
registry entries, registered tools, filesystem entries) were treated as
out of scope — the epic is about *provider* effects, and those identifiers
are already effectively unique keys within IronClaw's own registries.

30 name-based matches were found across `tests/e2e/scenarios/`. Of those,
6 touch genuine provider resources with any plausible collision path; the
rest are matches against fixture constants, just-created-and-immediately-
read-back responses (no list search involved), or IronClaw-internal
identifiers, and are not migrated.

## High risk — real collision path exists

### 1. `tests/e2e/scenarios/test_reborn_qa_trace_full_path.py:1080-1092` — `_assert_google_provider_outcome`

```python
title = _google_created_resource_name(create_call)   # LLM-requested name from the trace
files = await client.get(..., params={"q": f"name = '{title}' and trashed = false"})
matching = [item for item in files.json()["files"] if item["name"] == title]
assert matching, f"created Google resource missing for {case}: {files.text}"
resource_id = matching[-1]["id"]
```

`title` comes from what the model asked to name the file/doc/sheet in a
replayed trace — not a provider-issued id. `test_qa_journey_provider_leg_replays_through_emulate`
and `test_mutating_qa_journeys_replay_in_reverse_against_shared_provider_world`
(same file) run many QA cases back-to-back against one module-scoped Emulate
Google world (`provider_servers` fixture, `scope="module"`). If two cases
happen to request the same resource name — plausible, since these are
harvested/synthetic traces and several already reuse generic prep-doc-style
names — `matching[-1]["id"]` silently picks whichever one sorts last instead
of failing, and every assertion downstream (content check, sheet values)
runs against the wrong journey's resource.

**Recommended change:** don't derive the resource id by re-querying and
name-matching. The tool-call *result* for `google-docs__create_document` /
`google-drive__upload_file` / `google-sheets__create_spreadsheet` already
carries the provider-issued id in the turn/step response; thread that id
through instead of `_google_created_resource_name` + a `name=` search.
Requires reading `step["response"]` result payloads (today
`_recorded_provider_calls` only reads the *request* side of tool calls), so
this is a real code change, not a search-and-replace — cannot be attempted
without the Reborn binary to verify against.

### 2. `tests/e2e/scenarios/test_reborn_qa_trace_full_path.py:1127-1159` — `_assert_google_provider_baseline`

Same `name = '{title}'` search, used to assert a resource does *not* yet
exist ("prove this journey cannot observe mutations from an earlier
journey"). This is the isolation guard the epic exists to protect, and it is
built on the exact identification mechanism that's at risk: if an earlier
journey created a same-named resource for an unrelated reason, this
assertion silently fails to catch real leakage (false pass), or spuriously
fires (false failure) if a completely unrelated case happens to use the same
generic title. Same fix direction as #1: match on the id the earlier
journey's own tool-call result reported, not a name re-derived from a later
journey's request.

### 3. `tests/e2e/scenarios/test_reborn_qa_trace_full_path.py:1637-1662` — provider-fault idempotency check

```python
issues = await github_json(..., "GET", "/repos/nearai/ironclaw/issues")
if fault_case.expected_outcome == "committed_without_ack":
    expected_title = arguments["title"]
    assert [issue["title"] for issue in issues].count(expected_title) == 1, issues
else:
    assert len(issues) == 1, issues
    attempted_title = arguments.get("title")
    if attempted_title is not None:
        assert issues[0]["title"] != attempted_title, issues
```

This lists *every* open issue in `nearai/ironclaw` with no scoping beyond
`state=open`, in a repo shared across the module-scoped provider world.
`len(issues) == 1` (else-branch) is only correct if nothing else in the
shared world left an open issue behind — a real ordering hazard, given the
file explicitly has a "replays in reverse order" test for this same world.
The `count(expected_title) == 1` branch is inherently name-based by
necessity (it is testing "did a retried create silently duplicate", and a
duplicate wouldn't share an id with anything), but it should at minimum be
scoped to issues created after the test's own baseline snapshot, not the
repo's entire open-issue list. Not migrated: cannot run this file without
the Reborn binary.

## Medium risk

### 4. `tests/e2e/scenarios/test_reborn_qa_trace_full_path.py:1071-1078` — gmail subject search

```python
subject = send["arguments"]["message"]["subject"]
listed = await client.get(..., params={"q": f"subject:{subject}"})
assert listed.json().get("messages"), f"sent message missing for {case}"
```

Existence-only check (no id is threaded onward), and subjects in the traces
are QA-case-specific, so a same-subject collision across cases is less
likely than the Drive/Docs/Sheets case above, but the search is still
against a shared module-scoped inbox. Left alone: can't run, and the blast
radius if it did collide is a false pass, not silent misattribution to a
different resource.

## Low risk — left alone deliberately

- `test_reborn_emulate_full_path.py:137` — `next(item for item in issues if item.get("title") == title)`. `title` is `f"[canary] reborn-emulate-github-{uuid.uuid4().hex[:8]}"` — collision-proof by construction (an embedded random suffix already does the job a provider id would do). No change recommended. Also needs `ironclaw_binary` (legacy) to run; not attempted.
- `test_owner_scope.py:132-146` (`_wait_for_http_thread`) and similar "poll until visible then use the id" helpers — these match on a distinctive title *fragment* only until the resource's own id becomes discoverable (they return `thread["id"]` once found, and all later assertions use that id). This is IronClaw's own internal chat thread, not a provider resource, and is an inherent pattern for "find the thing I just created before I know its id" — out of scope for this epic.
- `test_slack_e2e.py:348`, `test_reborn_qa_trace_full_path.py:358,1290`, `test_emulate_reborn_provider_contracts.py` (pre-fix) — Slack channel lookup by `name == "reborn-alerts"`. This is a fixed, seeded fixture channel (`fixtures/emulate/slack.yaml`), read-only lookup of static config, not a journey-created resource; Slack also enforces channel-name uniqueness within a workspace. No change recommended.
- `test_emulate_reborn_provider_contracts.py` fork/branch/release assertions (`item["full_name"] == fork["full_name"]`, `item["name"] == branch_name`, `item["tag_name"] == "reborn-emulate-v1"`) — forks are one-per-account, and git branches/tags don't have a separate provider-issued id distinct from their name; matching by name is inherent to the resource type, not a stand-in for an id. No change recommended.
- `test_emulate_reborn_provider_contracts.py` Drive/Docs/Sheets/Slack-message assertions where the code already reads its own just-created response (`created.json()["title"] == marker`, `pr["title"] == pr_title`, `posted["message"]["text"] == text`) — no list search happens, so there's no wrong-resource risk to fix.
- `test_oauth_credential_fallback.py`, `test_routine_oauth_credential_injection.py`, `test_mission_gmail_3133.py`, `test_v2_engine_oauth_google.py` — `ext["name"] == "gmail"` / `"google_drive"`. This is IronClaw's own installed-extension registry, where `name` is the singleton `ExtensionName` per install (see `crates/ironclaw_common/src/identity.rs`), not a display string that can be duplicated. Out of scope.
- `test_reborn_webui_v2_filesystem_api.py`, `test_wasm_lifecycle.py`, `test_tool_permissions.py`, `test_v2_engine_tool_lifecycle.py`, `test_reborn_webui_v2_legacy_skills.py`, `test_reborn_qa_trace_replay.py` — matches on IronClaw's own tool-catalog / filesystem-entry / mock-trace function names. Not provider resources. Out of scope.

## Migrated and verified (this branch)

Only `tests/e2e/scenarios/test_emulate_reborn_provider_contracts.py` runs
without the Reborn binary (it talks to the Emulate REST APIs directly over
`httpx`, no `ironclaw`/`ironclaw-legacy` process involved). Two sites in that
file had a real, if narrow, collision path and were migrated:

1. **Calendar event lookup** (`test_emulate_google_covers_reborn_gsuite_read_inputs`,
   was `event["summary"] == "Reborn planning sync"` /
   `"PepsiCo procurement sync"`) — now also requires
   `event["id"] == "evt_reborn_planning_sync"` /
   `"evt_pepsico_procurement_sync"`, the provider-issued ids already present
   in `fixtures/emulate/google_gmail.yaml`.
2. **GitHub issue list membership** (`test_emulate_github_covers_reborn_repo_surfaces`,
   was `assert any(item["title"] == issue_title for item in issues)`) — now
   `assert any(item["number"] == issue["number"] for item in issues)`, using
   the issue number returned from the create call instead of re-matching the
   title.

Both are pure hardening: today's fixture data doesn't actually duplicate
these names within a single run of this file, so the change doesn't alter
current pass/fail behavior, only what a future duplicate would do (fail loud
at the right assertion instead of silently matching an unrelated resource,
or in the calendar case, matching an item purely because its summary
happened to repeat).

### Verification

```
cd tests/e2e && ./.venv/bin/python -m pytest scenarios/test_emulate_reborn_provider_contracts.py -q
# 10 passed, 3 skipped, 1 failed (pre-existing, unrelated: test_emulate_github_covers_reborn_repo_surfaces
# fails on `workflow_runs["total_count"] == 1`, a GitHub Actions run-count assertion this branch didn't
# touch; identical failure reproduces on origin/main before any edit in this branch).
```

Both changed assertions were sabotage-verified: temporarily forced each to
compare against a wrong id (`evt_wrong_id`, `issue["number"] + 9999`), reran
the specific test, confirmed it failed at exactly that assertion, then
reverted the sabotage. Diff is otherwise a no-op relative to the pre-existing
1-failed/10-passed/3-skipped baseline on this file.

## Not attempted

Everything under "High risk" and "Medium risk" above lives in
`test_reborn_qa_trace_full_path.py`, which needs a built Reborn binary
(`ironclaw_reborn_binary`/`provider_servers` fixtures) plus Emulate. The
binary is not built in this environment and a from-scratch build takes on
the order of 40 minutes, which this task's setup notes rule out. Per the
task's hard rule ("do not change assertions you cannot run"), these are
documented above with a recommended direction but not touched. The
recommended fix for the two high-risk sites requires plumbing the tool-call
*result* payload (which carries the provider-issued id) through
`_recorded_provider_calls`/`_google_created_resource_call`, which today only
look at the request side of `step["response"]["tool_calls"]"` — a nontrivial
change that needs the harness running to verify against real Emulate
responses.
