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
