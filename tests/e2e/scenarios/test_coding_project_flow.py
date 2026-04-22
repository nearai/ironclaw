"""E2E scenarios for the coding-agent UX: project chrome, `!`-shell mode, persistence.

Covers the browser-visible surface of the coding-agent work:
  - Creating a project via the REST API and setting it active.
  - The conversation-chrome bar renders project name + folder + branch.
  - Typing `!` in the input toggles the shell-mode badge.
  - Submitting `!echo hello` dispatches the shell tool through
    /api/chat/send (mode=shell), emits `shell_command` + `shell_output`
    SSE events, and renders a distinct monospace turn card with an
    exit-code badge.
  - Reloading the thread replays the shell turn from history (the
    backend pairs `shell_command` + `shell_output` conversation
    messages into a `TurnInfo.shell` payload).

Scenarios #6 and #7 in `tests/live/README.md` — coding-repo skill
activation driving a real branch + draft PR against `nearai/ironclaw`
— live in a separate live-tier file because they need a real LLM and a
write-scope GH_TOKEN. This file uses the mock LLM harness so it runs
on every CI pass.
"""

import tempfile
from pathlib import Path

import httpx
import pytest
from helpers import AUTH_TOKEN, SEL


def _auth_headers() -> dict[str, str]:
    return {"Authorization": f"Bearer {AUTH_TOKEN}"}


async def _create_active_project(ironclaw_server: str, workspace_path: Path) -> str:
    """Create a project via REST, set it active, return its id.

    The workspace path must exist — the backend validates it on
    `project_create` to avoid dangling references. Using a caller-owned
    tempdir keeps the test hermetic.
    """
    workspace_path.mkdir(parents=True, exist_ok=True)
    async with httpx.AsyncClient(timeout=30) as client:
        # Create
        create = await client.post(
            f"{ironclaw_server}/api/engine/projects",
            headers=_auth_headers(),
            json={
                "name": "coding-flow-test",
                "description": "E2E coding UX scenario",
                "workspace_path": str(workspace_path),
                "github_repo": "nearai/ironclaw",
                "default_branch": "staging",
            },
        )
        assert create.status_code == 200, (
            f"project_create failed: {create.status_code} {create.text}"
        )
        project_id = create.json()["project"]["id"]

        # Make it the active project so new threads inherit it.
        activate = await client.post(
            f"{ironclaw_server}/api/engine/projects/active",
            headers=_auth_headers(),
            json={"project_id": project_id},
        )
        assert activate.status_code == 200, (
            f"project_set_active failed: {activate.status_code} {activate.text}"
        )
    return project_id


async def test_project_chrome_renders_after_create(page, ironclaw_server):
    """Chrome shows project name + folder after create + reload.

    This is the first indicator the user sees that a thread is bound
    to a project; regressing it would leave the chrome bar hidden and
    the shell-mode dispatch unreachable from the UI.
    """
    with tempfile.TemporaryDirectory(prefix="ironclaw-e2e-coding-") as tmp:
        project_path = Path(tmp) / "project-a"
        await _create_active_project(ironclaw_server, project_path)

        # Reload so the ProjectUI IIFE fetches the newly-created project
        # and renders the chrome bar.
        await page.reload(wait_until="domcontentloaded")
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)

        chrome = page.locator("#project-chrome")
        await chrome.wait_for(state="visible", timeout=10000)
        name = await page.locator("#project-chrome-name").inner_text()
        assert "coding-flow-test" in name, f"project name missing from chrome, got: {name!r}"

        folder = await page.locator("#project-chrome-folder").inner_text()
        assert str(project_path) in folder or "~/" in folder, (
            f"project folder missing from chrome, got: {folder!r}"
        )


