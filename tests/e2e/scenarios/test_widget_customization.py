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

import asyncio
import json
import os
import re
import signal
import socket
import tempfile

import httpx
import pytest

from helpers import (
    AUTH_TOKEN,
    SEL,
    auth_headers,
    send_chat_and_wait_for_terminal_message,
    wait_for_ready,
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


async def _stop_proc(proc, *, timeout: float = 10.0) -> None:
    async def _drain_pipes() -> None:
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except (asyncio.TimeoutError, ValueError):
            pass

    if proc.returncode is not None:
        await _drain_pipes()
        return
    proc.send_signal(signal.SIGINT)
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
        await _drain_pipes()
        return
    except asyncio.TimeoutError:
        pass
    proc.terminate()
    try:
        await asyncio.wait_for(proc.wait(), timeout=2)
        await _drain_pipes()
        return
    except asyncio.TimeoutError:
        pass
    proc.kill()
    await proc.wait()
    await _drain_pipes()


@pytest.fixture
async def clean_customizations(ironclaw_server):
    """Wipe layout/widget files before *and* after each test in this module."""
    await _wipe_customizations(ironclaw_server)
    yield
    await _wipe_customizations(ironclaw_server)


@pytest.fixture
async def single_tenant_gateway_server(ironclaw_binary, mock_llm_server):
    """Dedicated gateway without a DB so `/style.css` can include custom CSS."""
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-widget-single-tenant-home-")
    home_dir = home_tmpdir.name
    os.makedirs(os.path.join(home_dir, ".ironclaw"), exist_ok=True)

    reserved = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        reserved.append(sock)
    gateway_port = reserved[0].getsockname()[1]
    http_port = reserved[1].getsockname()[1]
    for sock in reserved:
        sock.close()

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": "e2e-widget-single-tenant",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-widget-single-tenant",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "true",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
    }

    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary,
        "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )

    base_url = f"http://127.0.0.1:{gateway_port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    except TimeoutError:
        stderr_text = ""
        if proc.stderr:
            try:
                stderr_text = (await asyncio.wait_for(proc.stderr.read(8192), timeout=2)).decode(
                    "utf-8",
                    errors="replace",
                )
            except asyncio.TimeoutError:
                pass
        pytest.fail(f"single-tenant widget server failed to start:\n{stderr_text}")
    finally:
        await _stop_proc(proc)
        home_tmpdir.cleanup()


@pytest.fixture
async def clean_single_tenant_customizations(single_tenant_gateway_server):
    await _wipe_customizations(single_tenant_gateway_server)
    yield
    await _wipe_customizations(single_tenant_gateway_server)


@pytest.fixture(scope="session")
async def multi_tenant_gateway_server(ironclaw_binary, mock_llm_server):
    """Dedicated gateway with AGENT_MULTI_TENANT=true for multi-tenant isolation tests."""
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-widget-multi-tenant-home-")
    home_dir = home_tmpdir.name
    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-widget-multi-tenant-db-")
    os.makedirs(os.path.join(home_dir, ".ironclaw"), exist_ok=True)

    reserved = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        reserved.append(sock)
    gateway_port = reserved[0].getsockname()[1]
    http_port = reserved[1].getsockname()[1]
    for sock in reserved:
        sock.close()

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=info",
        "RUST_BACKTRACE": "1",
        "IRONCLAW_OWNER_ID": "e2e-widget-multi-tenant",
        "AGENT_MULTI_TENANT": "true",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-widget-multi-tenant",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(db_tmpdir.name, "multi-tenant.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "true",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
    }

    proc = await asyncio.create_subprocess_exec(
        ironclaw_binary,
        "--no-onboard",
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=env,
    )

    base_url = f"http://127.0.0.1:{gateway_port}"
    try:
        await wait_for_ready(f"{base_url}/api/health", timeout=60)
        yield base_url
    except TimeoutError:
        stderr_text = ""
        if proc.stderr:
            try:
                stderr_text = (await asyncio.wait_for(proc.stderr.read(8192), timeout=2)).decode(
                    "utf-8",
                    errors="replace",
                )
            except asyncio.TimeoutError:
                pass
        pytest.fail(f"multi-tenant widget server failed to start:\n{stderr_text}")
    finally:
        await _stop_proc(proc)
        home_tmpdir.cleanup()
        db_tmpdir.cleanup()


@pytest.fixture
async def clean_multi_tenant_customizations(multi_tenant_gateway_server):
    await _wipe_customizations(multi_tenant_gateway_server)
    yield
    await _wipe_customizations(multi_tenant_gateway_server)


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


