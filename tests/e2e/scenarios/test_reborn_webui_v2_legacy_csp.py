"""Legacy CSP/browser-safety checks ported to Reborn WebUI v2.

The legacy gateway test guarded against inline handlers, CSP console errors,
and page-load JavaScript failures. Reborn's React shell has different DOM
controls, so this port exercises equivalent visible behavior on the real
``ironclaw-reborn serve`` surface.
"""

import json
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


async def test_reborn_legacy_csp_logs_controls_remain_functional(
    reborn_v2_page, reborn_v2_server
):
    """Port the legacy logs pause/clear button wiring check to Reborn logs."""
    request_count = 0

    async def handle_operator_logs(route):
        nonlocal request_count
        request_count += 1
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps(
                {
                    "status": "available",
                    "logs": {
                        "source": "in_memory_tracing",
                        "entries": [
                            {
                                "id": "csp-log-control",
                                "timestamp": "2026-06-12T10:11:12.123Z",
                                "level": "info",
                                "target": "ironclaw::ui::logs",
                                "message": "log entry for CSP control wiring",
                                "thread_id": "thread-csp-controls",
                            }
                        ],
                        "next_cursor": None,
                        "tail_supported": True,
                        "follow_supported": False,
                    },
                }
            ),
        )

    await reborn_v2_page.route("**/api/webchat/v2/operator/logs**", handle_operator_logs)
    await reborn_v2_page.goto(
        f"{reborn_v2_server}/v2/logs?thread_id=thread-csp-controls"
    )

    entry = reborn_v2_page.locator(SEL_V2["logs_entry"]).first
    await expect(entry.locator(SEL_V2["logs_entry_message"])).to_contain_text(
        "log entry for CSP control wiring",
        timeout=10000,
    )

    await reborn_v2_page.get_by_role("button", name="Pause").click()
    await expect(reborn_v2_page.get_by_role("button", name="Resume")).to_be_visible()
    paused_request_count = request_count
    await reborn_v2_page.wait_for_timeout(2200)
    assert request_count == paused_request_count

    reborn_v2_page.on("dialog", lambda dialog: dialog.accept())
    await reborn_v2_page.get_by_role("button", name="Clear").click()
    await expect(reborn_v2_page.locator(SEL_V2["logs_entry"])).to_have_count(0)
    await expect(reborn_v2_page.get_by_text("Waiting for log entries")).to_be_visible()
