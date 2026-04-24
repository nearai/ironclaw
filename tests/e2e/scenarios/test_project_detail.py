"""Tests for the project detail (drill-in) page.

Seeds mock data via page.route() API interception, navigates to the
projects tab, drills into a project, and asserts control-room behavior.
"""

import json

from helpers import SEL
from playwright.async_api import expect


MOCK_PROJECT_ID = "068f67da-49b6-4f6c-9463-8d243c2cff6c"
FIRST_MISSION_ID = "m-001"
FIRST_MISSION_NAME = "Daily AI Paper Monitoring"
THREAD_DETAIL_ID = "t-002"
THREAD_DETAIL_TITLE = "Daily Work Digest"
THREAD_DETAIL_GOAL = "Analyze weekly research trends"
MISSION_RUN_GOAL = (
    "# Mission: Daily Work Digest Goal: Create and send a daily digest that reviews "
    "my Google Calendar, Gmail, Notion, and GitHub to identify what I need to do that day. "
    "Each run should: 1) look at today's Google Calendar events and summarize schedule and likely priorities; "
    "2) review Gmail for recent unread or important messages that imply actions, deadlines, or follow-ups; "
    "3) review Notion for tasks, meeting notes, pages, or items relevant to today, including due/urgent/open work when available; "
    "4) review GitHub for my open PRs, issues assigned to me, and PRs requesting my review; "
    "5) synthesize everything into one concise actionable morning briefing; 6) send the digest back to me in this conversation channel."
)

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
            "id": FIRST_MISSION_ID,
            "name": FIRST_MISSION_NAME,
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

MOCK_MISSION_DETAIL = {
    "mission": {
        "id": FIRST_MISSION_ID,
        "name": FIRST_MISSION_NAME,
        "status": "Active",
        "goal": "# Input\n- `query` — papers from the last 24h\n\n# Investigation Process\n1. Fetch papers\n2. Rank them\n3. Summarize notable work",
        "cadence_type": "daily",
        "cadence_description": "Every day at 9:00 AM",
        "thread_count": 42,
        "threads_today": 2,
        "max_threads_per_day": 3,
        "created_at": "2026-04-12T08:45:00Z",
        "next_fire_at": "2026-04-13T09:00:00Z",
        "current_focus": "Tighten filtering for papers with real-world impact.",
        "success_criteria": "Return a concise digest with 3-5 papers and clear takeaways.",
        "approach_history": [
            "Expected: produce a daily digest\nObserved: arXiv query is still broad\nFix applied: narrow to ai + cs.LG\nNext focus: improve ranking"
        ],
        "threads": [],
    }
}

MOCK_THREAD_DETAIL = {
    "thread": {
        "id": THREAD_DETAIL_ID,
        "goal": MISSION_RUN_GOAL,
        "title": "",
        "state": "Done",
        "thread_type": "mission_run",
        "step_count": 6,
        "total_tokens": 18234,
        "total_cost_usd": 0.42,
        "created_at": "2026-04-07T10:00:00Z",
        "completed_at": "2026-04-07T10:45:00Z",
        "messages": [
            {"role": "System", "content": "# Mission\nInvestigate weekly research themes."},
            {"role": "Assistant", "content": "## Findings\n- Agentic workflows are trending\n- Benchmarks remain noisy"},
        ],
    }
}


def _json_route(body):
    async def handler(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(body),
        )

    return handler


async def _open_project_detail(page):
    await page.evaluate(
        "() => {"
        "  if (window.bootstrap) window.bootstrap.engineV2Enabled = true;"
        "  engineV2Enabled = true;"
        "  applyEngineModeToTabs();"
        "}"
    )
    await page.locator(SEL["tab_button"].format(tab="projects")).click()
    await page.locator(SEL["projects_cards"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["projects_card"]).first.wait_for(state="visible", timeout=5000)
    await page.locator(SEL["projects_card_by_id"].format(id=MOCK_PROJECT_ID)).click()
    await page.locator(SEL["projects_drill"]).wait_for(state="visible", timeout=5000)
    await page.locator(SEL["projects_drill_name"]).wait_for(state="visible", timeout=5000)


async def _route_project_detail_fixtures(page):
    """Register all mock API routes needed for project detail tests."""

    await page.route("**/api/engine/projects/overview", _json_route(MOCK_OVERVIEW))
    # Register specific detail routes before generic list routes because Playwright
    # resolves overlapping routes in registration order.
    await page.route(
        f"**/api/engine/missions/{FIRST_MISSION_ID}",
        _json_route(MOCK_MISSION_DETAIL),
    )
    await page.route("**/api/engine/missions*", _json_route(MOCK_MISSIONS))
    await page.route(
        f"**/api/engine/threads/{THREAD_DETAIL_ID}",
        _json_route(MOCK_THREAD_DETAIL),
    )
    await page.route("**/api/engine/threads*", _json_route(MOCK_THREADS))
    await page.route("**/api/engine/projects/*/widgets", _json_route([]))


async def test_project_detail_screenshot(page, tmp_path):
    """Navigate to projects tab, drill into a project, capture screenshot."""

    await _route_project_detail_fixtures(page)
    await _open_project_detail(page)

    await expect(page.locator(SEL["projects_drill_name"])).to_have_text(
        "AI Research Intelligence"
    )
    await expect(page.locator(SEL["projects_mission_card"]).first).to_be_visible()
    await expect(page.locator(SEL["projects_activity_row"]).first).to_be_visible()

    await page.screenshot(path=str(tmp_path / "project-detail.png"))


async def test_project_mission_card_opens_canonical_missions_view(page):
    """Mission card in Projects should switch to the Missions tab and open the mission dossier."""
    await _route_project_detail_fixtures(page)
    await _open_project_detail(page)
    await page.locator(SEL["projects_mission_card"]).first.click()

    await expect(page.locator(SEL["tab_button"].format(tab="missions"))).to_have_attribute(
        "aria-selected", "true"
    )
    await expect(page.locator(SEL["tab_panel"].format(tab="projects"))).not_to_be_visible()
    await expect(page.locator(SEL["tab_panel"].format(tab="missions"))).to_be_visible()
    await expect(page.locator(SEL["missions_detail"])).to_be_visible()
    await expect(page.locator(SEL["missions_detail_title"])).to_have_text(
        FIRST_MISSION_NAME
    )


async def test_project_activity_row_opens_polished_thread_inspector(page):
    """Activity row in Projects should open the thread inspector inside Projects."""
    await _route_project_detail_fixtures(page)
    await _open_project_detail(page)
    await page.locator(SEL["projects_activity_row_by_id"].format(id=THREAD_DETAIL_ID)).click()

    await expect(page.locator(SEL["tab_button"].format(tab="projects"))).to_have_attribute(
        "aria-selected", "true"
    )
    await expect(page.locator(SEL["projects_detail"])).to_be_visible()
    await expect(page.locator(SEL["projects_thread_title"])).to_have_text(
        THREAD_DETAIL_TITLE
    )
    await expect(page.locator(SEL["projects_thread_subtitle"])).to_have_text(
        "Mission run"
    )
    await expect(page.locator(SEL["projects_thread_brief"])).to_contain_text(
        "Create and send a daily digest"
    )
    await expect(page.locator(SEL["projects_thread_meta"])).to_be_visible()
    await expect(page.locator(SEL["projects_thread_timeline"])).to_be_visible()
    await expect(page.locator(SEL["projects_thread_message"])).to_have_count(2)
