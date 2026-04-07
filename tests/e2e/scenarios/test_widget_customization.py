"""Frontend customization-via-chat scenarios for the widget extension system.

These tests exercise the workflow shipped in PR #1725: the user talks to the
agent in chat, the agent issues ``memory_write`` tool calls into the workspace
under ``.system/gateway/``, and on the next page load the gateway picks up the
new layout / widgets and serves a customized HTML bundle.

Two flows are covered:

1. **Tab bar to left side panel** — the agent writes
   ``.system/gateway/custom.css`` to flip the tab bar from a horizontal top
   strip into a vertical left-hand panel, and the test asserts the new layout
   is reflected in the live DOM (computed style + appended ``custom.css``).

2. **Workspace-data widget** — the agent writes a manifest + an
   ``index.js`` for a "Skills" widget that pulls workspace skills from
   ``/api/skills`` via ``IronClaw.api.fetch`` and renders them in a rich,
   editable list. The test asserts a new tab button appears, switches to it,
   and verifies the widget actually rendered into a panel marked with a
   stable ``data-testid``.

Both flows drive the agent through chat triggers defined in
``mock_llm.py::TOOL_CALL_PATTERNS`` (look for ``customize:`` prefixes).
"""

import json

import httpx
import pytest

from helpers import (
    AUTH_TOKEN,
    SEL,
    auth_headers,
    send_chat_and_wait_for_terminal_message,
)


# All gateway customization state lives under this prefix in the workspace.
_CUSTOM_PATHS = [
    ".system/gateway/custom.css",
    ".system/gateway/widgets/skills-viewer/manifest.json",
    ".system/gateway/widgets/skills-viewer/index.js",
]


async def _wipe_customizations(base_url: str) -> None:
    """Clear any per-test customization files from the shared workspace.

    The session-scoped ``ironclaw_server`` fixture is shared across every
    test in the run, so anything we write into the workspace must be wiped
    before yielding back to the next test. ``memory_write`` accepts an empty
    body for non-layer paths, and the gateway treats empty / unparseable
    widget files as "skip silently", which is exactly the cleanup behavior
    we want without needing a real DELETE endpoint.
    """
    async with httpx.AsyncClient(timeout=10) as client:
        for path in _CUSTOM_PATHS:
            await client.post(
                f"{base_url}/api/memory/write",
                headers=auth_headers(),
                json={"path": path, "content": "", "append": False},
            )


@pytest.fixture
async def clean_customizations(ironclaw_server):
    """Wipe layout/widget files before *and* after each test in this module."""
    await _wipe_customizations(ironclaw_server)
    yield
    await _wipe_customizations(ironclaw_server)


async def _open_authed_page(browser, base_url: str):
    """Open a fresh authenticated page and wait for the auth screen to clear.

    Mirrors the session-scoped ``page`` fixture but lets us re-open the page
    after a chat-driven workspace mutation so the gateway re-assembles the
    HTML with the new layout / widgets.
    """
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    pg = await context.new_page()
    await pg.goto(f"{base_url}/?token={AUTH_TOKEN}")
    await pg.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    return context, pg


async def _drive_chat_customization(page, prompt: str) -> None:
    """Send a customization prompt and wait for the agent to finish the turn.

    The mock LLM responds with one ``memory_write`` tool call per trigger
    phrase. The agent loop dispatches the tool, gets a result, and the mock
    LLM then summarizes it as plain text — at which point the chat input
    is re-enabled and a fresh assistant message is in the DOM. We block on
    that terminal state so the next reload sees the workspace write.
    """
    result = await send_chat_and_wait_for_terminal_message(
        page,
        prompt,
        timeout=30000,
    )
    # The summary text is "The memory_write tool returned: ..." (mock LLM
    # default tool-result fallback). Either an assistant or system terminal
    # message is acceptable — what we care about is that the turn settled.
    assert result["role"] in ("assistant", "system"), result


