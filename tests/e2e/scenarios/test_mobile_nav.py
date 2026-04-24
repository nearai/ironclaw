"""Mobile hamburger drawer (nearai/ironclaw#1344).

Covers the ≤768px layout shipped in `feat/mobile-hamburger-drawer`:
- mobile header + hamburger-activated drawer replace the desktop tab bar
- thread sidebar and settings subtab sidebar render through the drawer,
  which depends on CSS specificity that has historically been lost to
  surface stylesheets concatenated after base.css (the regression class
  this test is designed to catch)
"""

import pytest
from helpers import AUTH_TOKEN, SEL

MOBILE_VIEWPORT = {"width": 375, "height": 667}
DESKTOP_VIEWPORT = {"width": 1280, "height": 720}


async def _open_page(browser, ironclaw_server, viewport):
    context = await browser.new_context(viewport=viewport)
    pg = await context.new_page()
    await pg.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}", timeout=15000)
    await pg.locator(SEL["auth_screen"]).wait_for(state="hidden", timeout=15000)
    await pg.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )
    return context, pg


@pytest.fixture
async def mobile_page(browser, ironclaw_server):
    context, pg = await _open_page(browser, ironclaw_server, MOBILE_VIEWPORT)
    yield pg
    await context.close()


async def test_mobile_chrome_visible_desktop_chrome_hidden(mobile_page):
    """At ≤768px the mobile header renders and the desktop tab bar / thread sidebar are hidden.

    The thread-sidebar assertion is the regression for the chat.css
    specificity trap — an unconditional `.thread-sidebar { display: flex }`
    in chat.css beats base.css's plain mobile `.thread-sidebar { display: none }`
    on source order, which is why the override is scoped as
    `#tab-chat .thread-sidebar`.
    """
    await mobile_page.locator(".mobile-header").wait_for(state="visible", timeout=5000)

    tab_bar_visible = await mobile_page.locator(".tab-bar").is_visible()
    assert not tab_bar_visible, "desktop .tab-bar must be hidden at mobile viewport"

    thread_sidebar_visible = await mobile_page.locator("#thread-sidebar").is_visible()
    assert not thread_sidebar_visible, (
        "#thread-sidebar must be hidden at mobile viewport — if this fails, chat.css's "
        "unconditional `.thread-sidebar { display: flex }` is beating the mobile override"
    )


async def test_hamburger_toggles_drawer(mobile_page):
    """Tapping the hamburger opens the drawer; tapping the backdrop closes it.

    Drives the click handlers in mobile-menu.js end-to-end — the drawer's
    `.open` class is what the CSS animates, so asserting it on both sides
    of a toggle covers the full open/close cycle.
    """
    hamburger = mobile_page.locator("#hamburger-btn")
    menu = mobile_page.locator("#mobile-menu")
    backdrop = mobile_page.locator("#mobile-menu-backdrop")

    # Closed initially.
    assert await hamburger.get_attribute("aria-expanded") == "false"
    assert "open" not in (await menu.get_attribute("class") or "")

    # Open via hamburger tap.
    await hamburger.click()
    await mobile_page.wait_for_function(
        "() => document.getElementById('mobile-menu').classList.contains('open')",
        timeout=2000,
    )
    assert await hamburger.get_attribute("aria-expanded") == "true"
    assert await menu.get_attribute("aria-hidden") == "false"

    # Close via backdrop tap.
    await backdrop.click()
    await mobile_page.wait_for_function(
        "() => !document.getElementById('mobile-menu').classList.contains('open')",
        timeout=2000,
    )
    assert await hamburger.get_attribute("aria-expanded") == "false"


async def test_drawer_nav_switches_tab_and_closes(mobile_page):
    """Tapping a nav item in the drawer routes through the original tab button.

    This is the contract that keeps the desktop DOM authoritative: the
    cloned mobile nav button dispatches `.click()` on the source, so
    switchTab() runs unchanged. Regression guard: if the cloning loses
    the source binding, the hash will not update and the panel will not
    activate.
    """
    await mobile_page.locator("#hamburger-btn").click()
    await mobile_page.wait_for_function(
        "() => document.getElementById('mobile-menu').classList.contains('open')",
        timeout=2000,
    )

    # Drawer populated from the desktop tab-bar — find Settings by data-tab.
    nav_settings = mobile_page.locator('#mobile-menu-nav button[data-tab="settings"]')
    await nav_settings.wait_for(state="visible", timeout=3000)
    await nav_settings.click()

    # Tab switched + drawer closed.
    await mobile_page.locator(SEL["tab_panel"].format(tab="settings")).wait_for(
        state="visible", timeout=3000
    )
    await mobile_page.wait_for_function(
        "() => !document.getElementById('mobile-menu').classList.contains('open')",
        timeout=2000,
    )
    current_tab = await mobile_page.evaluate("() => currentTab")
    assert current_tab == "settings"