async def test_shell_mode_badge_toggles_on_bang_prefix(page, ironclaw_server):
    """Typing `!` toggles the `shell-mode` class on the input wrapper.

    The badge is rendered via CSS (`::before` on `.shell-mode`) and
    driven by a JS `input` listener installed by `ProjectUI`. A
    regression in either half would silently leave users typing
    regular text into shell mode, or (worse) sending their shell
    command to the LLM.
    """
    # Fresh navigation — no project needed for this badge-level check.
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    wrapper_has_shell_class = await page.evaluate(
        """() => document
             .querySelector('.chat-input-wrapper')
             .classList.contains('shell-mode')"""
    )
    assert not wrapper_has_shell_class, "shell-mode class must not be set by default"

    await chat_input.fill("!")
    # Input event fires synchronously; give it a microtask tick to flip
    # the class before asserting.
    await page.wait_for_function(
        """() => document
             .querySelector('.chat-input-wrapper')
             .classList.contains('shell-mode')""",
        timeout=2000,
    )

    await chat_input.fill("")
    await page.wait_for_function(
        """() => !document
             .querySelector('.chat-input-wrapper')
             .classList.contains('shell-mode')""",
        timeout=2000,
    )


async def test_shell_mode_roundtrip_persists_across_reload(page, ironclaw_server):
    """Submit `!echo hello` → render shell card → reload → still there.

    Exercises the end-to-end Stage C flow:
      1. Frontend detects `!` prefix and calls `sendShellCommand`.
      2. POST /api/chat/send with `mode=shell` dispatches the shell
         tool through ToolDispatcher, persists a `shell_command`
         and `shell_output` conversation message pair, and emits
         the matching SSE events.
      3. The shell-turn renderer paints a monospace card with
         exit-code badge.
      4. After reload, `GET /api/chat/history` returns a TurnInfo
         with a populated `shell` field; the history-replay path
         renders the same card.
    """
    with tempfile.TemporaryDirectory(prefix="ironclaw-e2e-coding-") as tmp:
        project_path = Path(tmp) / "project-b"
        await _create_active_project(ironclaw_server, project_path)

        # Reload so the new thread auto-created by the UI inherits
        # the active project — required for shell-mode dispatch to
        # resolve a workdir instead of 409-ing.
        await page.reload(wait_until="domcontentloaded")
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)
        await page.locator("#project-chrome").wait_for(state="visible", timeout=10000)

        # Type `!echo hello` and submit.
        chat_input = page.locator(SEL["chat_input"])
        await chat_input.fill("!echo hello")
        await chat_input.press("Enter")

        # Wait for the shell-turn card. The test uses the most
        # specific selector we can (the whole turn wrapper); asserting
        # on the inner body after the SSE fills it proves the renderer
        # paired shell_command + shell_output correctly.
        shell_turn = page.locator(".shell-turn").first
        await shell_turn.wait_for(state="visible", timeout=15000)

        # Body is hidden until `shell_output` arrives; waiting on the
        # `exit 0` badge is a stricter "fully rendered" signal.
        exit_badge = page.locator(".shell-turn-status.success").first
        await exit_badge.wait_for(state="visible", timeout=15000)
        exit_text = await exit_badge.inner_text()
        assert "exit 0" in exit_text, f"expected exit 0 badge, got: {exit_text!r}"

        body = await page.locator(".shell-turn-body").first.inner_text()
        assert "hello" in body, f"expected 'hello' in shell output body, got: {body!r}"

        # --- Reload + verify history replay keeps the turn ---
        await page.reload(wait_until="domcontentloaded")
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)

        # The history path rebuilds the card from the paired messages.
        # Don't assume the card count — other tests may have run first.
        await page.locator(".shell-turn").first.wait_for(state="visible", timeout=15000)
        replayed_body = await page.locator(".shell-turn-body").first.inner_text()
        assert "hello" in replayed_body, (
            f"shell turn did not survive reload; body was: {replayed_body!r}"
        )


