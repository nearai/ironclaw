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
import re

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
    ".system/gateway/layout.json",
    ".system/gateway/widgets/skills-viewer/manifest.json",
    ".system/gateway/widgets/skills-viewer/index.js",
]


async def _wipe_customizations(base_url: str) -> None:
    """Clear any per-test customization files from the shared workspace.

    The session-scoped ``ironclaw_server`` fixture is shared across every
    test in the run, so anything we write into the workspace must be wiped
    before yielding back to the next test. ``memory_write`` accepts an empty
    body for non-layer paths, and the gateway's widget loader
    (``read_widget_manifest``) treats empty / unparseable widget manifests
    as "skip with a ``warn!`` log and continue" — no 500s, no index-page
    breakage — which is exactly the cleanup behavior we want without
    needing a real DELETE endpoint. The parse-failure warn lines are
    expected noise in the server log for the duration of this suite.
    """
    async with httpx.AsyncClient(timeout=10) as client:
        for path in _CUSTOM_PATHS:
            resp = await client.post(
                f"{base_url}/api/memory/write",
                headers=auth_headers(),
                json={"path": path, "content": "", "append": False},
            )
            # Surface cleanup failures immediately instead of letting a
            # silent auth/server error bleed leftover workspace state into
            # the next test and turn this suite flaky.
            assert resp.status_code == 200, (
                f"failed to wipe {path}: "
                f"status={resp.status_code} body={resp.text!r}"
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

    The mock LLM responds with one *or more* ``memory_write`` tool calls
    per trigger phrase (the customization patterns deliberately fan out
    into multiple parallel calls so the v2 engine multi-tool dispatch
    path gets exercised). The agent loop runs every dispatched tool,
    feeds the results back to the LLM, and the mock summarizes them as
    plain text — at which point the chat input is re-enabled and a fresh
    assistant message is in the DOM. We block on that terminal state so
    the next reload sees every workspace write.
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
    # 1. One chat turn fans out into *two* parallel ``memory_write`` tool
    #    calls (manifest + index.js). This intentionally exercises the
    #    multi-tool-call path of the v2 engine — pinning the test to a
    #    single call per turn would silently mask regressions in parallel
    #    dispatch / accumulator handling.
    await _drive_chat_customization(
        page, "customize: install skills viewer widget"
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


async def test_layout_hidden_built_in_tab_and_image_upload_disabled(
    browser, ironclaw_server, clean_customizations
):
    """Regression: layout.json flags must match the real DOM, not a hypothesis.

    Two ``app.js`` selector bugs slid through code review on PR #1725
    because the layout-config IIFE was written against a hypothetical DOM
    rather than the one ``static/index.html`` actually ships:

    1. ``tabs.hidden`` used the ``.tab-btn[data-tab="…"]`` selector, which
       only matched widget-injected buttons (created by ``_addWidgetTab``
       with ``className = 'tab-btn'``). Built-in tab ``<button>``\\s in
       ``index.html`` are plain ``<button data-tab="chat">`` etc. with no
       class, so hiding a built-in like ``"routines"`` silently no-opped.
    2. ``chat.image_upload === false`` tried to hide ``#image-upload-btn``,
       which doesn't exist — the real composer uses ``#attach-btn`` (the
       paperclip) and ``#image-file-input`` (the hidden file input).

    Both bugs share the same root cause: there was no e2e test that
    actually loaded a customized layout and asked the browser whether the
    flags took effect. This test is that missing coverage. The next
    instance of this class of bug — somebody adds a new layout flag,
    targets the wrong selector, and ships it — should fail this test
    instead of a user.

    Drives the layout via a direct ``/api/memory/write`` POST rather than
    through chat: the customization path is independent of the agent
    loop, and side-stepping chat keeps the test fast and decoupled from
    the mock LLM's canned-response set.
    """
    # 1. Write a layout.json that exercises both flags. `tabs.hidden`
    #    targets a *built-in* tab on purpose — the previous bug was that
    #    only widget-provided tabs could be hidden, so testing with a
    #    built-in is what catches the regression.
    layout = {
        "tabs": {"hidden": ["routines"]},
        "chat": {"image_upload": False},
    }
    async with httpx.AsyncClient(timeout=10) as client:
        resp = await client.post(
            f"{ironclaw_server}/api/memory/write",
            headers=auth_headers(),
            json={
                "path": ".system/gateway/layout.json",
                "content": json.dumps(layout),
                "append": False,
            },
        )
        assert resp.status_code == 200, (
            f"failed to write layout.json: "
            f"status={resp.status_code} body={resp.text!r}"
        )

    # 2. Reload in a fresh context so the gateway re-assembles the HTML
    #    bundle with `window.__IRONCLAW_LAYOUT__` injected from the new
    #    layout.json. The IIFE in `app.js` then applies the flags.
    context, pg = await _open_authed_page(browser, ironclaw_server)
    try:
        # Wait for the tab bar to render before probing selectors —
        # otherwise the layout IIFE may not have run yet on a slow CI
        # machine and we'd race the assertion.
        await pg.locator(".tab-bar").wait_for(state="visible", timeout=10000)

        # 3. `tabs.hidden: ["routines"]` must hide the built-in routines
        #    tab. Use `getComputedStyle` rather than reading the inline
        #    `style` attribute so the assertion survives a future refactor
        #    that swaps `style.display = 'none'` for a class toggle.
        routines_display = await pg.evaluate(
            """() => {
              const btn = document.querySelector(
                '.tab-bar button[data-tab=\"routines\"]'
              );
              return btn ? getComputedStyle(btn).display : 'missing';
            }"""
        )
        assert routines_display == "none", (
            f"built-in routines tab should be hidden by layout.tabs.hidden, "
            f"got display={routines_display!r}"
        )

        # 4. The other built-in tabs must NOT be collateral damage. If a
        #    future selector change started over-matching, this would
        #    catch it before users noticed.
        for visible_tab in ("chat", "memory", "settings"):
            display = await pg.evaluate(
                f"""() => {{
                  const btn = document.querySelector(
                    '.tab-bar button[data-tab=\"{visible_tab}\"]'
                  );
                  return btn ? getComputedStyle(btn).display : 'missing';
                }}"""
            )
            assert display != "none", (
                f"built-in tab {visible_tab!r} should still be visible, "
                f"got display={display!r}"
            )
            assert display != "missing", (
                f"built-in tab {visible_tab!r} disappeared from the DOM "
                "entirely — index.html structure regressed"
            )

        # 5. `chat.image_upload: false` must hide the visible attach button
        #    AND disable the underlying file input. Hiding only the button
        #    would leave a programmatic
        #    `document.getElementById('image-file-input').click()` path
        #    open for a widget or extension to bypass the operator's
        #    intent — the previous bug targeted a non-existent
        #    `#image-upload-btn` and accomplished neither.
        attach_state = await pg.evaluate(
            """() => {
              const btn = document.getElementById('attach-btn');
              const input = document.getElementById('image-file-input');
              return {
                attachDisplay: btn ? getComputedStyle(btn).display : 'missing',
                inputDisabled: input ? !!input.disabled : 'missing',
                inputExists: !!input,
              };
            }"""
        )
        assert attach_state["attachDisplay"] == "none", (
            f"#attach-btn should be hidden by chat.image_upload=false, "
            f"got {attach_state!r}"
        )
        assert attach_state["inputExists"], (
            "#image-file-input must exist in the DOM — index.html structure "
            "regressed"
        )
        assert attach_state["inputDisabled"] is True, (
            f"#image-file-input must be disabled by chat.image_upload=false, "
            f"got {attach_state!r}"
        )
    finally:
        await context.close()


async def test_customized_index_carries_csp_nonce_on_every_inline_script(
    ironclaw_server, clean_customizations
):
    """Regression: customized HTML must ship a per-response CSP nonce.

    The gateway's customization assembly path injects inline ``<script>``
    blocks (layout JSON + widget modules) into the HTML, which would be
    blocked by the static CSP unless every script carries a ``nonce=``
    attribute matching a ``Content-Security-Policy: ...'nonce-…'``
    header on the response. The mechanism is unit-tested in
    ``test_stamp_nonce_into_html_*`` (Rust) but there was no e2e proof
    that the full pipeline — workspace mutation → ``index_handler`` →
    nonce stamping → response header — actually wires up correctly under
    a real HTTP request.

    This is the missing test for that pipeline. The reviewer flagged
    "no CSP nonce verification" as a coverage gap in the
    ``feat/frontend-extension-system`` audit. Drives the request through
    ``httpx`` rather than Playwright because we need to read the
    response headers byte-for-byte (a browser ``EventSource`` /
    ``fetch`` won't expose all CSP-related headers, and Playwright's
    ``page.goto`` happens at the JS layer where the nonce has already
    been validated and consumed by the browser).
    """
    # 1. Write a layout that forces the customized HTML path. Branding
    #    title is enough — `layout_has_customizations` returns true on
    #    any non-empty title, which routes index_handler through
    #    `build_frontend_html` and the nonce stamping path.
    layout = {"branding": {"title": "Acme AI"}}
    async with httpx.AsyncClient(timeout=10) as client:
        write = await client.post(
            f"{ironclaw_server}/api/memory/write",
            headers=auth_headers(),
            json={
                "path": ".system/gateway/layout.json",
                "content": json.dumps(layout),
                "append": False,
            },
        )
        assert write.status_code == 200, (
            f"failed to write layout.json: "
            f"status={write.status_code} body={write.text!r}"
        )

        # 2. Hit `/` directly. The bootstrap route is unauthenticated,
        #    so no token is required — but we send one anyway to mirror
        #    a real browser load (the browser fires the auth screen
        #    after the HTML lands).
        resp = await client.get(
            f"{ironclaw_server}/?token={AUTH_TOKEN}",
            headers=auth_headers(),
        )
    assert resp.status_code == 200, resp.text

    # 3. Contract A: response carries a per-response CSP header with a
    #    `'nonce-...'` source in `script-src`. The static CSP layer
    #    emits a different header (no nonce); a customized response
    #    must override it.
    csp = resp.headers.get("content-security-policy")
    assert csp is not None, (
        f"customized index must emit Content-Security-Policy header; got headers={dict(resp.headers)}"
    )
    nonce_match = re.search(r"'nonce-([0-9a-f]+)'", csp)
    assert nonce_match is not None, (
        f"CSP must include a 'nonce-...' source in script-src, got: {csp}"
    )
    nonce = nonce_match.group(1)
    # The nonce is generated as 16 random bytes hex-encoded → 32 chars.
    # Pin the length so a future regression that drops to 8 bytes (or
    # accidentally truncates) fails here with an actionable diff.
    assert len(nonce) == 32, (
        f"CSP nonce must be 32 hex chars (16 random bytes), got {len(nonce)}: {nonce}"
    )

    body = resp.text

    # 4. Contract B: every *inline* injected `<script>` block carries the
    #    same nonce attribute. The base `static/index.html` ships
    #    several `<script src="...">` tags (i18n bundles, theme-init,
    #    app.js, marked, DOMPurify) — those are external scripts
    #    authorized by `script-src 'self' <CDNs>` in the gateway CSP and
    #    deliberately do NOT carry a nonce. Only the inline blocks that
    #    `assemble_index` injects (layout JSON island + widget modules)
    #    need to be nonce-gated. Filtering on the absence of `src=` is
    #    the cleanest way to separate the two — a future regression
    #    that drops the nonce off an inline script still fails this
    #    check, but a baseline `<script src=...>` no longer trips it.
    #
    #    Skip the inline branding `<style>` blocks — those don't need a
    #    nonce because the gateway's CSP allows `'unsafe-inline'` for
    #    `style-src`. The Rust unit test
    #    `test_assemble_index_widget_style_has_no_nonce` pins that
    #    decision; this e2e check covers the request-path side too.
    expected_attr = f'nonce="{nonce}"'
    all_script_tags = re.findall(r"<script\b[^>]*>", body)
    inline_script_tags = [
        tag for tag in all_script_tags if not re.search(r"\bsrc\s*=", tag)
    ]
    assert inline_script_tags, (
        "customized HTML must contain at least one inline <script> tag "
        "(the layout JSON island injected by assemble_index). If this "
        "assertion fires, the customization assembly path stopped emitting "
        "inline scripts entirely. "
        f"All <script> tags seen: {all_script_tags!r}"
    )
    for tag in inline_script_tags:
        assert expected_attr in tag, (
            f"every inline <script> must carry nonce attribute matching the "
            f"response CSP nonce {nonce!r}; tag without match: {tag!r}"
        )

    # 5. Contract C: the placeholder sentinel must be gone — if a
    #    future regression breaks the substitution (e.g. switches the
    #    helper to a no-op), the placeholder would still be in the
    #    body and the browser would reject every script as
    #    nonce-mismatch. Catching this here gives a clearer
    #    diagnostic than "blank page in Chrome".
    assert "__IRONCLAW_CSP_NONCE__" not in body, (
        "NONCE_PLACEHOLDER sentinel must be substituted before serving — "
        "found unmodified placeholder in response body"
    )
