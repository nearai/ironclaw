"""Legacy routines/automation management coverage ported to Reborn WebChat v2."""

import asyncio
import json
from copy import deepcopy
from urllib.parse import parse_qs, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


def _automation(
    automation_id: str,
    name: str,
    *,
    state: str = "active",
    cron: str = "0 9 * * *",
    next_run_at: str | None = "2026-06-27T09:00:00Z",
    last_status: str | None = None,
    recent_runs: list[dict] | None = None,
) -> dict:
    return {
        "automation_id": automation_id,
        "name": name,
        "source": {"type": "schedule", "cron": cron, "timezone": "UTC"},
        "state": state,
        "is_active": state in ("active", "scheduled"),
        "next_run_at": next_run_at,
        "last_status": last_status,
        "recent_runs": recent_runs or [],
        "created_at": "2026-06-20T12:00:00Z",
    }


def _run(status: str, *, run_id: str, thread_id: str | None = None) -> dict:
    payload = {
        "run_id": run_id,
        "status": status,
        "submitted_at": "2026-06-25T12:00:00Z",
    }
    if thread_id:
        payload["thread_id"] = thread_id
    if status != "running":
        payload["completed_at"] = "2026-06-25T12:01:00Z"
    return payload


MOCK_AUTOMATIONS = [
    _automation(
        "daily-report",
        "Daily report",
        cron="0 9 * * 1-5",
        next_run_at="2026-06-27T09:00:00Z",
        last_status="ok",
        recent_runs=[
            _run(
                "ok",
                run_id="11111111-1111-1111-1111-111111111111",
                thread_id="thread-daily-report",
            )
        ],
    ),
    _automation(
        "paused-queue",
        "Paused queue",
        state="paused",
        cron="0 11 * * *",
        next_run_at="2026-06-27T11:00:00Z",
    ),
    _automation(
        "failing-sync",
        "Failing sync",
        cron="*/15 * * * *",
        next_run_at="2026-06-27T12:00:00Z",
        last_status="error",
        recent_runs=[
            _run(
                "error",
                run_id="22222222-2222-2222-2222-222222222222",
                thread_id="thread-failing-sync",
            )
        ],
    ),
    _automation(
        "running-import",
        "Running import",
        cron="0 * * * *",
        next_run_at="2026-06-27T13:00:00Z",
        recent_runs=[
            _run(
                "running",
                run_id="33333333-3333-3333-3333-333333333333",
                thread_id="thread-running-import",
            )
        ],
    ),
    _automation(
        "completed-once",
        "Completed one-shot",
        state="completed",
        next_run_at=None,
        last_status="ok",
        recent_runs=[
            _run(
                "ok",
                run_id="44444444-4444-4444-4444-444444444444",
                thread_id="thread-completed-once",
            )
        ],
    ),
]

SLACK_DM_TARGET = {
    "target_id": "slack:personal-dm:T123:U456",
    "channel": "slack",
    "display_name": "Slack DM",
    "description": "Direct messages in Slack",
}

SLACK_OPS_TARGET = {
    "target_id": "slack:channel:T123:COPS",
    "channel": "slack",
    "display_name": "Slack Ops",
    "description": "Operations channel",
}