async def test_shell_mode_rejects_when_no_project(page, ironclaw_server):
    """`!` without any active project must 409 rather than dispatching.

    The backend contract: shell mode requires a resolvable project
    (per-thread override → active pointer → error). A regression that
    ran shell commands against the gateway's own cwd would be a
    silent security foot-gun.
    """
    # Explicitly clear any active project the prior tests may have set.
    async with httpx.AsyncClient(timeout=15) as client:
        await client.post(
            f"{ironclaw_server}/api/engine/projects/active",
            headers=_auth_headers(),
            json={"project_id": None},
        )

    await page.reload(wait_until="domcontentloaded")
    await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)

    # Get a valid thread_id the page is currently bound to.
    thread_id = await page.evaluate("() => window.currentThreadId || null")
    assert thread_id, "test requires the UI to be bound to a thread"

    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.post(
            f"{ironclaw_server}/api/chat/send",
            headers=_auth_headers(),
            json={
                "content": "echo hello",
                "thread_id": thread_id,
                "mode": "shell",
            },
        )
        assert r.status_code == 409, (
            f"expected 409 for shell-mode without project, got: {r.status_code} {r.text}"
        )


async def test_dev_project_pills_render(page, ironclaw_server):
    """Chrome renders repo / branch / #issue / PR#n pills from thread.metadata.dev.

    This exercises the fix-issue skill's user-visible surface without
    spinning up a real agent or mutating GitHub — the test drives the
    UI layer directly with a mocked `project` payload matching the
    shape the backend emits after the skill has written `dev.*` keys
    via `thread_metadata_set`.

    The four pills — repo, branch, #issue, PR#n — are the primary
    signal a user has that a coding-agent conversation is progressing
    through the workflow. Regressing any of them would leave the
    conversation chrome stuck on the folder and the user unable to
    tell whether the agent has branched / filed the PR / which issue
    it's working on.
    """
    with tempfile.TemporaryDirectory(prefix="ironclaw-e2e-pills-") as tmp:
        project_path = Path(tmp) / "pills-project"
        await _create_active_project(ironclaw_server, project_path)

        # Reload so ProjectUI mounts and exposes its window surface.
        await page.reload(wait_until="domcontentloaded")
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)
        await page.locator("#project-chrome").wait_for(state="visible", timeout=10000)

        # Drive `refreshChromeFromThread` with the full `ThreadProjectContext`
        # shape the backend emits for a project the agent has worked on —
        # github_repo set, branch written by coding-repo, issue written by
        # fix-issue's step 1, PR written by fix-issue's step 7. The shape
        # mirrors `src/channels/web/types.rs::ThreadProjectContext` exactly.
        await page.evaluate(
            """(projectPath) => {
                window.ProjectUI.refreshChromeFromThread({
                    id: "11111111-2222-3333-4444-555555555555",
                    name: "pills-project",
                    workspace_path: projectPath,
                    github_repo: "nearai/ironclaw-e2e-test",
                    default_branch: "staging",
                    branch: "ip/fix-42-widget-pills",
                    dirty: false,
                    pr: {
                        number: 17,
                        title: "Fix widget pill rendering (#42)",
                        url: "https://github.com/nearai/ironclaw-e2e-test/pull/17",
                        state: "open"
                    },
                    issue: {
                        number: 42,
                        title: "Widget pills don't update on thread_metadata_set",
                        url: "https://github.com/nearai/ironclaw-e2e-test/issues/42"
                    },
                    is_override: false
                });
            }""",
            str(project_path),
        )

        # ── repo pill ───────────────────────────────────────────
        repo = page.locator("#project-chrome-repo")
        await repo.wait_for(state="visible", timeout=5000)
        repo_text = await repo.inner_text()
        assert repo_text == "nearai/ironclaw-e2e-test", (
            f"repo pill text wrong: {repo_text!r}"
        )
        repo_href = await repo.get_attribute("href")
        assert repo_href == "https://github.com/nearai/ironclaw-e2e-test", (
            f"repo pill href wrong: {repo_href!r}"
        )

        # ── branch pill ─────────────────────────────────────────
        branch = page.locator("#project-chrome-branch")
        await branch.wait_for(state="visible", timeout=5000)
        branch_text = await branch.inner_text()
        assert branch_text == "ip/fix-42-widget-pills", (
            f"branch pill text wrong: {branch_text!r}"
        )

        # ── issue pill ──────────────────────────────────────────
        issue = page.locator("#project-chrome-issue")
        await issue.wait_for(state="visible", timeout=5000)
        issue_text = await issue.inner_text()
        assert issue_text == "#42", f"issue pill text wrong: {issue_text!r}"
        issue_href = await issue.get_attribute("href")
        assert issue_href == "https://github.com/nearai/ironclaw-e2e-test/issues/42", (
            f"issue pill href wrong: {issue_href!r}"
        )
        issue_title = await issue.get_attribute("title")
        assert issue_title == "Widget pills don't update on thread_metadata_set", (
            f"issue pill hover title wrong: {issue_title!r}"
        )

        # ── PR pill ─────────────────────────────────────────────
        pr = page.locator("#project-chrome-pr")
        await pr.wait_for(state="visible", timeout=5000)
        pr_text = await pr.inner_text()
        # The renderer joins "PR #{n} {state}" so a regression that drops
        # the state label is caught here, not swallowed into a passing
        # partial match.
        assert "PR #17" in pr_text and "open" in pr_text, (
            f"PR pill text wrong: {pr_text!r}"
        )
        pr_href = await pr.get_attribute("href")
        assert pr_href == "https://github.com/nearai/ironclaw-e2e-test/pull/17", (
            f"PR pill href wrong: {pr_href!r}"
        )


