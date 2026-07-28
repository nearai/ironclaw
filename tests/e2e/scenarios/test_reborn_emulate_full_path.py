"""Full-path Reborn extension tests backed by Emulate provider state."""

import uuid

import httpx

from emulate_provider import (
    github_json,
    gmail_header,
    google_headers,
    slack_post,
)
from helpers import EMULATE_SLACK_BEARER, api_post
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


async def test_reborn_gmail_lifecycle_mutates_emulate(
    hosted_google_emulate_server,
):
    """Drive the real extension caller through send, readback, and cleanup."""
    server = hosted_google_emulate_server["base_url"]
    emulate_google_url = hosted_google_emulate_server["emulate_google_url"]

    await install_extension(server, "gmail")
    await complete_oauth_setup(server, "gmail", code="mock_auth_code")

    gmail = await get_extension(server, "gmail")
    assert gmail is not None, (
        "gmail should be installed; "
        f"available extensions: {await extension_names(server)}"
    )
    assert gmail["authenticated"] is True, gmail
    assert "gmail" in gmail.get("tools", []), gmail

    subject = f"[canary] reborn-emulate-gmail-{uuid.uuid4().hex[:8]}"
    thread_id = await new_thread(server)
    await send_chat(
        server,
        thread_id,
        f"send an email to e2e.google@example.com with subject '{subject}'",
    )

    history = await wait_for_response_containing(
        server,
        thread_id,
        "gmail roundtrip complete",
        timeout=90,
    )
    tool_results = completed_tool_results(history, "gmail")
    assert len(tool_results) >= 3, [tool_result_text(result) for result in tool_results]

    sent = tool_result_json(tool_results[0])
    sent_message = sent.get("message", sent)
    message_id = sent_message["id"]

    async with httpx.AsyncClient(timeout=10) as client:
        readback = await client.get(
            f"{emulate_google_url}/gmail/v1/users/me/messages/{message_id}",
            headers=google_headers(),
            params={"format": "full"},
        )
    readback.raise_for_status()
    message = readback.json()
    assert gmail_header(message, "Subject") == subject
    assert "TRASH" in message.get("labelIds", [])


