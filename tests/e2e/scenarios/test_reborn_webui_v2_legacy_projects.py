"""Legacy project overview coverage ported to Reborn WebChat v2."""

import json
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


MOCK_PROJECT_ID = "068f67da-49b6-4f6c-9463-8d243c2cff6c"
PRODUCT_PROJECT_ID = "b1234567-cafe-4000-a000-111111111111"
PROJECT_THREAD_ID = "thread-project-files"
PROJECT_WORKSPACE_FILE_BYTES = b"# Launch Brief\n\nShip the research digest.\n"

MOCK_PROJECTS = [
    {
        "project_id": "default",
        "name": "default",
        "description": "",
        "metadata": {},
        "state": "active",
        "role": "owner",
        "created_at": "2026-04-12T08:00:00Z",
        "updated_at": "2026-04-12T10:30:00Z",
    },
    {
        "project_id": MOCK_PROJECT_ID,
        "name": "AI Research Intelligence",
        "description": (
            "Stay informed on the latest AI research with daily paper digests "
            "and weekly trend analysis."
        ),
        "metadata": {
            "goals": [
                "Monitor arXiv AI papers daily",
                "Generate weekly trend synthesis reports",
            ]
        },
        "state": "active",
        "role": "owner",
        "created_at": "2026-04-12T08:45:00Z",
        "updated_at": "2026-04-12T09:15:00Z",
    },
    {
        "project_id": PRODUCT_PROJECT_ID,
        "name": "Product Launch Q2",
        "description": (
            "Coordinate the Q2 product launch campaign across marketing, "
            "engineering, and sales."
        ),
        "metadata": {"goals": ["Ship v2.0 by June 15", "Hit launch signups"]},
        "state": "active",
        "role": "owner",
        "created_at": "2026-04-11T08:45:00Z",
        "updated_at": "2026-04-12T08:45:00Z",
    },
]


