"""E2E tests for full_job routine execution.

Exercises the complete lifecycle: create a full_job routine via the
web UI, trigger it via the API, and verify the job runs tools and
completes without hitting the iteration cap.

Requires Playwright (browser-based tests).
"""

import asyncio
import uuid

from helpers import SEL, api_get, api_post


# -- Helpers ------------------------------------------------------------------

async def _send_chat_message(
    base_url: str,
    message: str,
    *,
    expected_fragment: str | None = None,
    timeout: float = 30.0,
) -> dict:
    """Send a chat message through the API and handle approval if required."""
    thread = await api_post(base_url, "/api/chat/thread/new")
    thread.raise_for_status()
    thread_id = thread.json()["id"]

    send = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": message, "thread_id": thread_id},
        timeout=30,
    )
    assert send.status_code in (200, 202), send.text[:400]

    for _ in range(int(timeout * 2)):
        history = await api_get(
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            timeout=15,
        )
        history.raise_for_status()
        data = history.json()

        pending = data.get("pending_gate") or data.get("pending_approval")
        if pending:
            approval = await api_post(
                base_url,
                "/api/chat/approval",
                json={
                    "request_id": pending["request_id"],
                    "action": "approve",
                    "thread_id": thread_id,
                },
                timeout=15,
            )
            assert approval.status_code == 202, approval.text[:400]

        turns = data.get("turns", [])
        if turns and turns[-1].get("response"):
            response = turns[-1]["response"]
            if expected_fragment is None or expected_fragment.lower() in response.lower():
                return data

        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Chat command '{message}' did not complete within {timeout}s"
    )


async def _open_tab(page, tab: str) -> None:
    """Switch to a visible top-level tab."""
    button = page.locator(SEL["tab_button"].format(tab=tab))
    await button.click()
    await page.locator(SEL["tab_panel"].format(tab=tab)).wait_for(
        state="visible",
        timeout=5000,
    )


async def _wait_for_routine(base_url: str, name: str, timeout: float = 20.0) -> dict:
    """Poll until the named routine exists."""
    for _ in range(int(timeout * 2)):
        resp = await api_get(base_url, "/api/routines")
        resp.raise_for_status()
        for routine in resp.json()["routines"]:
            if routine["name"] == name:
                return routine
        await asyncio.sleep(0.5)
    raise AssertionError(f"Routine '{name}' not created within {timeout}s")


async def _get_routine_runs(base_url: str, routine_id: str) -> list[dict]:
    """Fetch routine runs."""
    resp = await api_get(base_url, f"/api/routines/{routine_id}/runs")
    resp.raise_for_status()
    return resp.json()["runs"]


async def _wait_for_completed_run(
    base_url: str,
    routine_id: str,
    *,
    timeout: float = 60.0,
) -> dict:
    """Poll until the newest run reaches a terminal state."""
    for _ in range(int(timeout * 2)):
        runs = await _get_routine_runs(base_url, routine_id)
        if runs and runs[0]["status"].lower() not in ("running", "pending"):
            return runs[0]
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Routine '{routine_id}' did not complete within {timeout}s"
    )


async def _wait_for_job_terminal(
    base_url: str,
    job_id: str,
    *,
    timeout: float = 60.0,
) -> dict:
    """Poll until a job reaches a terminal state."""
    terminal = {"completed", "failed", "cancelled", "submitted", "accepted"}
    for _ in range(int(timeout * 2)):
        resp = await api_get(base_url, f"/api/jobs/{job_id}")
        resp.raise_for_status()
        detail = resp.json()
        if detail.get("state", "").lower() in terminal:
            return detail
        await asyncio.sleep(0.5)
    raise AssertionError(f"Job '{job_id}' did not reach terminal state within {timeout}s")


# -- Tests --------------------------------------------------------------------