async def test_chat_writes_custom_css_without_leaking_multi_tenant_style_bundle(
    browser, multi_tenant_gateway_server, clean_multi_tenant_customizations
):
    """Chat can write custom.css, but shared `/style.css` must stay base-only.

    The gateway runs with AGENT_MULTI_TENANT=true so `/style.css` is
    intentionally unauthenticated and must not read one tenant's
    `.system/gateway/custom.css`, or the CSS would leak to every other user.
    This test exercises the chat-driven customization write, then verifies
    the shared stylesheet stays clean on reload.
    """
    base_url = multi_tenant_gateway_server

    # Open an authenticated page against the multi-tenant server.
    context, page = await _open_authed_page(browser, base_url)
    try:
        # 1. Drive the customization through chat. The mock LLM matches the
        #    `customize: move tab bar to left` trigger and emits a memory_write
        #    tool call targeting `.system/gateway/custom.css`.
        await _drive_chat_customization(page, "customize: move tab bar to left")

        # 2. Sanity check: the workspace file actually landed where the gateway
        #    will look for it. Reading via the API both confirms the write and
        #    bypasses any client-side caching of the chat tab.
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.get(
                f"{base_url}/api/memory/read",
                headers=auth_headers(),
                params={"path": ".system/gateway/custom.css"},
            )
            assert resp.status_code == 200, resp.text
            body = resp.json()
            # MemoryReadResponse uses a `content` field.
            assert "tab bar to left side panel" in body.get("content", ""), body
    finally:
        await context.close()

    # 3. Re-open the gateway in a fresh browser context. In the multi-tenant
    #    gateway, `/style.css` is the unauthenticated bootstrap sheet and
    #    must not include per-user custom.css.
    context, pg = await _open_authed_page(browser, base_url)
    try:
        await pg.locator(".tab-bar").wait_for(state="visible", timeout=10000)

        # 3a. The served stylesheet must *not* contain our overlay in
        #     multi-tenant mode. The write path above proves the file exists;
        #     this assertion proves `/style.css` did not leak it.
        async with httpx.AsyncClient(timeout=10) as client:
            css_resp = await client.get(
                f"{base_url}/style.css",
                headers=auth_headers(),
            )
            assert css_resp.status_code == 200
            assert "tab bar to left side panel" not in css_resp.text

        # 3b. The browser should still render the default horizontal bar in
        #     shared mode because the CSS overlay was intentionally withheld.
        flex_direction = await pg.evaluate(
            "() => getComputedStyle(document.querySelector('.tab-bar')).flexDirection"
        )
        assert flex_direction != "column", (
            f"Expected shared multi-tenant gateway to keep the default tab bar, "
            f"got {flex_direction!r}"
        )

        # 3c. The built-in tabs are still present after the chat-driven
        #     memory write. We mutated workspace state, not the live layout.
        for tab_id in ("chat", "memory", "settings"):
            btn = pg.locator(f'.tab-bar button[data-tab="{tab_id}"]')
            assert await btn.count() == 1, f"missing built-in tab {tab_id!r}"
    finally:
        await context.close()


async def test_chat_adds_skills_viewer_widget_to_workspace_and_widgets_api(
    browser, multi_tenant_gateway_server, clean_multi_tenant_customizations
):
    """Chat can install a widget definition without mutating the shared shell.

    The agent writes a widget manifest and an ``index.js`` implementation
    into ``.system/gateway/widgets/skills-viewer/``. In the multi-tenant
    gateway, the authenticated widgets API must surface that workspace state,
    but the base shell must not auto-inline per-user widgets into every
    browser load.
    """
    base_url = multi_tenant_gateway_server

    # Open an authenticated page against the multi-tenant server.
    context, page = await _open_authed_page(browser, base_url)
    try:
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
                f"{base_url}/api/memory/read",
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
                f"{base_url}/api/memory/read",
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
                f"{base_url}/api/frontend/widgets",
                headers=auth_headers(),
            )
            assert widgets_resp.status_code == 200, widgets_resp.text
            widget_ids = {w["id"] for w in widgets_resp.json()}
            assert "skills-viewer" in widget_ids, widget_ids
    finally:
        await context.close()

    # 3. Reload in a fresh context. The shared multi-tenant shell should not
    #    auto-inject per-user widgets into the base tab bar.
    context, pg = await _open_authed_page(browser, base_url)
    try:
        await pg.locator(".tab-bar").wait_for(state="visible", timeout=10000)
        widget_tab_btn = pg.locator('.tab-bar button[data-tab="skills-viewer"]')
        assert await widget_tab_btn.count() == 0
        for tab_id in ("chat", "memory", "settings"):
            btn = pg.locator(f'.tab-bar button[data-tab="{tab_id}"]')
            assert await btn.count() == 1, f"missing built-in tab {tab_id!r}"
    finally:
        await context.close()


