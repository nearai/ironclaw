"""Screenshot test for the project detail (drill-in) page.

Seeds mock data via page.route() API interception, navigates to the
projects tab, drills into a project, and captures a screenshot for
PR documentation.
"""

import json


# ── Mock data ───────────────────────────────────────────────────

MOCK_PROJECT_ID = "068f67da-49b6-4f6c-9463-8d243c2cff6c"

MOCK_OVERVIEW = {
    "projects": [
        {
            "id": "default",
            "name": "default",
            "description": "",
            "active_missions": 2,
            "threads_today": 5,
            "cost_today_usd": 0.12,
            "health": "green",
            "last_activity": "2026-04-12T10:30:00Z",
        },
        {
            "id": MOCK_PROJECT_ID,
            "name": "AI Research Intelligence",
            "description": "Stay informed on the latest AI research — daily paper digests, weekly trend analysis, monthly reviews.",
            "goals": [
                "Monitor arXiv AI papers daily",
                "Filter and rank high-impact research",
                "Generate weekly trend synthesis reports",
                "Track paradigm shifts and emerging topics",
            ],
            "active_missions": 3,
            "threads_today": 7,
            "cost_today_usd": 0.45,
            "health": "green",
            "last_activity": "2026-04-12T09:15:00Z",
        },
        {
            "id": "b1234567-cafe-4000-a000-111111111111",
            "name": "Product Launch Q2",
            "description": "Coordinate the Q2 product launch campaign across marketing, engineering, and sales.",
            "goals": [
                "Ship v2.0 by June 15",
                "Hit 10K signups in launch week",
            ],
            "active_missions": 4,
            "threads_today": 3,
            "cost_today_usd": 0.23,
            "health": "yellow",
            "last_activity": "2026-04-12T08:45:00Z",
        },
    ],
    "attention": [
        {
            "type": "gate",
            "project_id": MOCK_PROJECT_ID,
            "thread_id": "t-001",
            "project_name": "AI Research Intelligence",
            "message": "Approval needed: web_fetch for arxiv.org",
        },
    ],
}

MOCK_MISSIONS = {
    "missions": [
        {
            "id": "m-001",
            "name": "Daily AI Paper Monitoring",
            "status": "Active",
            "cadence_type": "daily",
            "cadence_description": "Every day at 9:00 AM",
            "thread_count": 42,
            "last_run": "2026-04-12T09:00:00Z",
        },
        {
            "id": "m-002",
            "name": "Weekly Trend Synthesis",
            "status": "Active",
            "cadence_type": "weekly",
            "cadence_description": "Every Monday at 10:00 AM",
            "thread_count": 6,
            "last_run": "2026-04-07T10:00:00Z",
        },
        {
            "id": "m-003",
            "name": "Monthly Research Review",
            "status": "Active",
            "cadence_type": "monthly",
            "cadence_description": "1st of each month",
            "thread_count": 3,
            "last_run": "2026-04-01T12:00:00Z",
        },
        {
            "id": "m-004",
            "name": "Knowledge Base Maintenance",
            "status": "Paused",
            "cadence_type": "daily",
            "cadence_description": "Every day at 11:00 AM",
            "thread_count": 15,
            "last_run": "2026-04-10T11:00:00Z",
        },
    ],
}

MOCK_THREADS = {
    "threads": [
        {
            "id": "t-001",
            "title": "Daily digest — April 12",
            "state": "Running",
            "updated_at": "2026-04-12T09:15:00Z",
            "goal": "Scan arXiv for new AI papers",
        },
        {
            "id": "t-002",
            "title": "Weekly synthesis — Week 15",
            "state": "Done",
            "updated_at": "2026-04-07T10:45:00Z",
            "goal": "Analyze weekly research trends",
        },
        {
            "id": "t-003",
            "title": "Daily digest — April 11",
            "state": "Done",
            "updated_at": "2026-04-11T09:30:00Z",
            "goal": "Scan arXiv for new AI papers",
        },
        {
            "id": "t-004",
            "title": "Knowledge base update — April 10",
            "state": "Failed",
            "updated_at": "2026-04-10T11:20:00Z",
            "goal": "Update knowledge base with new papers",
        },
        {
            "id": "t-005",
            "title": "Daily digest — April 10",
            "state": "Done",
            "updated_at": "2026-04-10T09:25:00Z",
            "goal": "Scan arXiv for new AI papers",
        },
    ],
}


# ── Test ────────────────────────────────────────────────────────


async def test_project_detail_screenshot(page):
    """Navigate to projects tab, drill into a project, capture screenshot."""

    # Intercept API calls to return mock data.
    async def handle_overview(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(MOCK_OVERVIEW),
        )

    async def handle_missions(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(MOCK_MISSIONS),
        )

    async def handle_threads(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(MOCK_THREADS),
        )

    async def handle_widgets(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps([]),
        )

    await page.route("**/api/engine/projects/overview", handle_overview)
    await page.route("**/api/engine/missions*", handle_missions)
    await page.route("**/api/engine/threads*", handle_threads)
    await page.route("**/api/engine/projects/*/widgets", handle_widgets)

    # Enable engine v2 mode so the Projects tab is visible.
    await page.evaluate("engineV2 = true; applyEngineModeToTabs();")

    # Click the Projects tab.
    await page.locator('.tab-bar button[data-tab="projects"]').click()
    await page.locator("#cr-cards").wait_for(state="visible", timeout=5000)

    # Wait for project cards to render.
    await page.locator(".cr-card").first.wait_for(state="visible", timeout=5000)

    # Drill into the AI Research Intelligence project.
    await page.locator(
        f'.cr-card[data-id="{MOCK_PROJECT_ID}"]'
    ).click()

    # Wait for drill-in view to render.
    await page.locator("#cr-drill").wait_for(state="visible", timeout=5000)
    await page.locator(".cr-drill-name").wait_for(state="visible", timeout=5000)

    # Wait for missions to render.
    await page.locator(".cr-mission-card").first.wait_for(
        state="visible", timeout=5000
    )

    # Take the screenshot.
    await page.screenshot(path="project-detail.png")
