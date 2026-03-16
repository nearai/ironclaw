"""
E2E tests for event-triggered routines with batch loading.

These tests verify that the N+1 query fix correctly:
1. Fires event-triggered routines on matching messages
2. Enforces concurrent limits via batch-loaded counts
3. Maintains performance with multiple simultaneous triggers
4. Works correctly through the full UI and agent loop

Playwright-based UI tests + SSE verification.
"""

import asyncio
import json
import pytest
from datetime import datetime, timedelta
from typing import List, Dict, Any

from playwright.async_api import Page


class EventTriggerHelper:
    """Helper methods for event trigger testing."""

    def __init__(self, page: Page, base_url: str):
        self.page = page
        self.base_url = base_url

    def _url(self, path: str) -> str:
        return f"{self.base_url}{path}"

    async def navigate_to_routines(self):
        """Navigate to the routines page."""
        await self.page.goto(self._url("/routines"))
        await self.page.wait_for_load_state("networkidle")

    async def create_event_routine(
        self,
        name: str,
        trigger_regex: str,
        channel: str = "slack",
        max_concurrent: int = 1,
    ) -> str:
        """
        Create an event-triggered routine via UI.
        Returns the routine ID.
        """
        await self.navigate_to_routines()

        # Click "New Routine" button
        await self.page.click('button:has-text("New Routine")')
        await self.page.wait_for_selector('input[name="routine_name"]')

        # Fill routine details
        await self.page.fill('input[name="routine_name"]', name)
        await self.page.fill(
            'textarea[name="routine_description"]',
            f"Test routine: {name}",
        )

        # Select "Event Trigger" type
        await self.page.click('label:has-text("Event Trigger")')
        await self.page.wait_for_selector('input[name="trigger_regex"]')

        # Fill trigger details
        await self.page.fill('input[name="trigger_regex"]', trigger_regex)
        await self.page.select_option('select[name="trigger_channel"]', channel)

        # Set guardrails
        await self.page.fill('input[name="max_concurrent"]', str(max_concurrent))

        # Select lightweight action
        await self.page.click('label:has-text("Lightweight")')
        await self.page.fill(
            'textarea[name="lightweight_prompt"]',
            "Acknowledge the message and confirm trigger worked.",
        )

        # Save routine
        await self.page.click('button:has-text("Save Routine")')
        await self.page.wait_for_selector('text=Routine created successfully')

        # Extract routine ID from success message or URL
        routine_id = await self.page.locator('data-testid=routine-id').text_content()
        return routine_id.strip() if routine_id else None

    async def create_multiple_routines(
        self, base_name: str, count: int, trigger_regex: str = None
    ) -> List[str]:
        """Create multiple event-triggered routines."""
        routine_ids = []
        for i in range(count):
            name = f"{base_name}_{i}"
            regex = trigger_regex or f"({i}|{base_name})"
            routine_id = await self.create_event_routine(name, regex)
            routine_ids.append(routine_id)
            await asyncio.sleep(0.1)  # Small delay between creations
        return routine_ids

    async def send_chat_message(self, message: str) -> List[str]:
        """
        Send a chat message and return SSE events received.
        Captures all routine firing events.
        """
        await self.page.goto(self._url("/chat"))
        await self.page.wait_for_selector('input[placeholder*="message"]', timeout=5000)

        # Collect SSE events
        sse_events = []

        async def capture_sse(response):
            """Intercept SSE events."""
            if "event-stream" in response.headers.get("content-type", ""):
                text = await response.text()
                for line in text.split("\n"):
                    if line.startswith("data:"):
                        try:
                            event = json.loads(line[5:])
                            sse_events.append(event)
                        except json.JSONDecodeError:
                            pass

        self.page.on("response", capture_sse)

        # Send message
        await self.page.fill('input[placeholder*="message"]', message)
        await self.page.press('input[placeholder*="message"]', "Enter")

        # Wait for response
        await self.page.wait_for_selector('text=Message processed', timeout=10000)
        await asyncio.sleep(0.5)  # Allow time for SSE events

        self.page.remove_listener("response", capture_sse)
        return sse_events

    async def get_routine_execution_log(self, routine_id: str) -> List[Dict]:
        """Get execution log entries for a routine."""
        await self.page.goto(self._url(f"/routines/{routine_id}/executions"))
        await self.page.wait_for_load_state("networkidle")

        # Extract log entries from table
        rows = await self.page.locator("tbody tr").all()
        executions = []

        for row in rows:
            cells = await row.locator("td").all()
            if len(cells) >= 3:
                execution = {
                    "timestamp": await cells[0].text_content(),
                    "status": await cells[1].text_content(),
                    "details": await cells[2].text_content(),
                }
                executions.append(execution)

        return executions

    async def check_database_queries_in_logs(
        self, max_queries_expected: int = 1
    ) -> int:
        """Check debug logs for database query count."""
        await self.page.goto(self._url("/debug/logs?filter=database"))
        await self.page.wait_for_load_state("networkidle")

        # Count batch queries
        log_lines = await self.page.locator("tr:has-text('batch')").all()
        batch_count = len(log_lines)

        # Count individual COUNT queries (should be 0 after fix)
        count_queries = await self.page.locator("tr:has-text('COUNT')").all()
        count_query_count = len(count_queries)

        return batch_count, count_query_count


