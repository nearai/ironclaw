"""Full-path Reborn extension tests backed by Emulate provider state."""

import uuid

import httpx

from emulate_provider import github_json, google_headers
from reborn_emulate_harness import (
    complete_oauth_setup,
    completed_tool_results,
    extension_names,
    get_extension,
    install_extension,
    new_thread,
    patch_extension_validation_endpoint,
    send_chat,
    tool_result_json,
    tool_result_text,
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


async def test_reborn_github_issue_lifecycle_mutates_emulate(
    hosted_github_emulate_server,
):
    server = hosted_github_emulate_server["base_url"]
    mock_llm_url = hosted_github_emulate_server["mock_llm_url"]
    emulate_github_url = hosted_github_emulate_server["emulate_github_url"]
    tools_dir = hosted_github_emulate_server["wasm_tools_dir"]

    await install_extension(server, "github")
    patch_extension_validation_endpoint(
        tools_dir,
        "github",
        f"{emulate_github_url}/user",
    )
    await complete_oauth_setup(
        server,
        "github",
        code="mock_github_full_path_code",
        mock_base_url=mock_llm_url,
    )

    github = await get_extension(server, "github")
    assert github is not None, (
        "github should be installed; "
        f"available extensions: {await extension_names(server)}"
    )
    assert github["authenticated"] is True, github
    assert "github" in github.get("tools", []), github

    title = f"[canary] reborn-emulate-github-{uuid.uuid4().hex[:8]}"
    thread_id = await new_thread(server)
    await send_chat(
        server,
        thread_id,
        f"create a github issue in nearai/ironclaw titled '{title}'",
    )

    history = await wait_for_response_containing(
        server,
        thread_id,
        "github issue lifecycle complete",
        timeout=90,
    )
    tool_results = completed_tool_results(history, "github")
    assert len(tool_results) >= 3, history

    async with httpx.AsyncClient(timeout=10) as client:
        issues = await github_json(
            client,
            emulate_github_url,
            "GET",
            "/repos/nearai/ironclaw/issues",
        )
        issue = next(item for item in issues if item.get("title") == title)
        issue_number = issue["number"]
        readback = await github_json(
            client,
            emulate_github_url,
            "GET",
            f"/repos/nearai/ironclaw/issues/{issue_number}",
        )
        comments = await github_json(
            client,
            emulate_github_url,
            "GET",
            f"/repos/nearai/ironclaw/issues/{issue_number}/comments",
        )

    assert readback["title"] == title
    assert any(comment.get("body") == "Canary verification" for comment in comments)


async def test_reborn_google_drive_lifecycle_mutates_emulate(
    hosted_google_emulate_server,
):
    server = hosted_google_emulate_server["base_url"]
    emulate_google_url = hosted_google_emulate_server["emulate_google_url"]

    await install_extension(server, "google-drive")
    await complete_oauth_setup(
        server,
        "google-drive",
        code="mock_auth_code",
    )

    drive = await get_extension(server, "google-drive")
    assert drive is not None, (
        "google-drive should be installed; "
        f"available extensions: {await extension_names(server)}"
    )
    assert drive["authenticated"] is True, drive
    assert "google_drive" in drive.get("tools", []), drive

    title = f"[canary] reborn-emulate-drive-{uuid.uuid4().hex[:8]}.txt"
    expected_content = f"Canary Google Drive content for {title}"
    thread_id = await new_thread(server)
    await send_chat(
        server,
        thread_id,
        f"upload a google drive file titled '{title}'",
    )

    history = await wait_for_response_containing(
        server,
        thread_id,
        "google_drive lifecycle complete",
        timeout=90,
    )
    tool_results = completed_tool_results(history, "google_drive")
    assert len(tool_results) >= 2, [tool_result_text(result) for result in tool_results]

    uploaded = tool_result_json(tool_results[0])
    file_data = uploaded["file"]
    file_id = file_data["id"]
    assert file_data["name"] == title

    downloaded = tool_result_json(tool_results[1])
    assert downloaded["content"] == expected_content

    async with httpx.AsyncClient(timeout=10) as client:
        metadata = await client.get(
            f"{emulate_google_url}/drive/v3/files/{file_id}",
            headers=google_headers(),
        )
        media = await client.get(
            f"{emulate_google_url}/drive/v3/files/{file_id}",
            headers=google_headers(),
            params={"alt": "media"},
        )

    metadata.raise_for_status()
    media.raise_for_status()
    assert metadata.json()["name"] == title
    assert media.text == expected_content
