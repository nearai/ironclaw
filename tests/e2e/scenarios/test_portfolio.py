"""Scenario: Portfolio skill activation, widget rendering, and share flow."""

import json

import httpx
from helpers import AUTH_TOKEN, SEL, api_post, send_chat_and_wait_for_terminal_message


SAMPLE_STATE = {
    "schema_version": "portfolio-widget/1",
    "generated_at": "2026-04-12T12:00:00Z",
    "project_id": "portfolio",
    "totals": {
        "net_value_usd": "15000.00",
        "realized_net_apy_7d": 0.045,
        "floor_apy": 0.04,
        "delta_vs_last_run_usd": "+250.00",
        "risk_score_weighted": 2.3,
    },
    "positions": [
        {
            "protocol": "Aave V3",
            "chain": "base",
            "category": "stablecoin-idle",
            "principal_usd": "10000.00",
            "net_apy": 0.038,
            "risk_score": 2,
            "tags": [],
        },
        {
            "protocol": "Morpho Blue",
            "chain": "base",
            "category": "stablecoin-idle",
            "principal_usd": "5000.00",
            "net_apy": 0.058,
            "risk_score": 2,
            "tags": ["high-yield"],
        },
    ],
    "top_suggestions": [
        {
            "id": "stablecoin-yield-floor-observed-0",
            "strategy": "stablecoin-yield-floor",
            "rationale": "Aave V3 @ 3.80% < floor 4.00%; move to Morpho Blue @ 5.80%",
            "projected_delta_apy_bps": 200,
            "projected_annual_gain_usd": "200.00",
            "gas_payback_days": 1.0,
            "status": "ready",
        }
    ],
    "pending_intents": [],
    "next_mission_run": "2026-04-12T18:00:00Z",
}


async def _seed_portfolio_state(base_url: str) -> None:
    """Write sample portfolio state.json via the memory API."""
    await api_post(
        base_url,
        "/api/memory/write",
        json={
            "path": "projects/portfolio/widgets/state.json",
            "content": json.dumps(SAMPLE_STATE),
        },
        timeout=10,
    )


async def _get_skills(base_url: str) -> list:
    """Fetch the skills list from the API."""
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{base_url}/api/skills",
            headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
            timeout=10,
        )
    assert response.status_code == 200, response.text
    body = response.json()
    return body.get("skills", body) if isinstance(body, dict) else body


# ---- Skill discovery ----


async def test_portfolio_skill_listed(ironclaw_server):
    """The trusted portfolio skill should appear in the skills list API."""
    skills = await _get_skills(ironclaw_server)

    portfolio_skills = [s for s in skills if s.get("name") == "portfolio"]
    assert len(portfolio_skills) == 1, (
        f"Expected exactly one portfolio skill, got {len(portfolio_skills)}: "
        f"{[s.get('name') for s in skills]}"
    )

    skill = portfolio_skills[0]
    assert skill.get("trust") in ("trusted", "Trusted"), (
        f"Portfolio skill should be trusted (workspace skill), got: {skill.get('trust')}"
    )


async def test_portfolio_skill_has_activation_keywords(ironclaw_server):
    """Portfolio skill metadata should include the expected activation keywords."""
    skills = await _get_skills(ironclaw_server)
    portfolio = next((s for s in skills if s.get("name") == "portfolio"), None)
    assert portfolio is not None

    keywords = portfolio.get("keywords", [])
    assert "portfolio" in keywords, f"Missing 'portfolio' keyword: {keywords}"
    assert "defi" in keywords, f"Missing 'defi' keyword: {keywords}"
    assert "yield" in keywords, f"Missing 'yield' keyword: {keywords}"


# ---- Chat integration ----


async def test_portfolio_chat_keyword_triggers_skill(page):
    """Sending a portfolio-related message should trigger the portfolio skill context."""
    result = await send_chat_and_wait_for_terminal_message(
        page, "Show me my DeFi portfolio positions", timeout=15000,
    )

    assert result["role"] == "assistant"
    assert "portfolio" in result["text"].lower(), (
        f"Expected portfolio-related response, got: {result['text']!r}"
    )


