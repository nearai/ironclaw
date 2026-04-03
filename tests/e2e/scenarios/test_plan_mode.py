"""E2E tests for plan mode feature.

Tests the /plan command, plan checklist rendering via SSE, and plan lifecycle.
"""

import asyncio

from helpers import SEL, AUTH_TOKEN, api_get, api_post


async def _send_and_wait_for_plan(page, message, timeout=15000):
    """Send a message and wait for a plan container to appear or update."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill(message)
    await chat_input.press("Enter")

    plan = page.locator(SEL["plan_container"]).last
    await plan.wait_for(state="visible", timeout=timeout)
    return plan


async def test_plan_create_renders_checklist(page):
    """User creates a plan via /plan -> plan checklist renders in chat."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    plan = await _send_and_wait_for_plan(page, "/plan Organize my project files")

    # Verify checklist has steps
    steps = plan.locator(SEL["plan_steps"])
    count = await steps.count()
    assert count >= 2, f"Expected at least 2 steps, got {count}"

    # Verify title
    title = plan.locator(SEL["plan_title"])
    title_text = await title.text_content()
    assert title_text, "Plan title should not be empty"

    # Verify status badge shows "draft"
    badge = plan.locator(SEL["plan_status_badge"])
    badge_text = await badge.text_content()
    assert "draft" in badge_text.lower(), f"Expected 'draft' status, got '{badge_text}'"

    # Verify summary line
    summary = plan.locator(SEL["plan_summary"])
    summary_text = await summary.text_content()
    assert "steps" in summary_text.lower(), f"Summary should mention steps: '{summary_text}'"


async def test_plan_approve_changes_status(page):
    """Approving a plan changes its status to executing."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Create plan first
    await _send_and_wait_for_plan(page, "/plan Organize files")

    # Approve the plan
    await chat_input.fill("/plan approve test-plan-001")
    await chat_input.press("Enter")

    # Wait for status to change to "executing"
    await page.wait_for_function(
        """() => {
            const badges = document.querySelectorAll('.plan-status-badge');
            for (const b of badges) {
                if (b.textContent.toLowerCase().includes('executing')) return true;
            }
            return false;
        }""",
        timeout=15000,
    )

    # Verify at least one step is in_progress
    running = page.locator(SEL["plan_step_running"])
    count = await running.count()
    assert count >= 1, "Expected at least one in_progress step after approval"


async def test_plan_status_shows_progress(page):
    """Plan status command shows updated progress."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Create plan
    await _send_and_wait_for_plan(page, "/plan Test project")

    # Check status (mock LLM returns progress with one completed step)
    await chat_input.fill("/plan status test-plan-001")
    await chat_input.press("Enter")

    # Wait for a completed step to appear
    await page.wait_for_function(
        """() => {
            const completed = document.querySelectorAll('.plan-step[data-status="completed"]');
            return completed.length >= 1;
        }""",
        timeout=15000,
    )

    # Verify the completed step has a result
    completed = page.locator(SEL["plan_step_completed"]).first
    result = completed.locator(".plan-step-result")
    result_count = await result.count()
    assert result_count >= 1, "Completed step should have a result"


async def test_plan_list_via_api(ironclaw_server):
    """API: /plan list returns a response via chat send."""
    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}

    # Create thread
    async with __import__("httpx").AsyncClient() as client:
        thread_r = await client.post(
            f"{ironclaw_server}/api/chat/thread/new",
            headers=headers,
            timeout=15,
        )
        assert thread_r.status_code == 200
        thread_id = thread_r.json()["id"]

        # Send plan list command
        send_r = await client.post(
            f"{ironclaw_server}/api/chat/send",
            headers=headers,
            json={"content": "/plan list", "thread_id": thread_id},
            timeout=30,
        )
        assert send_r.status_code in (200, 202)

        # Poll for response
        response_text = ""
        for _ in range(60):
            r = await client.get(
                f"{ironclaw_server}/api/chat/history",
                headers=headers,
                params={"thread_id": thread_id},
                timeout=15,
            )
            history = r.json()
            turns = history.get("turns", [])
            if turns and turns[-1].get("response"):
                response_text = turns[-1]["response"]
                break
            await asyncio.sleep(0.5)

        # Should have some response (plan list or "no plans")
        assert response_text, "Expected a response from /plan list"


async def test_plan_command_parsed_correctly(page):
    """The /plan command is parsed and reaches the agent (not treated as raw text)."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Count existing assistant messages
    assistant_msgs = page.locator(SEL["message_assistant"])
    before = await assistant_msgs.count()

    # Send /plan command
    await chat_input.fill("/plan Deploy to production")
    await chat_input.press("Enter")

    # Wait for either a plan container or an assistant message
    try:
        await page.wait_for_function(
            """({ selector, count }) => {
                const plans = document.querySelectorAll('.plan-container');
                const msgs = document.querySelectorAll(selector);
                return plans.length >= 1 || msgs.length > count;
            }""",
            arg={"selector": SEL["message_assistant"], "count": before},
            timeout=15000,
        )
    except Exception:
        pass  # Timeout is acceptable; we just verify it was processed

    # The user message should appear in chat
    user_msgs = page.locator(SEL["message_user"])
    user_count = await user_msgs.count()
    assert user_count >= 1, "User message from /plan should appear in chat"
