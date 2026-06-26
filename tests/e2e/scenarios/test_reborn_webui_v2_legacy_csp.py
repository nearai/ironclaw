"""Legacy CSP/browser-safety checks ported to Reborn WebUI v2.

The legacy gateway test guarded against inline handlers, CSP console errors,
and page-load JavaScript failures. Reborn's React shell has different DOM
controls, so this port exercises equivalent visible behavior on the real
``ironclaw-reborn serve`` surface.
"""

import re

from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2
from reborn_webui_harness import (
    reborn_v2_browser,  # noqa: F401 - imported fixture
    reborn_v2_page,  # noqa: F401 - imported fixture
    reborn_v2_server,  # noqa: F401 - imported fixture
)


def _record_csp_console_errors(page, violations):
    def handle_console(msg):
        text = msg.text
        lower = text.lower()
        if "content security policy" in lower or (
            msg.type == "error" and "refused" in lower
        ):
            violations.append(text)

    page.on("console", handle_console)


async def test_reborn_legacy_csp_no_violations_on_load(reborn_v2_page):
    """Port of legacy CSP console violation check to the Reborn shell."""
    violations = []
    _record_csp_console_errors(reborn_v2_page, violations)

    await reborn_v2_page.reload(wait_until="load")
    await expect(reborn_v2_page.locator(SEL_V2["chat_composer"])).to_be_visible(
        timeout=15000
    )
    await reborn_v2_page.wait_for_timeout(2000)

    assert violations == [], (
        "CSP violations detected on Reborn page load:\n" + "\n".join(violations)
    )


async def test_reborn_legacy_csp_no_inline_event_handlers(reborn_v2_page):
    """Port of legacy inline-event-handler scan to Reborn's rendered DOM."""
    inline_handlers = await reborn_v2_page.evaluate(
        """() => {
            const handlerAttrs = [
                "onclick", "onchange", "onsubmit", "onload", "onerror",
                "onmouseover", "onfocus", "onblur", "onkeydown", "onkeyup",
                "oninput", "onmousedown", "onmouseup"
            ];
            const found = [];
            for (const el of document.querySelectorAll("*")) {
                for (const attr of handlerAttrs) {
                    if (!el.hasAttribute(attr)) continue;
                    const tag = el.tagName.toLowerCase();
                    const id = el.id ? "#" + el.id : "";
                    const classes = typeof el.className === "string" && el.className
                        ? "." + el.className.split(" ")[0]
                        : "";
                    found.push(`${tag}${id}${classes}[${attr}]`);
                }
            }
            return found;
        }"""
    )

    assert inline_handlers == [], (
        "Found inline event handlers in Reborn DOM:\n"
        + "\n".join(f"  - {handler}" for handler in inline_handlers)
    )


async def test_reborn_legacy_csp_no_js_errors_on_page_load(
    reborn_v2_server, reborn_v2_browser
):
    """Port of legacy pageerror guard to a fresh authenticated Reborn page."""
    errors = []
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    page.on("pageerror", lambda err: errors.append(str(err)))

    try:
        await page.goto(f"{reborn_v2_server}/v2/?token={REBORN_V2_AUTH_TOKEN}")
        await expect(page.locator(SEL_V2["chat_composer"])).to_be_visible(timeout=15000)
        await page.wait_for_timeout(2000)
    finally:
        await context.close()

    assert errors == [], "JavaScript errors on Reborn page load:\n" + "\n".join(errors)


async def test_reborn_legacy_csp_core_controls_remain_functional(reborn_v2_page):
    """Port of legacy button wiring check to Reborn's visible controls."""
    sidebar = reborn_v2_page.locator(SEL_V2["sidebar"])
    toggle = reborn_v2_page.locator(SEL_V2["sidebar_toggle"])
    composer = reborn_v2_page.locator(SEL_V2["chat_composer"])

    await expect(sidebar).to_be_visible(timeout=15000)
    await expect(composer).to_be_visible(timeout=15000)

    await toggle.click()
    await expect(sidebar).to_be_hidden(timeout=5000)
    await toggle.click()
    await expect(sidebar).to_be_visible(timeout=5000)

    await reborn_v2_page.get_by_role("link", name=re.compile("^Settings$")).click()
    await expect(reborn_v2_page).to_have_url(re.compile(".*/settings.*"), timeout=10000)
    search = reborn_v2_page.get_by_placeholder("Search settings")
    await search.fill("tools")
    await expect(search).to_have_value("tools")

    await reborn_v2_page.get_by_role("button", name="Clear search").click()
    await expect(search).to_have_value("")

    await sidebar.get_by_role("button", name=re.compile("^New$")).click()
    await expect(reborn_v2_page).to_have_url(re.compile(".*/chat.*"), timeout=10000)
    await expect(composer).to_be_visible(timeout=15000)