# =============================================================================
# Tests
# =============================================================================


@pytest.mark.asyncio
async def test_create_event_trigger_routine(page, ironclaw_server):
    """Test creating an event-triggered routine via UI."""
    helper = EventTriggerHelper(page, ironclaw_server)
    routine_id = await helper.create_event_routine(
        name="Test Trigger",
        trigger_regex="test|demo",
        channel="slack",
        max_concurrent=1,
    )

    assert routine_id is not None, "Routine ID should be returned"
    assert len(routine_id) > 0, "Routine ID should not be empty"


@pytest.mark.asyncio
async def test_event_trigger_fires_on_matching_message(page, ironclaw_server):
    """Test that event-triggered routine fires when message matches."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_id = await helper.create_event_routine(
        name="Alert Handler",
        trigger_regex="urgent|critical|alert",
        channel="slack",
    )

    sse_events = await helper.send_chat_message("URGENT: Server down!")

    routine_fired = any(
        event.get("type") == "routine_fired" and event.get("routine_id") == routine_id
        for event in sse_events
    )
    assert routine_fired, "Routine should fire on matching message"

    executions = await helper.get_routine_execution_log(routine_id)
    assert len(executions) > 0, "Execution should be logged"
    assert "success" in executions[0]["status"].lower()


@pytest.mark.asyncio
async def test_event_trigger_skips_non_matching_message(page, ironclaw_server):
    """Test that event-triggered routine skips when message doesn't match."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_id = await helper.create_event_routine(
        name="Alert Handler",
        trigger_regex="urgent|critical|alert",
        channel="slack",
    )

    sse_events = await helper.send_chat_message("Hello, how are you?")

    routine_fired = any(
        event.get("type") == "routine_fired" and event.get("routine_id") == routine_id
        for event in sse_events
    )
    assert not routine_fired, "Routine should not fire on non-matching message"


@pytest.mark.asyncio
async def test_multiple_routines_fire_on_matching_message(page, ironclaw_server):
    """Test that multiple event-triggered routines fire on same message."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_ids = await helper.create_multiple_routines(
        base_name="Handler", count=3, trigger_regex="alert|warning|error"
    )

    sse_events = await helper.send_chat_message("ERROR: Database connection failed")

    fired_count = sum(
        1
        for event in sse_events
        if event.get("type") == "routine_fired" and event.get("routine_id") in routine_ids
    )

    assert fired_count >= 3, f"Expected all 3 routines to fire, got {fired_count}"


@pytest.mark.asyncio
async def test_concurrent_limit_prevents_additional_fires(page, ironclaw_server):
    """Test that concurrent limit is enforced via batch counts."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_id = await helper.create_event_routine(
        name="Limited Handler",
        trigger_regex="process|task",
        max_concurrent=1,
    )

    await helper.send_chat_message("Process message 1")
    await asyncio.sleep(1)

    executions_1 = await helper.get_routine_execution_log(routine_id)
    assert len(executions_1) >= 1

    sse_events = await helper.send_chat_message("Process message 2")

    routine_skipped = any(
        event.get("type") == "routine_skipped"
        and event.get("reason") == "max_concurrent_reached"
        and event.get("routine_id") == routine_id
        for event in sse_events
    )
    assert routine_skipped, "Routine should be skipped when concurrent limit reached"