async def test_dev_project_pills_hide_when_fields_absent(page, ironclaw_server):
    """Pills hide cleanly when the underlying `dev.*` keys aren't set.

    Ensures the chrome doesn't paint a stale pill after the thread
    transitions out of a dev-workflow (agent switches to a different
    project, or clears `thread.metadata.dev`). The missing-field branch
    is tested separately because the happy-path assertion above passes
    even if the hide-on-null logic is wrong — a stale pill would just
    never clear.
    """
    with tempfile.TemporaryDirectory(prefix="ironclaw-e2e-pills-") as tmp:
        project_path = Path(tmp) / "no-pills-project"
        await _create_active_project(ironclaw_server, project_path)

        await page.reload(wait_until="domcontentloaded")
        await page.locator(SEL["chat_input"]).wait_for(state="visible", timeout=10000)
        await page.locator("#project-chrome").wait_for(state="visible", timeout=10000)

        # Payload without github_repo / issue / pr — only the basics.
        await page.evaluate(
            """(projectPath) => {
                window.ProjectUI.refreshChromeFromThread({
                    id: "11111111-2222-3333-4444-555555555555",
                    name: "no-pills-project",
                    workspace_path: projectPath,
                    default_branch: "main",
                    is_override: false
                });
            }""",
            str(project_path),
        )

        # All four optional pills must be hidden.
        for pill_id in (
            "#project-chrome-repo",
            "#project-chrome-branch",
            "#project-chrome-issue",
            "#project-chrome-pr",
        ):
            hidden = await page.evaluate(
                f"() => document.querySelector('{pill_id}').hidden"
            )
            assert hidden, f"expected {pill_id} to be hidden with no dev.* fields"


async def test_github_repo_validation_rejects_invalid_slug(ironclaw_server):
    """`POST /api/engine/projects` rejects malformed github_repo values.

    Drives the `GitHubRepo::new` validation through the tool dispatcher
    to make sure the type newtype is enforced at the HTTP boundary —
    not at some later render step where a bad value would corrupt
    chrome rendering until manually reset.
    """
    async with httpx.AsyncClient(timeout=15) as client:
        r = await client.post(
            f"{ironclaw_server}/api/engine/projects",
            headers=_auth_headers(),
            json={
                "name": "invalid-gh",
                "description": "should reject",
                "github_repo": "not a slug",
            },
        )
        # The tool surfaces `InvalidParameters`, which maps to 400 via
        # the gateway's `map_dispatch_err`.
        assert r.status_code == 400, (
            f"expected 400 for invalid github_repo, got: {r.status_code} {r.text}"
        )