async def _open_mocked_automations_page(
    reborn_v2_server,
    reborn_v2_browser,
    *,
    automations=None,
    scheduler_enabled=True,
):
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    current_automations = [deepcopy(item) for item in (automations or MOCK_AUTOMATIONS)]
    list_requests: list[dict] = []
    pause_requests: list[str] = []
    resume_requests: list[str] = []
    delete_requests: list[str] = []
    preference_requests: list[dict] = []
    preferences = {
        "final_reply_target": dict(SLACK_DM_TARGET),
        "final_reply_target_status": "available",
        "default_modality": "text",
    }
    targets = [
        {
            "target": dict(SLACK_DM_TARGET),
            "capabilities": {
                "final_replies": True,
                "gate_prompts": True,
                "auth_prompts": True,
            },
        },
        {
            "target": dict(SLACK_OPS_TARGET),
            "capabilities": {
                "final_replies": True,
                "gate_prompts": True,
                "auth_prompts": True,
            },
        },
    ]

    async def fulfill_json(route, payload, status=200):
        await route.fulfill(
            status=status,
            content_type="application/json",
            body=json.dumps(payload),
            headers={"Cache-Control": "no-store"},
        )

    def find_automation(automation_id: str) -> dict | None:
        return next(
            (
                automation
                for automation in current_automations
                if automation["automation_id"] == automation_id
            ),
            None,
        )

    async def handle_automations(route):
        nonlocal current_automations
        request = route.request
        parsed = urlparse(request.url)
        path = parsed.path

        if path == "/api/webchat/v2/automations" and request.method == "GET":
            query = parse_qs(parsed.query)
            list_requests.append(query)
            include_completed = query.get("include_completed") == ["true"]
            visible = [
                automation
                for automation in current_automations
                if include_completed or automation.get("state") != "completed"
            ]
            await fulfill_json(
                route,
                {
                    "automations": visible,
                    "scheduler_enabled": scheduler_enabled,
                },
            )
            return

        if (
            path.startswith("/api/webchat/v2/automations/")
            and path.endswith("/pause")
            and request.method == "POST"
        ):
            automation_id = path.removeprefix("/api/webchat/v2/automations/").removesuffix(
                "/pause"
            )
            pause_requests.append(automation_id)
            automation = find_automation(automation_id)
            if automation:
                automation["state"] = "paused"
                automation["is_active"] = False
            await fulfill_json(route, {"updated": True, "automation": automation})
            return

        if (
            path.startswith("/api/webchat/v2/automations/")
            and path.endswith("/resume")
            and request.method == "POST"
        ):
            automation_id = path.removeprefix("/api/webchat/v2/automations/").removesuffix(
                "/resume"
            )
            resume_requests.append(automation_id)
            automation = find_automation(automation_id)
            if automation:
                automation["state"] = "active"
                automation["is_active"] = True
            await fulfill_json(route, {"updated": True, "automation": automation})
            return

        if (
            path.startswith("/api/webchat/v2/automations/")
            and request.method == "DELETE"
        ):
            automation_id = path.removeprefix("/api/webchat/v2/automations/")
            delete_requests.append(automation_id)
            current_automations = [
                automation
                for automation in current_automations
                if automation["automation_id"] != automation_id
            ]
            await fulfill_json(route, {"updated": True})
            return

        await route.continue_()

    async def handle_outbound(route):
        request = route.request
        path = urlparse(request.url).path

        if path == "/api/webchat/v2/outbound/preferences" and request.method == "GET":
            await fulfill_json(route, preferences)
            return

        if path == "/api/webchat/v2/outbound/targets" and request.method == "GET":
            await fulfill_json(route, {"targets": targets, "next_cursor": None})
            return

        if path == "/api/webchat/v2/outbound/preferences" and request.method == "POST":
            payload = json.loads(request.post_data or "{}")
            preference_requests.append(payload)
            target_id = payload.get("final_reply_target_id")
            selected = next(
                (
                    option["target"]
                    for option in targets
                    if option["target"]["target_id"] == target_id
                ),
                None,
            )
            preferences["final_reply_target"] = selected
            preferences["final_reply_target_status"] = (
                "available" if selected else "none_configured"
            )
            await fulfill_json(route, preferences)
            return

        await route.continue_()

    await page.route("**/api/webchat/v2/automations**", handle_automations)
    await page.route("**/api/webchat/v2/outbound/**", handle_outbound)
    await page.goto(f"{reborn_v2_server}/v2/automations?token={REBORN_V2_AUTH_TOKEN}")
    await expect(page.get_by_role("heading", name="Automations")).to_be_visible(
        timeout=15000
    )

    return {
        "context": context,
        "page": page,
        "list_requests": list_requests,
        "pause_requests": pause_requests,
        "resume_requests": resume_requests,
        "delete_requests": delete_requests,
        "preference_requests": preference_requests,
    }


def _row(page, name: str):
    return page.get_by_role("row").filter(has_text=name)


async def _select_automation(page, name: str):
    await _row(page, name).get_by_role("button").click()