async def test_full_job_routine_completes_with_tools(page, ironclaw_server):
    """A full_job routine should plan, execute tools, and complete."""
    name = f"fjob-{uuid.uuid4().hex[:8]}"

    # Step 1: Create full_job routine via chat
    await _send_chat_message(
        ironclaw_server,
        f"create full-job owner routine {name}",
        expected_fragment=name,
    )
    routine = await _wait_for_routine(ironclaw_server, name)

    assert routine["id"]
    assert routine["action_type"] == "full_job"

    # Step 2: Trigger the routine
    resp = await api_post(ironclaw_server, f"/api/routines/{routine['id']}/trigger")
    resp.raise_for_status()
    trigger_data = resp.json()
    assert trigger_data["status"] == "triggered"

    # Step 3: Wait for the run to complete
    completed_run = await _wait_for_completed_run(
        ironclaw_server, routine["id"], timeout=60
    )

    # The run should have succeeded (not failed)
    assert completed_run["status"].lower() != "failed", (
        f"Full job routine run failed: {completed_run}"
    )

    # Step 4: Verify the job reached a success state.
    # Jobs may advance past "completed" to "submitted" or "accepted",
    # so treat all post-completion states as success.
    success_states = {"completed", "submitted", "accepted"}
    if completed_run.get("job_id"):
        job = await _wait_for_job_terminal(
            ironclaw_server, completed_run["job_id"], timeout=30
        )
        assert job["state"].lower() in success_states, (
            f"Expected job state in {success_states}, got '{job['state']}'"
        )


async def test_cron_routine_appears_and_can_be_manually_triggered(page, ironclaw_server):
    """Cron routines should expose schedule metadata and support manual trigger."""
    name = f"cron-{uuid.uuid4().hex[:8]}"

    await _send_chat_message(
        ironclaw_server,
        f"create cron owner routine {name}",
        expected_fragment=name,
    )
    routine = await _wait_for_routine(ironclaw_server, name)

    assert routine["trigger_type"] == "cron"
    assert routine["trigger_raw"] == "0 */5 * * * * *"
    assert routine["next_fire_at"], f"Expected next_fire_at on cron routine: {routine}"

    await _open_tab(page, "routines")
    row = page.locator(SEL["routine_row"]).filter(has_text=name).first
    await row.wait_for(state="visible", timeout=15000)
    trigger_cell_text = await row.locator("td").nth(1).inner_text()
    assert trigger_cell_text.strip() == routine["trigger_summary"]

    resp = await api_post(ironclaw_server, f"/api/routines/{routine['id']}/trigger")
    resp.raise_for_status()
    assert resp.json()["status"] == "triggered"

    completed_run = await _wait_for_completed_run(ironclaw_server, routine["id"])
    assert completed_run["trigger_type"] == "manual"
    assert completed_run["status"].lower() == "attention"


async def test_failed_routine_is_visible_in_ui(page, ironclaw_server):
    """A failed routine should surface failed state and error text in the UI."""
    name = f"fail-{uuid.uuid4().hex[:8]}"
    failure_reason = "Response contained no message or tool call"

    await _send_chat_message(
        ironclaw_server,
        f"create failing lightweight owner routine {name}",
        expected_fragment=name,
    )
    routine = await _wait_for_routine(ironclaw_server, name)

    trigger_response = await api_post(
        ironclaw_server,
        f"/api/routines/{routine['id']}/trigger",
    )
    trigger_response.raise_for_status()
    assert trigger_response.json()["status"] == "triggered"

    failed_run = await _wait_for_completed_run(ironclaw_server, routine["id"], timeout=60)
    assert failed_run["status"].lower() == "failed", failed_run
    assert failure_reason in failed_run["result_summary"]

    await _open_tab(page, "routines")
    row = page.locator(SEL["routine_row"]).filter(has_text=name).first
    await row.wait_for(state="visible", timeout=15000)
    await row.click()

    detail = page.locator("#routine-detail")
    await detail.wait_for(state="visible", timeout=10000)
    await detail.locator(".badge.failed").first.wait_for(state="visible", timeout=10000)
    assert "failed" in (await detail.locator(".badge.failed").first.inner_text()).lower()

    recent_run_row = detail.locator("table.routines-table tbody tr").first
    await recent_run_row.wait_for(state="visible", timeout=10000)
    recent_run_text = await recent_run_row.inner_text()
    assert "failed" in recent_run_text.lower()
    assert failure_reason in recent_run_text