async def _open_mocked_projects_page(reborn_v2_server, reborn_v2_browser):
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}
    )
    try:
        page = await context.new_page()
        project_requests: list[str] = []
        thread_create_requests: list[dict] = []
        project_thread_requests: list[str] = []
        project_file_requests: list[str] = []

        async def fulfill_json(route, payload, status=200):
            await route.fulfill(
                status=status,
                content_type="application/json",
                body=json.dumps(payload),
                headers={"Cache-Control": "no-store"},
            )

        async def handle_projects(route):
            request = route.request
            parsed = urlparse(request.url)
            path = parsed.path

            if path == "/api/webchat/v2/projects" and request.method == "GET":
                project_requests.append(path)
                await fulfill_json(route, {"projects": MOCK_PROJECTS})
                return

            prefix = "/api/webchat/v2/projects/"
            if path.startswith(prefix) and request.method == "GET":
                project_id = unquote(path.removeprefix(prefix))
                project_requests.append(path)
                project = next(
                    (
                        candidate
                        for candidate in MOCK_PROJECTS
                        if candidate["project_id"] == project_id
                    ),
                    None,
                )
                await fulfill_json(
                    route,
                    {"project": project},
                    status=200 if project is not None else 404,
                )
                return

            await route.continue_()

        async def handle_threads(route):
            request = route.request
            parsed = urlparse(request.url)
            path = parsed.path

            if path == "/api/webchat/v2/threads" and request.method == "GET":
                query = parse_qs(parsed.query)
                if query.get("project_id") == [MOCK_PROJECT_ID]:
                    project_thread_requests.append(request.url)
                    await fulfill_json(
                        route,
                        {
                            "threads": [
                                {
                                    "thread_id": PROJECT_THREAD_ID,
                                    "title": "Weekly research digest",
                                    "goal": "Summarize launch-readiness signals.",
                                    "thread_type": "chat",
                                    "project_id": MOCK_PROJECT_ID,
                                    "created_at": "2026-04-12T11:30:00Z",
                                    "updated_at": "2026-04-12T12:00:00Z",
                                }
                            ],
                            "next_cursor": None,
                        },
                    )
                    return
                await fulfill_json(route, {"threads": [], "next_cursor": None})
                return

            if path == "/api/webchat/v2/threads" and request.method == "POST":
                body = json.loads(request.post_data or "{}")
                thread_create_requests.append(body)
                await fulfill_json(
                    route,
                    {
                        "thread": {
                            "thread_id": "thread-project-scoped",
                            "title": "Project scoped conversation",
                            "project_id": body.get("project_id"),
                            "created_at": "2026-04-12T11:00:00Z",
                            "updated_at": "2026-04-12T11:00:00Z",
                        }
                    },
                )
                return

            if path == "/api/webchat/v2/threads/thread-project-scoped/timeline":
                await fulfill_json(route, {"messages": [], "next_cursor": None})
                return

            if path == f"/api/webchat/v2/threads/{PROJECT_THREAD_ID}/files":
                project_file_requests.append(request.url)
                query = parse_qs(parsed.query)
                if query.get("path") == ["/workspace/reports"]:
                    await fulfill_json(
                        route,
                        {
                            "entries": [
                                {
                                    "name": "launch-brief.md",
                                    "path": "/workspace/reports/launch-brief.md",
                                    "kind": "file",
                                    "size": len(PROJECT_WORKSPACE_FILE_BYTES),
                                }
                            ]
                        },
                    )
                    return

                await fulfill_json(
                    route,
                    {
                        "entries": [
                            {
                                "name": "reports",
                                "path": "/workspace/reports",
                                "kind": "directory",
                            },
                            {
                                "name": "README.md",
                                "path": "/workspace/README.md",
                                "kind": "file",
                                "size": 42,
                            },
                        ]
                    },
                )
                return

            if path == f"/api/webchat/v2/threads/{PROJECT_THREAD_ID}/files/content":
                project_file_requests.append(request.url)
                await route.fulfill(
                    status=200,
                    content_type="text/markdown",
                    body=PROJECT_WORKSPACE_FILE_BYTES.decode("utf-8"),
                    headers={"Cache-Control": "no-store"},
                )
                return

            await route.continue_()

        await page.route("**/api/webchat/v2/projects**", handle_projects)
        await page.route("**/api/webchat/v2/threads**", handle_threads)
        await page.goto(
            f"{reborn_v2_server}/v2/projects?token={REBORN_V2_AUTH_TOKEN}"
        )

        try:
            await expect(page.locator(SEL_V2["projects_grid"])).to_be_visible(
                timeout=15000
            )
        except AssertionError as error:
            body_text = await page.locator("body").inner_text(timeout=1000)
            raise AssertionError(
                f"Projects grid did not render on {page.url}.\nBody text:\n{body_text}"
            ) from error

        return {
            "context": context,
            "page": page,
            "project_requests": project_requests,
            "thread_create_requests": thread_create_requests,
            "project_thread_requests": project_thread_requests,
            "project_file_requests": project_file_requests,
        }
    except Exception:
        await context.close()
        raise