async def _accept_next_confirm(page):
    loop = asyncio.get_running_loop()
    dialog_future = loop.create_future()

    def handle_dialog(dialog):
        if not dialog_future.done():
            dialog_future.set_result({"type": dialog.type, "message": dialog.message})
        loop.create_task(dialog.accept())

    page.once("dialog", handle_dialog)
    return dialog_future


async def test_reborn_legacy_automations_render_scheduler_and_delivery_defaults(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_automations_page(
        reborn_v2_server,
        reborn_v2_browser,
        scheduler_enabled=False,
    )
    try:
        page = harness["page"]

        await expect(page.get_by_text("Scheduling is turned off")).to_be_visible()
        await expect(page.get_by_text("Delivery defaults")).to_be_visible()
        await expect(page.get_by_text("Current default")).to_be_visible()
        await expect(page.get_by_text("Slack DM").first).to_be_visible()
        await expect(page.get_by_text("Slack Ops")).to_be_visible()

        await expect(_row(page, "Daily report")).to_be_visible()
        await expect(_row(page, "Failing sync")).to_be_visible()
        await expect(_row(page, "Running import")).to_be_visible()
        await expect(_row(page, "Paused queue")).to_be_visible()
        await expect(_row(page, "Completed one-shot")).to_have_count(0)
        await expect(
            page.get_by_role("cell", name="Weekdays at 9:00 AM (UTC)").first
        ).to_be_visible()
        await expect(page.get_by_role("cell", name="Every 15 minutes").first).to_be_visible()

        assert harness["list_requests"][0].get("limit") == ["50"]
        assert harness["list_requests"][0].get("run_limit") == ["25"]
        assert "include_completed" not in harness["list_requests"][0]

        await page.get_by_text("Slack Ops", exact=True).click()
        await page.get_by_role("button", name="Save").click()
        await expect(page.get_by_role("status").filter(has_text="Saved")).to_be_visible(
            timeout=5000
        )
        assert harness["preference_requests"] == [
            {"final_reply_target_id": SLACK_OPS_TARGET["target_id"]}
        ]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_automations_filters_and_completed_query(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_automations_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]

        await page.get_by_role("button", name="Failures", exact=True).click()
        await expect(_row(page, "Failing sync")).to_be_visible()
        await expect(_row(page, "Daily report")).to_have_count(0)
        await expect(_row(page, "Running import")).to_have_count(0)

        await page.get_by_role("button", name="Running", exact=True).click()
        await expect(_row(page, "Running import")).to_be_visible()
        await expect(_row(page, "Failing sync")).to_have_count(0)

        await page.get_by_role("button", name="Paused", exact=True).click()
        await expect(_row(page, "Paused queue")).to_be_visible()
        await expect(_row(page, "Running import")).to_have_count(0)

        await page.get_by_role("button", name="Completed", exact=True).click()
        await expect(_row(page, "Completed one-shot")).to_be_visible(timeout=5000)
        await expect(_row(page, "Daily report")).to_have_count(0)

        assert any(
            request.get("include_completed") == ["true"]
            for request in harness["list_requests"]
        )
    finally:
        await harness["context"].close()


async def test_reborn_legacy_automations_pause_resume_and_delete(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_automations_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]

        await _select_automation(page, "Daily report")
        await page.get_by_role("button", name="Pause: Daily report").click()
        await expect(
            page.get_by_role("button", name="Resume: Daily report")
        ).to_be_visible(timeout=5000)
        assert harness["pause_requests"] == ["daily-report"]

        await _select_automation(page, "Paused queue")
        await page.get_by_role("button", name="Resume: Paused queue").click()
        await expect(
            page.get_by_role("button", name="Pause: Paused queue")
        ).to_be_visible(timeout=5000)
        assert harness["resume_requests"] == ["paused-queue"]

        await _select_automation(page, "Failing sync")
        dialog_future = await _accept_next_confirm(page)
        await page.get_by_role("button", name="Delete: Failing sync").click()
        dialog = await asyncio.wait_for(dialog_future, timeout=5)
        assert dialog["type"] == "confirm"
        assert "Failing sync" in dialog["message"]

        await expect(_row(page, "Failing sync")).to_have_count(0, timeout=5000)
        assert harness["delete_requests"] == ["failing-sync"]
    finally:
        await harness["context"].close()