async def test_chat_moves_tab_bar_to_left_panel(
    page, browser, ironclaw_server, clean_customizations
):
    """User asks the agent to move the top tab bar into a left side panel.

    The agent writes ``.system/gateway/custom.css`` via ``memory_write``;
    the gateway appends that file onto ``/style.css`` on the next request,
    so reloading the page must show the tab bar laid out vertically.
    """
    # 1. Drive the customization through chat. The mock LLM matches the
    #    `customize: move tab bar to left` trigger and emits a memory_write
    #    tool call targeting `.system/gateway/custom.css`.
    await _drive_chat_customization(page, "customize: move tab bar to left")

    # 2. Sanity check: the workspace file actually landed where the gateway
    #    will look for it. Reading via the API both confirms the write and
    #    bypasses any client-side caching of the chat tab.
    async with httpx.AsyncClient(timeout=10) as client:
        resp = await client.get(
            f"{ironclaw_server}/api/memory/read",
            headers=auth_headers(),
            params={"path": ".system/gateway/custom.css"},
        )
        assert resp.status_code == 200, resp.text
        body = resp.json()
        # MemoryReadResponse uses a `content` field.
        assert "tab bar to left side panel" in body.get("content", ""), body

    # 3. Re-open the gateway in a fresh browser context. The gateway's
    #    `css_handler` will append the workspace's `custom.css` onto the
    #    embedded base stylesheet, so the reload picks up the new layout.
    context, pg = await _open_authed_page(browser, ironclaw_server)
    try:
        await pg.locator(".tab-bar").wait_for(state="visible", timeout=10000)

        # 3a. The served stylesheet must contain our overlay. This catches
        #     regressions in custom.css plumbing even if the browser would
        #     otherwise lay out the tab bar identically by accident.
        async with httpx.AsyncClient(timeout=10) as client:
            css_resp = await client.get(
                f"{ironclaw_server}/style.css",
                headers=auth_headers(),
            )
            assert css_resp.status_code == 200
            assert "tab bar to left side panel" in css_resp.text
            assert "flex-direction: column" in css_resp.text

        # 3b. The browser must actually render the tab bar vertically. Use
        #     getComputedStyle so we cover both the rule application *and*
        #     CSS specificity (the !important override beating the base
        #     `.tab-bar` rule).
        flex_direction = await pg.evaluate(
            "() => getComputedStyle(document.querySelector('.tab-bar')).flexDirection"
        )
        assert flex_direction == "column", (
            f"Expected tab bar flex-direction=column after customization, "
            f"got {flex_direction!r}"
        )

        # 3c. The tab bar should now span the full viewport height (left
        #     side panel) instead of sitting as a thin top strip. The exact
        #     px width depends on viewport math; assert it grew to ~the
        #     220px we set in custom.css and is taller than it is wide.
        size = await pg.evaluate(
            "() => { const r = document.querySelector('.tab-bar').getBoundingClientRect();"
            "  return { width: r.width, height: r.height }; }"
        )
        assert size["width"] >= 200, size
        assert size["height"] > size["width"], size

        # 3d. The built-in tabs are still present (we only restyled the bar,
        #     we did not remove anything).
        for tab_id in ("chat", "memory", "settings"):
            btn = pg.locator(f'.tab-bar button[data-tab="{tab_id}"]')
            assert await btn.count() == 1, f"missing built-in tab {tab_id!r}"
    finally:
        await context.close()