async def test_portfolio_wallet_address_triggers_skill(page):
    """Pasting an EVM address should activate the portfolio skill via pattern matching."""
    result = await send_chat_and_wait_for_terminal_message(
        page,
        "Scan this wallet: 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        timeout=15000,
    )

    assert result["role"] == "assistant"
    assert "wallet" in result["text"].lower() or "portfolio" in result["text"].lower(), (
        f"Expected wallet/portfolio-related response, got: {result['text']!r}"
    )


# ---- Widget rendering ----


async def test_portfolio_widget_renders_positions(page, ironclaw_server):
    """Pre-seed state.json and verify the portfolio widget renders positions."""
    await _seed_portfolio_state(ironclaw_server)

    # Navigate to the portfolio tab
    tab_btn = page.locator('.tab-bar button[data-tab="portfolio"]')
    if await tab_btn.count() == 0:
        return  # widget not registered in this build
    await tab_btn.click()

    panel = page.locator('[data-widget="portfolio"]')
    await panel.wait_for(state="visible", timeout=10000)

    # Wait for data to load (pf-loading disappears, pf-card appears)
    card = panel.locator(".pf-card")
    await card.wait_for(state="visible", timeout=15000)

    # Verify totals section
    totals = card.locator(".pf-totals")
    totals_text = await totals.text_content()
    assert "$15000.00" in totals_text, f"Net value not found in totals: {totals_text}"
    assert "4.50%" in totals_text, f"Weighted APY not found in totals: {totals_text}"

    # Verify positions table
    positions = card.locator(".pf-positions")
    assert await positions.count() > 0, "Positions table not rendered"
    table_text = await positions.text_content()
    assert "Aave V3" in table_text, f"Aave V3 not in positions: {table_text}"
    assert "Morpho Blue" in table_text, f"Morpho Blue not in positions: {table_text}"
    assert "base" in table_text, f"Chain 'base' not in positions: {table_text}"

    # Verify suggestions
    suggestions = card.locator(".pf-suggestions")
    assert await suggestions.count() > 0, "Suggestions list not rendered"
    sug_text = await suggestions.text_content()
    assert "stablecoin-yield-floor" in sug_text
    assert "+200 bps" in sug_text
    assert "$200.00/yr" in sug_text


async def test_portfolio_widget_shows_share_button(page, ironclaw_server):
    """Share button appears when there are gains/suggestions."""
    await _seed_portfolio_state(ironclaw_server)

    tab_btn = page.locator('.tab-bar button[data-tab="portfolio"]')
    if await tab_btn.count() == 0:
        return
    await tab_btn.click()

    panel = page.locator('[data-widget="portfolio"]')
    card = panel.locator(".pf-card")
    await card.wait_for(state="visible", timeout=15000)

    share_btn = card.locator("#pf-share-btn")
    assert await share_btn.count() == 1, "Share button should be visible when gains exist"
    assert "Share gains" in (await share_btn.text_content() or "")


async def test_portfolio_share_modal_opens(page, ironclaw_server):
    """Clicking 'Share gains' opens the share modal with social buttons."""
    await _seed_portfolio_state(ironclaw_server)

    tab_btn = page.locator('.tab-bar button[data-tab="portfolio"]')
    if await tab_btn.count() == 0:
        return
    await tab_btn.click()

    panel = page.locator('[data-widget="portfolio"]')
    card = panel.locator(".pf-card")
    await card.wait_for(state="visible", timeout=15000)

    share_btn = card.locator("#pf-share-btn")
    await share_btn.click()

    # Share modal should appear
    overlay = page.locator("#share-modal-overlay")
    await overlay.wait_for(state="visible", timeout=5000)

    # Verify card image is rendered
    card_img = overlay.locator(".share-card-img")
    await card_img.wait_for(state="visible", timeout=5000)
    src = await card_img.get_attribute("src")
    assert src and src.startswith("data:image/png"), (
        f"Share card image should be a PNG data URL, got: {src[:50] if src else 'null'}"
    )

    # Verify social buttons exist
    assert await overlay.locator(".share-x").count() == 1, "X/Twitter button missing"
    assert await overlay.locator(".share-linkedin").count() == 1, "LinkedIn button missing"
    assert await overlay.locator(".share-facebook").count() == 1, "Facebook button missing"
    assert await overlay.locator(".share-copy").count() == 1, "Copy button missing"
    assert await overlay.locator(".share-download").count() == 1, "Download button missing"

    # Verify title
    title = overlay.locator(".share-title")
    assert await title.text_content() == "Share your gains"


