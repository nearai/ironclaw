"""True black-box smoke suite for `ironclaw-reborn serve`.

The in-process Reborn integration harness (`tests/integration/`) is the
coverage workhorse for Reborn behavior, but it cannot prove three things: real
process startup, real HTTP end-to-end, and process-death durability (its
`new_at_path()` reopen approximates a restart; it cannot cover kill -9 /
partial-write / lock-file behavior). This suite is the thin black-box layer on
top of the real `ironclaw-reborn` binary — SMALL and permanent, five
scenarios, not a second workhorse. Resist scope growth here; deeper
functional/UI coverage belongs in `test_reborn_webui_v2_smoke.py` and its
siblings, not this file.

Pure HTTP via `httpx` — no Playwright/browser fixture, since nothing here
needs to render the SPA. All process/config plumbing (config TOML, bearer
env, mock-LLM wiring, start/stop/restart) is reused from
`reborn_webui_harness.py`; see that module for the shared implementation.

Run directly:

    cd tests/e2e && source .venv/bin/activate
    pytest scenarios/test_reborn_blackbox_smoke.py -v

Wired into CI as the `blackbox-smoke` job in `.github/workflows/reborn-e2e.yml`.
"""

import os
import shutil
import subprocess

import httpx

from reborn_webui_harness import (
    create_thread,
    fetch_timeline,
    finalized_assistant_count,
    reborn_bearer_headers,
    reborn_v2_restartable_server,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture
    send_message,
    wait_for_assistant_message,
    wait_for_capability_preview,
)


def _descendant_pids(pid: int) -> list[int]:
    """Snapshot the full subtree of live descendant PIDs under `pid`.

    Must be called while `pid` is still alive: once the parent dies its children
    are reparented (to init or a subreaper), so `pgrep -P <pid>` no longer lists
    them. A post-mortem `pgrep -P <pid>` would therefore always come back empty
    and pass vacuously — the leak check has to capture the tree first, then probe
    each captured PID directly after the kill.
    """
    descendants: list[int] = []
    frontier = [pid]
    while frontier:
        parent = frontier.pop()
        result = subprocess.run(
            ["pgrep", "-P", str(parent)], capture_output=True, text=True
        )
        for token in result.stdout.split():
            child = int(token)
            descendants.append(child)
            frontier.append(child)
    return descendants


def _pid_alive(pid: int) -> bool:
    """True if `pid` still names a live process (signal 0 probe, no signal sent)."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        # Exists but owned by another user — still counts as alive.
        return True
    return True


async def test_reborn_blackbox_boot_health_and_chat_roundtrip(reborn_v2_server):
    """`serve` boots, `/api/health` responds, and a scripted chat turn round-trips."""
    async with httpx.AsyncClient() as anon:
        health = await anon.get(f"{reborn_v2_server}/api/health", timeout=15)
        assert health.status_code == 200, health.text

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_server)
        await send_message(client, reborn_v2_server, thread_id, "what is 2 + 2?")
        reply = await wait_for_assistant_message(client, reborn_v2_server, thread_id)

    assert reply["content"] == "The answer is 4.", reply


async def test_reborn_blackbox_tool_call_turn_executes_and_replies(reborn_v2_yolo_server):
    """A tool-call turn executes the tool and then finalizes a reply.

    Uses the auto-approve `yolo` profile so the turn isn't blocked on an
    approval gate — gate behavior itself is covered by the tool-permissions
    and approval scenario files, not this black-box smoke.
    """
    marker = "blackbox-smoke-echo-8821"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client, reborn_v2_yolo_server, thread_id, f"reborn builtin echo {marker}"
        )
        preview = await wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.echo",
            output_fragment=marker,
        )
        await wait_for_assistant_message(client, reborn_v2_yolo_server, thread_id)

    assert preview["status"] == "completed", preview
    assert marker in (preview.get("output_preview") or ""), preview


async def test_reborn_blackbox_graceful_restart_preserves_thread_history(
    reborn_v2_restartable_server,
):
    """SIGINT -> start: prior thread/turn history survives a graceful restart."""
    state, start_server, stop_server = reborn_v2_restartable_server

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, state["base_url"])
        await send_message(client, state["base_url"], thread_id, "what is 2 + 2?")
        await wait_for_assistant_message(client, state["base_url"], thread_id)

    await stop_server()
    restarted_url = await start_server()

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        timeline = await fetch_timeline(client, restarted_url, thread_id)

    assert finalized_assistant_count(timeline) > 0, timeline


async def test_reborn_blackbox_kill9_durability(reborn_v2_restartable_server):
    """SIGKILL after a completed turn: data survives, server comes back healthy.

    This is the scenario the in-process integration harness cannot cover —
    its restart approximation (`new_at_path()` reopen) never actually kills a
    process. Only a real black-box test can prove the on-disk libsql store
    tolerates an unclean death with no wedge or corruption.
    """
    state, start_server, stop_server = reborn_v2_restartable_server

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, state["base_url"])
        await send_message(client, state["base_url"], thread_id, "what is 2 + 2?")
        await wait_for_assistant_message(client, state["base_url"], thread_id)

    pid = state["proc"].pid
    # Snapshot the child process tree *before* the kill: after the parent dies
    # its children reparent away, so they can only be checked by the PIDs we
    # captured while it was alive (see `_descendant_pids`).
    descendants = _descendant_pids(pid) if shutil.which("pgrep") is not None else []
    await stop_server(hard=True)

    still_alive = [child for child in descendants if _pid_alive(child)]
    assert not still_alive, (
        f"kill -9 of pid {pid} left descendant processes running: {still_alive}"
    )

    restarted_url = await start_server()

    async with httpx.AsyncClient() as anon:
        health = await anon.get(f"{restarted_url}/api/health", timeout=15)
        assert health.status_code == 200, health.text

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        timeline = await fetch_timeline(client, restarted_url, thread_id)

    assert finalized_assistant_count(timeline) > 0, timeline


async def test_reborn_blackbox_auth_boundary(reborn_v2_server):
    """A v2 route rejects a missing bearer with 401 and accepts a valid one with 200.

    Minimal boundary check only — the full bearer/`?token=` shim matrix is
    already covered by `test_reborn_v2_bearer_auth_and_token_shim_scope` in
    `test_reborn_webui_v2_smoke.py`.
    """
    async with httpx.AsyncClient() as anon:
        unauth = await anon.get(f"{reborn_v2_server}/api/webchat/v2/session", timeout=15)
    assert unauth.status_code == 401, unauth.text

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        authed = await client.get(f"{reborn_v2_server}/api/webchat/v2/session", timeout=15)
    assert authed.status_code == 200, authed.text
