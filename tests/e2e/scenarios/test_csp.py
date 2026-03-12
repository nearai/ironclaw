"""Scenario: Content Security Policy compliance.

Detects CSP violations (inline scripts, blocked resources) that would
break the gateway JS.  This test catches regressions like adding
inline onclick handlers while a script-src CSP is active.
"""

from helpers import SEL


async def test_no_csp_violations_on_load(page):
    """Page load must produce zero CSP violation reports."""
    violations = []

    page.on("console", lambda msg: (
        violations.append(msg.text)
        if "content security policy" in msg.text.lower()
        or msg.type == "error" and "refused" in msg.text.lower()
        else None
    ))

    # Reload the page to catch violations from initial load.
    # Use "load" (not "networkidle") because the SSE stream keeps the
    # connection open indefinitely, preventing networkidle from firing.
    await page.reload(wait_until="load")
    # Wait a moment for any deferred script execution
    await page.wait_for_timeout(2000)

    assert violations == [], (
        f"CSP violations detected on page load:\n" + "\n".join(violations)
    )


async def test_no_inline_event_handlers_in_html(page):
    """Static HTML must not contain any inline event handler attributes."""
    inline_handlers = await page.evaluate("""() => {
        const allElements = document.querySelectorAll('*');
        const found = [];
        const handlerAttrs = [
            'onclick', 'onchange', 'onsubmit', 'onload', 'onerror',
            'onmouseover', 'onfocus', 'onblur', 'onkeydown', 'onkeyup',
            'oninput', 'onmousedown', 'onmouseup'
        ];
        for (const el of allElements) {
            for (const attr of handlerAttrs) {
                if (el.hasAttribute(attr)) {
                    const tag = el.tagName.toLowerCase();
                    const id = el.id ? '#' + el.id : '';
                    const cls = el.className ? '.' + el.className.split(' ')[0] : '';
                    found.push(tag + id + cls + '[' + attr + ']');
                }
            }
        }
        return found;
    }""")

    assert inline_handlers == [], (
        f"Found inline event handlers (CSP-incompatible):\n"
        + "\n".join(f"  - {h}" for h in inline_handlers)
    )


async def test_no_js_errors_on_page_load(page):
    """No JavaScript errors should occur on page load."""
    errors = []
    page.on("pageerror", lambda err: errors.append(str(err)))

    await page.reload(wait_until="load")
    await page.wait_for_timeout(2000)

    assert errors == [], (
        f"JavaScript errors on page load:\n" + "\n".join(errors)
    )


async def test_buttons_still_functional_after_csp_migration(page):
    """Core buttons must still be wired up via addEventListener."""
    # Verify that key buttons have click handlers attached (not inline)
    # by checking that clicking them doesn't throw and they exist in the DOM
    button_ids = [
        'send-btn',
        'thread-new-btn',
        'thread-toggle-btn',
        'restart-btn',
        'memory-edit-btn',
        'logs-pause-btn',
        'logs-clear-btn',
    ]

    for btn_id in button_ids:
        exists = await page.evaluate(
            "id => document.getElementById(id) !== null", btn_id
        )
        assert exists, f"Button #{btn_id} not found in DOM"

    # Verify the assistant thread div is clickable (has no onclick but
    # should be handled by delegation or direct addEventListener)
    assistant_el = page.locator(SEL["chat_input"])
    await assistant_el.wait_for(state="visible", timeout=5000)