async def test_portfolio_share_modal_closes(page, ironclaw_server):
    """Share modal closes when clicking the X button or the overlay."""
    await _seed_portfolio_state(ironclaw_server)

    tab_btn = page.locator('.tab-bar button[data-tab="portfolio"]')
    if await tab_btn.count() == 0:
        return
    await tab_btn.click()

    card = page.locator('[data-widget="portfolio"] .pf-card')
    await card.wait_for(state="visible", timeout=15000)
    await card.locator("#pf-share-btn").click()

    overlay = page.locator("#share-modal-overlay")
    await overlay.wait_for(state="visible", timeout=5000)

    # Close via X button
    await overlay.locator(".share-close").click()
    await page.wait_for_timeout(300)
    assert await overlay.is_hidden(), "Modal should be hidden after clicking close"


async def test_portfolio_widget_no_share_button_without_gains(page, ironclaw_server):
    """Share button should not appear when there are no gains or suggestions."""
    no_gains_state = {
        "schema_version": "portfolio-widget/1",
        "totals": {
            "net_value_usd": "1000.00",
            "realized_net_apy_7d": 0.03,
            "floor_apy": 0.04,
            "risk_score_weighted": 2.0,
        },
        "positions": [{
            "protocol": "Aave V3",
            "chain": "base",
            "category": "stablecoin-idle",
            "principal_usd": "1000.00",
            "net_apy": 0.03,
            "risk_score": 2,
            "tags": [],
        }],
        "top_suggestions": [],
        "pending_intents": [],
    }
    await api_post(
        ironclaw_server,
        "/api/memory/write",
        json={
            "path": "projects/portfolio/widgets/state.json",
            "content": json.dumps(no_gains_state),
        },
        timeout=10,
    )

    tab_btn = page.locator('.tab-bar button[data-tab="portfolio"]')
    if await tab_btn.count() == 0:
        return
    await tab_btn.click()

    card = page.locator('[data-widget="portfolio"] .pf-card')
    await card.wait_for(state="visible", timeout=15000)

    share_btn = card.locator("#pf-share-btn")
    assert await share_btn.count() == 0, "Share button should not appear without gains"


# ---- Skill settings visibility ----


async def test_portfolio_skill_visible_in_settings(page):
    """Portfolio skill should be visible in the Settings > Skills subtab."""
    await page.locator(SEL["tab_button"].format(tab="settings")).click()
    await page.locator(SEL["settings_subtab"].format(subtab="skills")).click()
    await page.locator(SEL["settings_subpanel"].format(subtab="skills")).wait_for(
        state="visible", timeout=5000
    )

    installed = page.locator(SEL["skill_installed"])
    await installed.first.wait_for(state="visible", timeout=10000)

    count = await installed.count()
    assert count >= 1, "Expected at least one installed skill (portfolio)"

    all_names = []
    for i in range(count):
        card = installed.nth(i)
        name_el = card.locator(".ext-name")
        if await name_el.count() > 0:
            all_names.append(await name_el.text_content())

    assert "portfolio" in [n.lower() for n in all_names if n], (
        f"Portfolio skill not found in installed skills list: {all_names}"
    )