async def test_settings_sidebar_is_fullwidth_on_mobile(mobile_page):
    """Settings subtab list renders at the tab-panel width, not the desktop 180px.

    Regression for the settings.css specificity trap — mirrors the
    chat.css one. Without the `#tab-settings`-scoped mobile overrides,
    `.settings-sidebar` keeps its unconditional desktop width:180px and
    the mobile drill-down secondary menu looks broken.
    """
    # Navigate to Settings via the drawer.
    await mobile_page.locator("#hamburger-btn").click()
    await mobile_page.locator('#mobile-menu-nav button[data-tab="settings"]').click()
    await mobile_page.locator(SEL["tab_panel"].format(tab="settings")).wait_for(
        state="visible", timeout=3000
    )

    sidebar = mobile_page.locator("#tab-settings .settings-sidebar")
    await sidebar.wait_for(state="visible", timeout=3000)

    sidebar_width, panel_width = await mobile_page.evaluate(
        """() => {
            const sidebar = document.querySelector('#tab-settings .settings-sidebar');
            const panel = document.getElementById('tab-settings');
            return [sidebar.getBoundingClientRect().width,
                    panel.getBoundingClientRect().width];
        }"""
    )
    # Full-width within 1px rounding; any value near 180 means desktop
    # styling leaked through.
    assert abs(sidebar_width - panel_width) < 2, (
        f"settings-sidebar width {sidebar_width}px should match tab-panel "
        f"width {panel_width}px on mobile (desktop leakage produces ~180px)"
    )


async def test_settings_subtab_drill_down(mobile_page):
    """Tapping a settings subtab on mobile drills into detail view and hides the sidebar.

    Exercises the `.settings-layout.settings-detail-active` toggle in
    switchSettingsSubtab(). The drill-down hides the sidebar and reveals
    the content + back button — the mobile UX that depends on both the
    JS handler and the CSS specificity fix.
    """
    await mobile_page.locator("#hamburger-btn").click()
    await mobile_page.locator('#mobile-menu-nav button[data-tab="settings"]').click()
    await mobile_page.locator("#tab-settings .settings-sidebar").wait_for(
        state="visible", timeout=3000
    )

    # Tap a subtab — use Inference, the default-active one.
    await mobile_page.locator(SEL["settings_subtab"].format(subtab="inference")).click()

    # Layout enters drill-down mode; sidebar hides, content shows.
    await mobile_page.wait_for_function(
        """() => document.querySelector('.settings-layout')
            .classList.contains('settings-detail-active')""",
        timeout=3000,
    )
    sidebar_visible = await mobile_page.locator("#tab-settings .settings-sidebar").is_visible()
    assert not sidebar_visible, "settings-sidebar must hide in drill-down view"

    back_btn_visible = await mobile_page.locator(".settings-back-btn").is_visible()
    assert back_btn_visible, "back button must appear in drill-down view"


async def test_desktop_viewport_unchanged(browser, ironclaw_server):
    """At >768px the mobile header is hidden and the desktop tab bar renders.

    Guards against a mobile CSS rule accidentally leaking onto desktop.
    """
    context, pg = await _open_page(browser, ironclaw_server, DESKTOP_VIEWPORT)
    try:
        header_visible = await pg.locator(".mobile-header").is_visible()
        assert not header_visible, ".mobile-header must be hidden on desktop"

        tab_bar_visible = await pg.locator(".tab-bar").is_visible()
        assert tab_bar_visible, ".tab-bar must be visible on desktop"

        thread_sidebar_visible = await pg.locator("#thread-sidebar").is_visible()
        assert thread_sidebar_visible, "#thread-sidebar must be visible on desktop"
    finally:
        await context.close()