async def test_reborn_slack_delivery_and_github_release_cross_provider_path(
    hosted_provider_emulate_server,
):
    """Drive real extension callers and verify exact Slack side effects."""
    server = hosted_provider_emulate_server["base_url"]
    mock_llm_url = hosted_provider_emulate_server["mock_llm_url"]
    github_url = hosted_provider_emulate_server["emulate_github_url"]
    slack_url = hosted_provider_emulate_server["emulate_slack_url"]
    tools_dir = hosted_provider_emulate_server["wasm_tools_dir"]

    await install_extension(server, "slack_tool")
    slack_setup = await api_post(
        server,
        "/api/extensions/slack_tool/setup",
        json={"secrets": {"slack_bot_token": EMULATE_SLACK_BEARER}},
        timeout=30,
    )
    assert slack_setup.status_code == 200, slack_setup.text
    assert slack_setup.json().get("success") is True, slack_setup.text
    await complete_oauth_setup(
        server,
        "slack_tool",
        code="mock_slack_full_path_code",
        mock_base_url=mock_llm_url,
    )
    slack = await get_extension(server, "slack_tool")
    assert slack is not None, await extension_names(server)
    assert slack["authenticated"] is True, slack
    assert "slack_tool" in slack.get("tools", []), slack

    await install_extension(server, "github")
    patch_extension_validation_endpoint(
        tools_dir,
        "github",
        f"{github_url}/user",
    )
    await complete_oauth_setup(
        server,
        "github",
        code="mock_github_cross_provider_code",
        mock_base_url=mock_llm_url,
    )

    for extension_name in ("google-calendar", "google-drive", "gmail"):
        await install_extension(server, extension_name)
        await complete_oauth_setup(
            server,
            extension_name,
            code="mock_auth_code",
            mock_base_url=mock_llm_url,
        )

    async with httpx.AsyncClient(timeout=10) as client:
        users = await slack_post(client, slack_url, "users.list")
        reviewers = {
            member["name"]: member["id"]
            for member in users["members"]
            if member["name"] in {"qa-reviewer", "no-email-user"}
        }
        dm_channels = {}
        for name, user_id in reviewers.items():
            opened = await slack_post(
                client,
                slack_url,
                "conversations.open",
                {"users": user_id, "return_im": True},
            )
            dm_channels[name] = opened["channel"]["id"]

        release_tag = f"reborn-cross-provider-{uuid.uuid4().hex[:8]}"
        await github_json(
            client,
            github_url,
            "POST",
            "/repos/nearai/ironclaw/releases",
            payload={
                "tag_name": release_tag,
                "name": "Reborn cross-provider canary",
                "body": "Fixture release for GitHub to Slack dispatch.",
            },
            expected_status=201,
        )

    # Two separate deliveries prove that channel selection is not shared and
    # that each requested target receives the marker exactly once.
    markers = {
        "qa-reviewer": f"slack-dm-a-{uuid.uuid4().hex[:8]}",
        "no-email-user": f"slack-dm-b-{uuid.uuid4().hex[:8]}",
    }
    for name, marker in markers.items():
        thread_id = await new_thread(server)
        await send_chat(
            server,
            thread_id,
            f"send slack canary {marker} to {dm_channels[name]}",
        )
        delivery_history = await wait_for_response_containing(
            server,
            thread_id,
            "slack delivery lifecycle complete",
            timeout=90,
        )
        responses = " ".join(
            turn.get("response") or ""
            for turn in delivery_history.get("turns", [])
        )
        assert dm_channels[name] not in responses
        assert reviewers[name] not in responses

    cross_marker = f"github-release-slack-{uuid.uuid4().hex[:8]}"
    cross_thread_id = await new_thread(server)
    await send_chat(
        server,
        cross_thread_id,
        "notify slack channel "
        f"{dm_channels['qa-reviewer']} about the latest release in "
        f"nearai/ironclaw with marker {cross_marker}",
    )
    cross_history = await wait_for_response_containing(
        server,
        cross_thread_id,
        "github release to slack lifecycle complete",
        timeout=90,
    )
    assert completed_tool_results(cross_history, "github"), cross_history
    assert completed_tool_results(cross_history, "slack_tool"), cross_history

    async with httpx.AsyncClient(timeout=10) as client:
        histories = {}
        for name, channel in dm_channels.items():
            response = await slack_post(
                client,
                slack_url,
                "conversations.history",
                {"channel": channel},
            )
            histories[name] = [message["text"] for message in response["messages"]]

    for name, marker in markers.items():
        assert histories[name].count(marker) == 1
        other_name = next(candidate for candidate in markers if candidate != name)
        assert markers[other_name] not in histories[name]
    assert histories["qa-reviewer"].count(cross_marker) == 1
    assert cross_marker not in histories["no-email-user"]

    cross_provider_cases = [
        (
            "prepare meeting and notify slack channel "
            f"{dm_channels['qa-reviewer']} with marker {{marker}}",
            "calendar drive to slack complete",
            ("google_calendar", "google_drive", "slack_tool"),
        ),
        (
            "check unread gmail and notify slack channel "
            f"{dm_channels['qa-reviewer']} with marker {{marker}}",
            "gmail to slack complete",
            ("gmail", "slack_tool"),
        ),
        (
            f"read slack channel {dm_channels['qa-reviewer']}, look up drive, "
            f"and notify {dm_channels['no-email-user']} with marker {{marker}}",
            "slack drive to slack complete",
            ("slack_tool", "google_drive"),
        ),
    ]
    workflow_markers = []
    for prompt_template, completion, expected_tools in cross_provider_cases:
        marker = f"cross-provider-{uuid.uuid4().hex[:8]}"
        workflow_markers.append(marker)
        thread_id = await new_thread(server)
        await send_chat(server, thread_id, prompt_template.format(marker=marker))
        workflow_history = await wait_for_response_containing(
            server,
            thread_id,
            completion,
            timeout=90,
        )
        for tool_name in expected_tools:
            assert completed_tool_results(workflow_history, tool_name), workflow_history

    async with httpx.AsyncClient(timeout=10) as client:
        reviewer_history = await slack_post(
            client,
            slack_url,
            "conversations.history",
            {"channel": dm_channels["qa-reviewer"]},
        )
        no_email_history = await slack_post(
            client,
            slack_url,
            "conversations.history",
            {"channel": dm_channels["no-email-user"]},
        )
    reviewer_texts = [message["text"] for message in reviewer_history["messages"]]
    no_email_texts = [message["text"] for message in no_email_history["messages"]]
    assert reviewer_texts.count(workflow_markers[0]) == 1
    assert reviewer_texts.count(workflow_markers[1]) == 1
    assert no_email_texts.count(workflow_markers[2]) == 1