async def test_reborn_legacy_projects_overview_search_and_open_workspace(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_projects_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        project_requests = harness["project_requests"]

        default_card = page.locator(SEL_V2["project_card_for"].format(id="default"))
        research_card = page.locator(
            SEL_V2["project_card_for"].format(id=MOCK_PROJECT_ID)
        )
        product_card = page.locator(
            SEL_V2["project_card_for"].format(id=PRODUCT_PROJECT_ID)
        )

        await expect(default_card).to_be_visible(timeout=5000)
        await expect(research_card).to_contain_text("AI Research Intelligence")
        await expect(research_card).to_contain_text("weekly trend analysis")
        await expect(research_card).to_contain_text("Monitor arXiv AI papers daily")
        await expect(product_card).to_contain_text("Product Launch Q2")

        search = page.locator(SEL_V2["projects_search_input"])
        await search.fill("trend synthesis")
        await expect(research_card).to_be_visible()
        await expect(product_card).to_have_count(0)
        await expect(default_card).to_have_count(0)

        await search.fill("")
        research_card = page.locator(
            SEL_V2["project_card_for"].format(id=MOCK_PROJECT_ID)
        )
        await research_card.locator(SEL_V2["project_open_workspace"]).click()

        await expect(
            page.locator(SEL_V2["project_workspace_for"].format(id=MOCK_PROJECT_ID))
        ).to_be_visible(timeout=10000)
        await expect(page.locator(SEL_V2["project_workspace_title"])).to_have_text(
            "AI Research Intelligence"
        )
        await page.wait_for_url(f"**/v2/projects/{MOCK_PROJECT_ID}**", timeout=5000)
        assert "/api/webchat/v2/projects" in project_requests
        assert f"/api/webchat/v2/projects/{MOCK_PROJECT_ID}" in project_requests
    finally:
        await harness["context"].close()


async def test_reborn_legacy_projects_search_no_match_can_be_cleared(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_projects_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]
        search = page.locator(SEL_V2["projects_search_input"])

        await search.fill("no-project-matches-this")
        await expect(
            page.get_by_text("No projects match the current search")
        ).to_be_visible(timeout=5000)
        await expect(search).to_be_visible()
        await expect(
            page.locator(SEL_V2["project_card_for"].format(id=MOCK_PROJECT_ID))
        ).to_have_count(0)

        await search.fill("")
        await expect(
            page.locator(SEL_V2["project_card_for"].format(id=MOCK_PROJECT_ID))
        ).to_be_visible(timeout=5000)
        await expect(
            page.locator(SEL_V2["project_card_for"].format(id=PRODUCT_PROJECT_ID))
        ).to_be_visible()
    finally:
        await harness["context"].close()


async def test_reborn_legacy_project_creation_opens_seeded_chat_thread(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_projects_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]

        await page.get_by_role("button", name="New project").click()
        await page.wait_for_url("**/v2/chat/thread-project-scoped", timeout=10000)

        composer = page.locator(SEL_V2["chat_composer"])
        await expect(composer).to_be_visible(timeout=10000)
        await expect(composer).to_have_value(
            "Create a new project for me. I want to set up a project for: ",
            timeout=5000,
        )

        assert len(harness["thread_create_requests"]) == 1
        assert "project_id" not in harness["thread_create_requests"][0]
        assert harness["thread_create_requests"][0]["client_action_id"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_project_workspace_starts_scoped_chat_thread(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_projects_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]

        await page.locator(
            SEL_V2["project_card_for"].format(id=MOCK_PROJECT_ID)
        ).locator(SEL_V2["project_open_workspace"]).click()
        await expect(
            page.locator(SEL_V2["project_workspace_for"].format(id=MOCK_PROJECT_ID))
        ).to_be_visible(timeout=10000)

        await page.get_by_role("button", name="New conversation").click()
        await page.wait_for_url("**/v2/chat/thread-project-scoped", timeout=10000)
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=10000)

        assert len(harness["thread_create_requests"]) == 1
        assert harness["thread_create_requests"][0]["project_id"] == MOCK_PROJECT_ID
        assert harness["thread_create_requests"][0]["client_action_id"]
    finally:
        await harness["context"].close()


async def test_reborn_legacy_project_workspace_lists_and_downloads_files(
    reborn_v2_server, reborn_v2_browser
):
    harness = await _open_mocked_projects_page(reborn_v2_server, reborn_v2_browser)
    try:
        page = harness["page"]

        await page.locator(
            SEL_V2["project_card_for"].format(id=MOCK_PROJECT_ID)
        ).locator(SEL_V2["project_open_workspace"]).click()
        await expect(
            page.locator(SEL_V2["project_workspace_for"].format(id=MOCK_PROJECT_ID))
        ).to_be_visible(timeout=10000)

        await expect(page.get_by_text("Weekly research digest")).to_be_visible(
            timeout=10000
        )
        reports_entry = page.locator(
            SEL_V2["project_filesystem_entry_for"].format(path="/workspace/reports")
        )
        await expect(reports_entry).to_be_visible(timeout=10000)
        await reports_entry.click()

        launch_brief_entry = page.locator(
            SEL_V2["project_filesystem_entry_for"].format(
                path="/workspace/reports/launch-brief.md"
            )
        )
        await expect(launch_brief_entry).to_be_visible(timeout=10000)

        async with page.expect_download() as download_info:
            await launch_brief_entry.click()
        download = await download_info.value
        assert download.suggested_filename == "launch-brief.md"
        assert Path(await download.path()).read_bytes() == PROJECT_WORKSPACE_FILE_BYTES

        assert harness["project_thread_requests"]
        assert any("project_id=" in url for url in harness["project_thread_requests"])
        assert any("/files" in url for url in harness["project_file_requests"])
        assert any("/files/content" in url for url in harness["project_file_requests"])
    finally:
        await harness["context"].close()