async def test_chat_adds_skills_viewer_widget_to_top_panel(
    page, browser, ironclaw_server, clean_customizations
):
    """User asks the agent to add a Skills widget to the top tab bar.

    The agent writes a widget manifest and an ``index.js`` implementation
    into ``.system/gateway/widgets/skills-viewer/``. On the next reload the
    gateway resolves the widget, inlines its module script (with a CSP
    nonce), and the runtime auto-mounts it as a new tab via
    ``IronClaw.registerWidget({ slot: 'tab', ... })``. The widget then
    fetches workspace skills from ``/api/skills`` and renders them.
    """
    # 1. Two chat turns — one for the manifest, one for the implementation.
    #    The mock LLM matches each trigger with a single memory_write call;
    #    splitting the work this way keeps every turn within the
    #    one-tool-per-response shape that mock_llm currently supports.
    await _drive_chat_customization(
        page, "customize: create skills viewer manifest"
    )
    await _drive_chat_customization(
        page, "customize: install skills viewer code"
    )

    # 2. Confirm both files actually landed in the workspace.
    async with httpx.AsyncClient(timeout=10) as client:
        manifest_resp = await client.get(
            f"{ironclaw_server}/api/memory/read",
            headers=auth_headers(),
            params={
                "path": ".system/gateway/widgets/skills-viewer/manifest.json",
            },
        )
        assert manifest_resp.status_code == 200, manifest_resp.text
        manifest_doc = manifest_resp.json()
        manifest = json.loads(manifest_doc["content"])
        assert manifest["id"] == "skills-viewer"
        assert manifest["slot"] == "tab"

        index_resp = await client.get(
            f"{ironclaw_server}/api/memory/read",
            headers=auth_headers(),
            params={
                "path": ".system/gateway/widgets/skills-viewer/index.js",
            },
        )
        assert index_resp.status_code == 200, index_resp.text
        assert "registerWidget" in index_resp.json()["content"]

        # 2a. The widgets API should now report the new widget. This is the
        #     gateway's own discovery path — it walks the workspace dir and
        #     parses each manifest.json — so it doubles as an integration
        #     check on the FrontendBundle assembler.
        widgets_resp = await client.get(
            f"{ironclaw_server}/api/frontend/widgets",
            headers=auth_headers(),
        )
        assert widgets_resp.status_code == 200, widgets_resp.text
        widget_ids = {w["id"] for w in widgets_resp.json()}
        assert "skills-viewer" in widget_ids, widget_ids

    # 3. Reload in a fresh context — the gateway will assemble a new HTML
    #    bundle that injects the widget JS as a CSP-noncedinline module.
    context, pg = await _open_authed_page(browser, ironclaw_server)
    try:
        # 3a. The runtime must have added a tab button for the widget. Use
        #     a generous timeout because widget mounting happens after the
        #     ES module loads, which is post-DOMContentLoaded.
        widget_tab_btn = pg.locator(
            '.tab-bar button[data-tab="skills-viewer"]'
        )
        await widget_tab_btn.wait_for(state="visible", timeout=15000)
        assert (await widget_tab_btn.text_content() or "").strip() == "Skills"

        # 3b. Activate the widget tab and wait for the widget's own root to
        #     show up. The widget JS sets `data-testid="skills-viewer-root"`
        #     on the container as its very first action, so this fires
        #     before the asynchronous /api/skills fetch resolves.
        await widget_tab_btn.click()
        root = pg.locator('[data-testid="skills-viewer-root"]')
        await root.wait_for(state="visible", timeout=10000)
        title = pg.locator('[data-testid="skills-viewer-title"]')
        assert (await title.text_content() or "").strip() == "Workspace Skills"

        # 3c. The list area must resolve into either an empty-state marker
        #     or one or more skill cards — *not* the loading placeholder
        #     and *not* the error path. We don't pin the exact set of
        #     skills because the e2e workspace ships with whatever the
        #     embedded registry seeds, but we do guarantee the widget
        #     successfully talked to /api/skills via IronClaw.api.fetch.
        await pg.wait_for_function(
            """() => {
              const root = document.querySelector('[data-testid=\"skills-viewer-root\"]');
              if (!root) return false;
              if (root.querySelector('[data-testid=\"skills-viewer-error\"]')) return 'error';
              if (root.querySelector('[data-testid=\"skills-viewer-empty\"]')) return true;
              return root.querySelectorAll('[data-testid=\"skills-viewer-card\"]').length > 0;
            }""",
            timeout=10000,
        )
        # Surface a clearer failure if the widget hit the /api/skills error
        # branch — this means the auth wrapper or the endpoint regressed.
        error_count = await pg.locator(
            '[data-testid="skills-viewer-error"]'
        ).count()
        assert error_count == 0, "skills-viewer widget failed to fetch /api/skills"

        # 3d. The widget container is mounted *inside* `.tab-content` with
        #     `data-widget="skills-viewer"`, which is the contract the
        #     gateway runtime exposes for CSS scoping. Verifying the
        #     attribute makes sure widgets ride the same isolation path
        #     even when they don't ship a style.css.
        widget_root_attr = await pg.evaluate(
            """() => {
              const el = document.querySelector('#tab-skills-viewer');
              return el && el.getAttribute('data-widget');
            }"""
        )
        assert widget_root_attr == "skills-viewer", widget_root_attr
    finally:
        await context.close()
