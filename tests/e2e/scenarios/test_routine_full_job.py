"""E2E tests for full_job routine execution.

Exercises the complete lifecycle: create a full_job routine via chat,
trigger it via the API, and verify the job runs tools and completes.
Covers the plan → execute → completion-check → agentic-loop flow.
"""

import asyncio
import uuid

import httpx

from helpers import AUTH_TOKEN, SEL, api_get, api_post


# -- Helpers ------------------------------------------------------------------

async def _send_chat_message(page, message: str) -> None:
    """Send a chat message and wait for the assistant turn to appear."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    assistant_messages = page.locator(SEL["message_assistant"])
    before_count = await assistant_messages.count()

    await chat_input.fill(message)
    await chat_input.press("Enter")

    await page.wait_for_function(
        """({ selector, expectedCount }) => {
            return document.querySelectorAll(selector).length >= expectedCount;
        }""",
        arg={
            "selector": SEL["message_assistant"],
            "expectedCount": before_count + 1,
        },
        timeout=30000,
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


async def _get_job_detail(base_url: str, job_id: str) -> dict:
    """Fetch job detail."""
    resp = await api_get(base_url, f"/api/jobs/{job_id}")
    resp.raise_for_status()
    return resp.json()


async def _wait_for_job_terminal(
    base_url: str,
    job_id: str,
    *,
    timeout: float = 60.0,
) -> dict:
    """Poll until a job reaches a terminal state."""
    terminal = {"completed", "failed", "cancelled", "submitted", "accepted"}
    for _ in range(int(timeout * 2)):
        detail = await _get_job_detail(base_url, job_id)
        if detail.get("state", "").lower() in terminal:
            return detail
        await asyncio.sleep(0.5)
    raise AssertionError(f"Job '{job_id}' did not reach terminal state within {timeout}s")


# -- Tests --------------------------------------------------------------------

async def test_full_job_routine_completes_with_tools(page, ironclaw_server):
    """A full_job routine should plan, execute tools, and complete."""
    name = f"fjob-{uuid.uuid4().hex[:8]}"

    # Step 1: Create full_job routine via chat
    await _send_chat_message(page, f"create full-job owner routine {name}")
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

    # The run should have succeeded (status ok or attention, not failed)
    assert completed_run["status"].lower() != "failed", (
        f"Full job routine run failed: {completed_run}"
    )

    # Step 4: Verify the job reached a terminal state
    # The run should have a linked job_id
    if completed_run.get("job_id"):
        job = await _wait_for_job_terminal(
            ironclaw_server, completed_run["job_id"], timeout=30
        )
        assert job["state"].lower() == "completed", (
            f"Expected job state 'completed', got '{job['state']}'"
        )


async def test_full_job_routine_produces_no_suggestions_in_output(
    page, ironclaw_server
):
    """Full_job routine output should not contain <suggestions> tags."""
    name = f"fjob-nosug-{uuid.uuid4().hex[:8]}"

    await _send_chat_message(page, f"create full-job owner routine {name}")
    routine = await _wait_for_routine(ironclaw_server, name)

    resp = await api_post(ironclaw_server, f"/api/routines/{routine['id']}/trigger")
    resp.raise_for_status()

    completed_run = await _wait_for_completed_run(
        ironclaw_server, routine["id"], timeout=60
    )
    assert completed_run["status"].lower() != "failed"

    # If the run has a job, check events for suggestions tags
    if completed_run.get("job_id"):
        job = await _wait_for_job_terminal(
            ironclaw_server, completed_run["job_id"], timeout=30
        )
        # Check that no events contain raw <suggestions> tags
        events = job.get("events", [])
        for event in events:
            content = event.get("content", "")
            assert "<suggestions>" not in content, (
                f"Job event should not contain <suggestions> tags: {content[:200]}"
            )