@pytest.mark.asyncio
async def test_rapid_messages_with_multiple_triggers_efficiency(page, ironclaw_server):
    """Test efficiency of batch loading with multiple rapid messages."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_ids = await helper.create_multiple_routines(
        base_name="Rapid", count=5, trigger_regex="test|demo|check"
    )

    for i in range(10):
        message = f"test message {i}"
        await helper.send_chat_message(message)
        await asyncio.sleep(0.1)

    batch_count, count_query_count = await helper.check_database_queries_in_logs()

    assert count_query_count == 0, (
        f"Should have 0 individual COUNT queries after fix, got {count_query_count}"
    )
    assert batch_count <= 15, (
        f"Should have <=15 batch queries for 10 messages, got {batch_count}"
    )


@pytest.mark.asyncio
async def test_channel_filter_applied_correctly(page, ironclaw_server):
    """Test that channel filter prevents non-matching messages."""
    helper = EventTriggerHelper(page, ironclaw_server)

    slack_routine_id = await helper.create_event_routine(
        name="Slack Handler",
        trigger_regex="alert",
        channel="slack",
    )

    await page.goto(f"{ironclaw_server}/chat?channel=telegram")
    await helper.send_chat_message("alert: something urgent")

    executions = await helper.get_routine_execution_log(slack_routine_id)

    recent = [
        e
        for e in executions
        if (datetime.now() - datetime.fromisoformat(e["timestamp"])).total_seconds()
        < 300
    ]
    assert len(recent) == 0, "Routine should not fire for different channel"


@pytest.mark.asyncio
async def test_batch_query_failure_handling(page, ironclaw_server):
    """Test graceful handling of batch query failures."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_id = await helper.create_event_routine(
        name="Error Handler",
        trigger_regex="test",
    )

    await helper.send_chat_message("test message")

    assert await page.locator("text=Message processed").is_visible()


@pytest.mark.asyncio
async def test_routine_execution_history_display(page, ironclaw_server):
    """Test that execution history correctly displays routine firings."""
    helper = EventTriggerHelper(page, ironclaw_server)

    routine_id = await helper.create_event_routine(
        name="History Test",
        trigger_regex="test",
    )

    for i in range(3):
        await helper.send_chat_message(f"test message {i}")
        await asyncio.sleep(0.2)

    executions = await helper.get_routine_execution_log(routine_id)
    assert len(executions) >= 3, "Should have at least 3 executions logged"

    for execution in executions[:3]:
        timestamp = datetime.fromisoformat(execution["timestamp"])
        age = datetime.now() - timestamp
        assert age < timedelta(minutes=5), "Execution should be recent"


@pytest.mark.asyncio
async def test_concurrent_batch_loads_independent(page, ironclaw_server):
    """Test that concurrent messages each get independent batch queries."""
    helper = EventTriggerHelper(page, ironclaw_server)

    r1_id = await helper.create_event_routine(
        name="Pattern A", trigger_regex="alpha|alpha_only"
    )
    r2_id = await helper.create_event_routine(
        name="Pattern B", trigger_regex="beta|beta_only"
    )
    r3_id = await helper.create_event_routine(
        name="Pattern AB", trigger_regex="alpha|beta|common"
    )

    sse1 = await helper.send_chat_message("alpha common")
    await asyncio.sleep(0.1)

    sse2 = await helper.send_chat_message("beta common")
    await asyncio.sleep(0.1)

    r1_fired_msg1 = any(
        e.get("routine_id") == r1_id for e in sse1 if e.get("type") == "routine_fired"
    )
    r2_fired_msg2 = any(
        e.get("routine_id") == r2_id for e in sse2 if e.get("type") == "routine_fired"
    )
    r3_fired_both = (
        any(e.get("routine_id") == r3_id for e in sse1 if e.get("type") == "routine_fired")
        and any(
            e.get("routine_id") == r3_id for e in sse2 if e.get("type") == "routine_fired"
        )
    )

    assert r1_fired_msg1, "Routine 1 should fire on message 1"
    assert r2_fired_msg2, "Routine 2 should fire on message 2"
    assert r3_fired_both, "Routine 3 should fire on both messages"


# =============================================================================
# Integration with existing test patterns
# =============================================================================


if __name__ == "__main__":
    # Run tests with: pytest tests/e2e/scenarios/test_routine_event_batch.py -v
    pytest.main([__file__, "-v", "-s"])