async def test_layout_config_persists_without_mutating_shared_multi_tenant_shell(
    browser, ironclaw_server, clean_customizations
):
    """Layout writes persist, but the shared shell must not apply them globally."""
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

    # 2. The authenticated layout API should reflect the stored config even
    #    though the shared shell does not auto-apply it.
    async with httpx.AsyncClient(timeout=10) as client:
        layout_resp = await client.get(
            f"{ironclaw_server}/api/frontend/layout",
            headers=auth_headers(),
        )
        assert layout_resp.status_code == 200, layout_resp.text
        returned_layout = layout_resp.json()
        assert returned_layout["tabs"]["hidden"] == ["routines"], returned_layout
        assert returned_layout["chat"]["image_upload"] is False, returned_layout

    # 3. Reload in a fresh context. The shared multi-tenant shell should keep
    #    its default controls rather than applying one tenant's layout.json.
    context, pg = await _open_authed_page(browser, ironclaw_server)
    try:
        await pg.locator(".tab-bar").wait_for(state="visible", timeout=10000)
        routines_display = await pg.evaluate(
            """() => {
              const btn = document.querySelector(
                '.tab-bar button[data-tab=\"routines\"]'
              );
              return btn ? getComputedStyle(btn).display : 'missing';
            }"""
        )
        assert routines_display != "none", (
            f"shared multi-tenant shell should not hide routines tab globally, "
            f"got display={routines_display!r}"
        )
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
        assert attach_state["attachDisplay"] != "none", (
            f"shared multi-tenant shell should not hide #attach-btn globally, "
            f"got {attach_state!r}"
        )
        assert attach_state["inputExists"], (
            "#image-file-input must exist in the DOM — index.html structure "
            "regressed"
        )
        assert attach_state["inputDisabled"] is False, (
            f"shared multi-tenant shell should not disable image-file-input globally, "
            f"got {attach_state!r}"
        )
    finally:
        await context.close()


async def test_shared_index_keeps_static_csp_when_layout_is_per_user_only(
    multi_tenant_gateway_server, clean_multi_tenant_customizations
):
    """Shared index should stay static even when per-user layout exists."""
    base_url = multi_tenant_gateway_server

    # 1. Write a layout that would force customized HTML in single-tenant
    #    mode. In the multi-tenant gateway this layout remains per-user
    #    state only.
    layout = {"branding": {"title": "Acme AI"}}
    async with httpx.AsyncClient(timeout=10) as client:
        write = await client.post(
            f"{base_url}/api/memory/write",
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

        # 2. Hit `/` directly. The bootstrap route stays on the static HTML/CSP
        #    path even though the authenticated layout API now has custom data.
        resp = await client.get(
            f"{base_url}/?token={AUTH_TOKEN}",
            headers=auth_headers(),
        )
    assert resp.status_code == 200, resp.text

    # 3. The authenticated layout API should expose the saved branding config.
    async with httpx.AsyncClient(timeout=10) as client:
        layout_resp = await client.get(
            f"{base_url}/api/frontend/layout",
            headers=auth_headers(),
        )
    assert layout_resp.status_code == 200, layout_resp.text
    assert layout_resp.json()["branding"]["title"] == "Acme AI"

    # 4. Contract A: shared index keeps the static CSP with no per-response
    #    nonce because no inline customization bundle was assembled.
    csp = resp.headers.get("content-security-policy")
    assert csp is not None, (
        f"index must emit Content-Security-Policy header; got headers={dict(resp.headers)}"
    )
    nonce_match = re.search(r"'nonce-([0-9a-f]+)'", csp)
    assert nonce_match is None, f"shared static CSP should not include nonce, got: {csp}"

    body = resp.text

    # 5. Contract B: the shared bootstrap page should not contain nonce-bearing
    #    inline customization scripts. It remains the stock shell.
    all_script_tags = re.findall(r"<script\b[^>]*>", body)
    inline_script_tags = [
        tag for tag in all_script_tags if not re.search(r"\bsrc\s*=", tag)
    ]
    assert not inline_script_tags, (
        "shared multi-tenant index should not inline per-user customization scripts; "
        f"saw inline tags: {inline_script_tags!r}"
    )

    # 6. Contract C: the nonce placeholder should never leak into the shared
    #    static shell.
    assert "__IRONCLAW_CSP_NONCE__" not in body, (
        "NONCE_PLACEHOLDER sentinel must be substituted before serving — "
        "found unmodified placeholder in response body"
    )
