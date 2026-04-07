"""Scenario: ClawHub disabled mode (CLAWHUB_ENABLED=false).

Verifies that when ClawHub is disabled:
- Gateway status reports clawhub_enabled=false
- Skill search API returns 501
- URL-based skill installs are blocked (403)
- Content-based skill installs still work
- Skill listing still works
"""

from helpers import api_get, api_post, auth_headers


async def test_gateway_status_shows_clawhub_disabled(clawhub_disabled_server):
    """GET /api/gateway/status should report clawhub_enabled: false."""
    r = await api_get(clawhub_disabled_server, "/api/gateway/status")
    assert r.status_code == 200
    assert r.json()["clawhub_enabled"] is False


async def test_skills_search_api_returns_501(clawhub_disabled_server):
    """POST /api/skills/search should return 501 when ClawHub is disabled."""
    r = await api_post(clawhub_disabled_server, "/api/skills/search", json={"query": "test"})
    assert r.status_code == 501


async def test_skills_install_url_blocked(clawhub_disabled_server):
    """POST /api/skills/install with a URL should be rejected when ClawHub is disabled."""
    r = await api_post(
        clawhub_disabled_server,
        "/api/skills/install",
        headers={**auth_headers(), "X-Confirm-Action": "true"},
        json={"url": "https://example.com/SKILL.md"},
    )
    assert r.status_code == 403, f"Expected 403 for URL install, got {r.status_code}: {r.text}"
    assert "disabled" in r.text.lower()


async def test_skills_install_content_allowed(clawhub_disabled_server):
    """POST /api/skills/install with inline content should succeed when ClawHub is disabled."""
    skill_content = (
        "---\n"
        "name: e2e-test-skill\n"
        "version: 0.1.0\n"
        "description: A test skill for E2E\n"
        "activation:\n"
        "  keywords: [e2e-test]\n"
        "---\n"
        "# E2E Test Skill\n"
        "This is a test skill installed via content.\n"
    )
    r = await api_post(
        clawhub_disabled_server,
        "/api/skills/install",
        headers={**auth_headers(), "X-Confirm-Action": "true"},
        json={"content": skill_content},
    )
    assert r.status_code == 200, f"Expected 200 for content install, got {r.status_code}: {r.text}"
    assert r.json().get("success") is True, f"Expected success, got: {r.json()}"


async def test_skills_list_api_still_works(clawhub_disabled_server):
    """GET /api/skills should return 200 regardless of ClawHub status."""
    r = await api_get(clawhub_disabled_server, "/api/skills")
    assert r.status_code == 200
    data = r.json()
    assert "skills" in data
    assert isinstance(data["skills"], list)
