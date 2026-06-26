"""Legacy project overview coverage ported to Reborn WebChat v2."""

import json
from urllib.parse import unquote, urlparse

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


MOCK_PROJECT_ID = "068f67da-49b6-4f6c-9463-8d243c2cff6c"
PRODUCT_PROJECT_ID = "b1234567-cafe-4000-a000-111111111111"

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
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    project_requests: list[str] = []

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

    await page.route("**/api/webchat/v2/projects**", handle_projects)
    await page.goto(f"{reborn_v2_server}/v2/projects?token={REBORN_V2_AUTH_TOKEN}")

    try:
        await expect(page.locator(SEL_V2["projects_grid"])).to_be_visible(timeout=15000)
    except AssertionError as error:
        body_text = await page.locator("body").inner_text(timeout=1000)
        raise AssertionError(
            f"Projects grid did not render on {page.url}.\nBody text:\n{body_text}"
        ) from error

    return {"context": context, "page": page, "project_requests": project_requests}


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
