"""Full-path Reborn extension tests backed by Emulate provider state."""

import uuid

import httpx

from emulate_provider import google_headers
from reborn_emulate_harness import (
    complete_oauth_setup,
    completed_tool_results,
    extension_names,
    get_extension,
    install_extension,
    new_thread,
    send_chat,
    tool_result_json,
    wait_for_response_containing,
)


async def test_reborn_google_calendar_lifecycle_mutates_emulate(
    hosted_google_emulate_server,
):
    server = hosted_google_emulate_server["base_url"]
    emulate_google_url = hosted_google_emulate_server["emulate_google_url"]

    await install_extension(server, "google-calendar")
    await complete_oauth_setup(
        server,
        "google-calendar",
        code="mock_calendar_full_path_code",
    )

    calendar = await get_extension(server, "google-calendar")
    assert calendar is not None, (
        "google-calendar should be installed; "
        f"available extensions: {await extension_names(server)}"
    )
    assert calendar["authenticated"] is True, calendar
    assert "google_calendar" in calendar.get("tools", []), calendar

    title = f"[canary] reborn-emulate-calendar-{uuid.uuid4().hex[:8]}"
    thread_id = await new_thread(server)
    await send_chat(
        server,
        thread_id,
        f"create a google calendar event titled '{title}'",
    )

    history = await wait_for_response_containing(
        server,
        thread_id,
        "google_calendar lifecycle complete",
        timeout=90,
    )
    tool_results = completed_tool_results(history, "google_calendar")
    assert len(tool_results) >= 3, history

    created = tool_result_json(tool_results[0])
    event = created.get("event", created)
    event_id = event["id"]
    assert event["summary"] == title

    async with httpx.AsyncClient(timeout=10) as client:
        readback = await client.get(
            f"{emulate_google_url}/calendar/v3/calendars/primary/events/{event_id}",
            headers=google_headers(),
        )

    if readback.status_code == 200:
        assert readback.json().get("status") == "cancelled", readback.text
    else:
        assert readback.status_code in (404, 410), readback.text
