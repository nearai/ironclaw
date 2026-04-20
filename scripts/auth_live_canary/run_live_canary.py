#!/usr/bin/env python3
"""Live auth canary runner with two modes.

Starts a fresh local IronClaw instance and verifies real provider-backed auth
through either of two paths, selected by ``--mode``:

- ``seeded`` — seeds real provider credentials into the DB and exercises both
  ``/v1/responses`` and the browser UI. Proves credential persistence /
  refresh reliability.
- ``browser`` — triggers OAuth in the browser, completes provider login/consent
  in Playwright, then verifies the authenticated extension through both the
  browser chat UI and ``/v1/responses``. Proves browser consent flow
  correctness.

The LLM itself stays deterministic by reusing ``tests/e2e/mock_llm.py`` for
tool selection. The external dependency under test is the real provider API
and the stored credential / refresh behavior, not model output drift.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.auth_registry import (
    BROWSER_CASES,
    BrowserProviderCase,
    SeededProviderCase,
    configured_browser_cases,
    configured_seeded_cases,
)
from scripts.live_canary.auth_runtime import (
    activate_extension,
    complete_oauth_flow,
    create_responses_probe,
    install_extension,
    put_secret,
    wait_for_extension_state,
)
from scripts.live_canary.common import (
    DEFAULT_VENV,
    CanaryError,
    ProbeResult,
    api_request,
    bootstrap_python,
    cargo_build,
    env_str,
    install_playwright,
    load_e2e_helpers,
    start_gateway_stack,
    stop_gateway_stack,
    venv_python,
    write_results,
)

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-live-canary"
GOOGLE_SCOPE_DEFAULT = "gmail.modify gmail.compose calendar.events"

# Per-mode constants. Keeping these in one table makes it obvious which mode
# owns which identifiers; adding a third mode means adding one row, not
# duplicating another script.
MODE_CONFIG = {
    "seeded": {
        "owner_user_id": "auth-live-owner",
        "temp_prefix": "ironclaw-live-auth",
        "gateway_token_prefix": "auth-live",
        "reexec_env": "AUTH_LIVE_CANARY_REEXEC",
        "extra_gateway_env_names": (
            "GOOGLE_OAUTH_CLIENT_ID",
            "GOOGLE_OAUTH_CLIENT_SECRET",
        ),
        "failure_label": "Live auth canary",
    },
    "browser": {
        "owner_user_id": "auth-browser-owner",
        "temp_prefix": "ironclaw-browser-auth",
        "gateway_token_prefix": "browser-auth",
        "reexec_env": "AUTH_BROWSER_CANARY_REEXEC",
        "extra_gateway_env_names": (
            "GOOGLE_OAUTH_CLIENT_ID",
            "GOOGLE_OAUTH_CLIENT_SECRET",
            "GITHUB_OAUTH_CLIENT_ID",
            "GITHUB_OAUTH_CLIENT_SECRET",
        ),
        "failure_label": "Browser auth canary",
    },
}


# ── Seeded mode ──────────────────────────────────────────────────────────────


def expire_secret_in_db(db_path: Path, user_id: str, secret_name: str) -> None:
    with sqlite3.connect(db_path) as conn:
        cursor = conn.execute(
            """
            UPDATE secrets
            SET expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 hour')
            WHERE user_id = ? AND name = ?
            """,
            (user_id, secret_name),
        )
        conn.commit()
    if cursor.rowcount != 1:
        raise CanaryError(f"Expected exactly one secret row for {user_id}/{secret_name}")


async def seeded_response_probe(
    base_url: str,
    token: str,
    probe: SeededProviderCase,
) -> ProbeResult:
    started = time.perf_counter()
    response = await api_request(
        "POST",
        base_url,
        "/v1/responses",
        token=token,
        json_body={"model": "default", "input": probe.response_prompt},
        timeout=180,
    )
    latency_ms = int((time.perf_counter() - started) * 1000)
    if response.status_code != 200:
        return ProbeResult(
            provider=probe.key,
            mode="responses_api",
            success=False,
            latency_ms=latency_ms,
            details={"status_code": response.status_code, "body": response.text[:1000]},
        )

    body = response.json()
    response_id = body.get("id")
    output = body.get("output", [])
    tool_names = [item.get("name") for item in output if item.get("type") == "function_call"]
    tool_outputs = [
        item.get("output", "")
        for item in output
        if item.get("type") == "function_call_output"
    ]
    texts: list[str] = []
    for item in output:
        if item.get("type") != "message":
            continue
        for content in item.get("content", []):
            if content.get("type") == "output_text":
                texts.append(content.get("text", ""))
    response_text = "\n".join(texts)

    get_response = await api_request(
        "GET",
        base_url,
        f"/v1/responses/{response_id}",
        token=token,
        timeout=30,
    )
    fetched_status = get_response.status_code

    success = (
        body.get("status") == "completed"
        and probe.expected_tool_name in tool_names
        and bool(tool_outputs)
        and not any(
            marker in output_text.lower()
            for output_text in tool_outputs
            for marker in ("error", "authentication required", "unauthorized", "forbidden")
        )
        and probe.expected_text.lower() in response_text.lower()
        and fetched_status == 200
    )

    return ProbeResult(
        provider=probe.key,
        mode="responses_api",
        success=success,
        latency_ms=latency_ms,
        details={
            "response_id": response_id,
            "status": body.get("status"),
            "tool_names": tool_names,
            "tool_outputs": tool_outputs,
            "response_text": response_text,
            "get_status_code": fetched_status,
            "error": body.get("error"),
        },
    )


async def seeded_browser_probe(
    browser: Any,
    base_url: str,
    token: str,
    probe: SeededProviderCase,
    output_dir: Path,
    *,
    open_authed_page_fn: Any,
    send_chat_and_wait_for_terminal_message_fn: Any,
) -> ProbeResult:
    started = time.perf_counter()
    context = None
    page = None
    try:
        context, page = await open_authed_page_fn(browser, base_url, token=token)
        result = await send_chat_and_wait_for_terminal_message_fn(
            page,
            probe.response_prompt,
            timeout=120000,
        )
        thread_id = await page.evaluate("currentThreadId")
        history = await api_request(
            "GET",
            base_url,
            f"/api/chat/history?thread_id={thread_id}",
            token=token,
            timeout=30,
        )
        history.raise_for_status()
        tool_names = [
            tool_call.get("name")
            for turn in history.json().get("turns", [])
            for tool_call in turn.get("tool_calls", [])
        ]
        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            result.get("role") == "assistant"
            and probe.expected_text.lower() in result.get("text", "").lower()
            and probe.expected_tool_name in tool_names
        )
        return ProbeResult(
            provider=probe.key,
            mode="browser",
            success=success,
            latency_ms=latency_ms,
            details={**result, "thread_id": thread_id, "tool_names": tool_names},
        )
    except Exception as exc:  # noqa: BLE001
        latency_ms = int((time.perf_counter() - started) * 1000)
        screenshot_path = output_dir / f"{probe.key}-browser-failure.png"
        if page is not None:
            try:
                await page.screenshot(path=str(screenshot_path), full_page=True)
            except Exception:  # noqa: BLE001
                pass
        return ProbeResult(
            provider=probe.key,
            mode="browser",
            success=False,
            latency_ms=latency_ms,
            details={
                "error": str(exc),
                "screenshot": str(screenshot_path) if screenshot_path.exists() else None,
            },
        )
    finally:
        if context is not None:
            await context.close()


async def seed_google_via_oauth(
    base_url: str, token: str, db_path: Path, owner_user_id: str,
) -> bool:
    """Authenticate Google extensions via the OAuth callback flow.

    Returns True if Google credentials were configured and the OAuth flow
    completed, False if no Google credentials are available (skipped).
    """
    google_access = env_str("AUTH_LIVE_GOOGLE_ACCESS_TOKEN")
    google_refresh = env_str("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    if google_refresh and not google_access:
        raise CanaryError(
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN is required when AUTH_LIVE_GOOGLE_REFRESH_TOKEN is set"
        )
    if not google_access:
        return False

    # Install Gmail and complete OAuth flow. The mock_llm exchange endpoint
    # reads AUTH_LIVE_GOOGLE_* env vars and returns the real tokens.
    await install_extension(
        base_url, token,
        name="gmail",
        expected_display_name="Gmail",
    )
    await complete_oauth_flow(base_url, token, extension_name="gmail")

    # Ensure combined scopes cover all Google extensions (Gmail + Calendar).
    await put_secret(
        base_url, token,
        user_id=owner_user_id,
        name="google_oauth_token_scopes",
        value=env_str("AUTH_LIVE_GOOGLE_SCOPES") or GOOGLE_SCOPE_DEFAULT,
        provider="google",
    )

    # Optionally expire the access token to exercise the refresh path.
    if google_refresh and env_str("AUTH_LIVE_FORCE_GOOGLE_REFRESH", "1") != "0":
        expire_secret_in_db(db_path, owner_user_id, "google_oauth_token")

    return True


async def seed_non_oauth_credentials(
    base_url: str, token: str, owner_user_id: str,
) -> None:
    """Seed non-OAuth credentials (GitHub PAT, Notion tokens) directly."""
    github_token = env_str("AUTH_LIVE_GITHUB_TOKEN")
    if github_token:
        await put_secret(
            base_url, token,
            user_id=owner_user_id,
            name="github_token",
            value=github_token,
            provider="github",
        )

    notion_access = env_str("AUTH_LIVE_NOTION_ACCESS_TOKEN")
    notion_refresh = env_str("AUTH_LIVE_NOTION_REFRESH_TOKEN")
    if notion_refresh and not notion_access:
        raise CanaryError(
            "AUTH_LIVE_NOTION_ACCESS_TOKEN is required when AUTH_LIVE_NOTION_REFRESH_TOKEN is set"
        )
    if notion_access:
        await put_secret(
            base_url, token,
            user_id=owner_user_id,
            name="mcp_notion_access_token",
            value=notion_access,
            provider="mcp:notion",
        )
    if notion_refresh:
        await put_secret(
            base_url, token,
            user_id=owner_user_id,
            name="mcp_notion_access_token_refresh_token",
            value=notion_refresh,
            provider="mcp:notion",
        )
    # Notion MCP uses DCR — seed client_id/secret so ironclaw can refresh.
    notion_client_id = env_str("AUTH_LIVE_NOTION_CLIENT_ID")
    notion_client_secret = env_str("AUTH_LIVE_NOTION_CLIENT_SECRET")
    if notion_client_id:
        await put_secret(
            base_url, token,
            user_id=owner_user_id,
            name="mcp_notion_client_id",
            value=notion_client_id,
            provider="mcp:notion",
        )
    if notion_client_secret:
        await put_secret(
            base_url, token,
            user_id=owner_user_id,
            name="mcp_notion_client_secret",
            value=notion_client_secret,
            provider="mcp:notion",
        )


async def run_seeded_mode(args: argparse.Namespace, stack: Any) -> list[ProbeResult]:
    probes = configured_seeded_cases(args.case)
    if not probes:
        raise CanaryError(
            "No live provider cases are configured. Set at least one AUTH_LIVE_* credential env var."
        )

    owner_user_id = MODE_CONFIG["seeded"]["owner_user_id"]

    # Phase 1: Google extensions — authenticate via OAuth flow so ironclaw
    # marks them as properly authenticated (direct DB seeding doesn't work).
    google_oauth_done = await seed_google_via_oauth(
        stack.base_url, stack.gateway_token, stack.db_path, owner_user_id,
    )

    # Phase 2: Non-OAuth credentials (GitHub PAT, Notion tokens) — seed directly.
    await seed_non_oauth_credentials(stack.base_url, stack.gateway_token, owner_user_id)

    # Phase 3: Install and activate all extensions.
    for probe in probes:
        is_google = probe.shared_secret_name == "google_oauth_token"
        if is_google and google_oauth_done and probe.extension_install_name == "gmail":
            # Already installed and authenticated via OAuth flow above.
            continue
        ext = await install_extension(
            stack.base_url,
            stack.gateway_token,
            name=probe.extension_install_name,
            expected_display_name=probe.expected_display_name,
            install_kind=probe.install_kind,
            install_url=probe.install_url,
        )
        if is_google and google_oauth_done:
            # Google extensions share google_oauth_token but ironclaw tracks
            # auth per-extension. Complete OAuth for each one individually.
            await complete_oauth_flow(
                stack.base_url, stack.gateway_token,
                extension_name=ext["name"],
            )
        else:
            await activate_extension(
                stack.base_url,
                stack.gateway_token,
                extension_name=ext["name"],
                expected_display_name=ext.get("display_name") or probe.expected_display_name,
            )

    results: list[ProbeResult] = []
    for probe in probes:
        results.append(await seeded_response_probe(stack.base_url, stack.gateway_token, probe))

    open_authed_page_fn, send_chat_and_wait_for_terminal_message_fn = load_e2e_helpers(
        "open_authed_page",
        "send_chat_and_wait_for_terminal_message",
    )
    from playwright.async_api import async_playwright

    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=env_str("HEADED") != "1")
        try:
            for probe in probes:
                if probe.browser_enabled:
                    results.append(
                        await seeded_browser_probe(
                            browser,
                            stack.base_url,
                            stack.gateway_token,
                            probe,
                            args.output_dir,
                            open_authed_page_fn=open_authed_page_fn,
                            send_chat_and_wait_for_terminal_message_fn=send_chat_and_wait_for_terminal_message_fn,
                        )
                    )
        finally:
            await browser.close()

    return results


# ── Browser mode ─────────────────────────────────────────────────────────────


def storage_state_path(case_key: str) -> str | None:
    return env_str(f"AUTH_BROWSER_{case_key.upper()}_STORAGE_STATE_PATH")


def provider_username(case_key: str) -> str | None:
    return env_str(f"AUTH_BROWSER_{case_key.upper()}_USERNAME")


def provider_password(case_key: str) -> str | None:
    return env_str(f"AUTH_BROWSER_{case_key.upper()}_PASSWORD")


async def open_gateway_page(
    browser: Any,
    base_url: str,
    token: str,
    storage_state: str | None,
) -> tuple[Any, Any]:
    kwargs: dict[str, Any] = {"viewport": {"width": 1280, "height": 720}}
    if storage_state:
        kwargs["storage_state"] = storage_state
    context = await browser.new_context(**kwargs)
    page = await context.new_page()
    await page.goto(f"{base_url}/?token={token}", timeout=15000)
    await page.locator("#auth-screen").wait_for(state="hidden", timeout=10000)
    return context, page


async def wait_for_auth_card(page: Any, selectors: dict[str, str], extension_name: str | None = None) -> Any:
    selector = selectors["auth_card"]
    if extension_name:
        selector += f'[data-extension-name="{extension_name}"]'
    card = page.locator(selector).first
    await card.wait_for(state="visible", timeout=30000)
    return card


async def trigger_auth_card(
    page: Any,
    selectors: dict[str, str],
    case: BrowserProviderCase,
) -> Any:
    chat_input = page.locator(selectors["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    await chat_input.fill(case.trigger_prompt)
    await chat_input.press("Enter")
    return await wait_for_auth_card(page, selectors, case.auth_extension_name)


async def click_auth_popup(page: Any, oauth_button: Any) -> Any:
    try:
        async with page.expect_popup(timeout=10000) as popup_info:
            await oauth_button.click()
        return await popup_info.value
    except Exception:
        href = await oauth_button.get_attribute("href")
        if not href:
            raise CanaryError("OAuth button had no popup and no href")
        popup = await page.context.new_page()
        await popup.goto(href, timeout=30000)
        return popup


async def click_first_button_with_text(page: Any, labels: list[str], timeout_ms: int = 4000) -> bool:
    for label in labels:
        locator = page.get_by_role("button", name=re.compile(label, re.I)).first
        try:
            await locator.wait_for(state="visible", timeout=timeout_ms)
            await locator.click()
            return True
        except Exception:
            continue
    return False


async def handle_google_popup(popup: Any, case_key: str) -> None:
    username = provider_username(case_key)
    password = provider_password(case_key)

    await popup.wait_for_load_state("domcontentloaded", timeout=30000)

    if username:
        email_input = popup.locator('input[type="email"]').first
        try:
            await email_input.wait_for(state="visible", timeout=8000)
            await email_input.fill(username)
            await click_first_button_with_text(popup, ["Next"])
        except Exception:
            pass

    if password:
        password_input = popup.locator('input[type="password"]').first
        try:
            await password_input.wait_for(state="visible", timeout=12000)
            await password_input.fill(password)
            await click_first_button_with_text(popup, ["Next"])
        except Exception:
            pass

    await click_first_button_with_text(
        popup,
        ["Continue", "Allow", "Grant access", "Go to IronClaw", "Confirm"],
        timeout_ms=10000,
    )


async def handle_notion_popup(popup: Any, case_key: str) -> None:
    username = provider_username(case_key)
    password = provider_password(case_key)

    await popup.wait_for_load_state("domcontentloaded", timeout=30000)

    if username:
        email_input = popup.locator('input[type="email"]').first
        try:
            await email_input.wait_for(state="visible", timeout=8000)
            await email_input.fill(username)
            await click_first_button_with_text(popup, ["Continue", "Next", "Sign in"])
        except Exception:
            pass

    if password:
        password_input = popup.locator('input[type="password"]').first
        try:
            await password_input.wait_for(state="visible", timeout=10000)
            await password_input.fill(password)
            await click_first_button_with_text(popup, ["Continue", "Sign in", "Log in"])
        except Exception:
            pass

    await click_first_button_with_text(
        popup,
        ["Allow access", "Allow", "Grant access", "Select pages", "Continue"],
        timeout_ms=10000,
    )


async def handle_github_popup(popup: Any, case_key: str) -> None:
    username = provider_username(case_key)
    password = provider_password(case_key)

    await popup.wait_for_load_state("domcontentloaded", timeout=30000)

    if username:
        username_input = popup.locator(
            'input[name="login"], input#login_field, input[autocomplete="username"]'
        ).first
        try:
            await username_input.wait_for(state="visible", timeout=8000)
            await username_input.fill(username)
        except Exception:
            pass

    if password:
        password_input = popup.locator(
            'input[name="password"], input#password, input[type="password"]'
        ).first
        try:
            await password_input.wait_for(state="visible", timeout=8000)
            await password_input.fill(password)
            await click_first_button_with_text(popup, ["Sign in", "Log in"], timeout_ms=8000)
        except Exception:
            pass

    await click_first_button_with_text(
        popup,
        ["Authorize", "Authorize IronClaw", "Continue", "Approve", "Grant access"],
        timeout_ms=10000,
    )


async def complete_provider_auth(
    popup: Any,
    case: BrowserProviderCase,
    output_dir: Path,
) -> None:
    if case.key == "google":
        await handle_google_popup(popup, case.key)
    elif case.key == "notion":
        await handle_notion_popup(popup, case.key)
    elif case.key == "github":
        await handle_github_popup(popup, case.key)
    else:
        raise CanaryError(f"No popup handler for provider {case.key}")

    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        url = popup.url
        if "/oauth/callback" in url or "connected" in url.lower():
            return
        try:
            await popup.wait_for_load_state("networkidle", timeout=3000)
        except Exception:
            pass
        await asyncio.sleep(1.0)

    screenshot = output_dir / f"{case.key}-oauth-timeout.png"
    try:
        await popup.screenshot(path=str(screenshot), full_page=True)
    except Exception:
        pass
    raise CanaryError(f"Timed out waiting for {case.key} OAuth callback page")


async def browser_oauth_probe(
    browser: Any,
    base_url: str,
    token: str,
    case: BrowserProviderCase,
    selectors: dict[str, str],
    send_chat_and_wait_for_terminal_message_fn: Any,
    output_dir: Path,
) -> list[ProbeResult]:
    storage_state = storage_state_path(case.key)
    context = None
    page = None
    popup = None
    results: list[ProbeResult] = []
    started = time.perf_counter()
    try:
        context, page = await open_gateway_page(browser, base_url, token, storage_state)
        auth_card = await trigger_auth_card(page, selectors, case)
        oauth_button = auth_card.locator(selectors["auth_oauth_btn"]).first
        await oauth_button.wait_for(state="visible", timeout=10000)
        popup = await click_auth_popup(page, oauth_button)
        await complete_provider_auth(popup, case, output_dir)

        await auth_card.wait_for(state="hidden", timeout=30000)
        await wait_for_extension_state(
            base_url,
            token,
            case.expected_extension_name,
            authenticated=True,
            active=True,
            timeout=60.0,
        )

        chat_result = await send_chat_and_wait_for_terminal_message_fn(
            page,
            case.trigger_prompt,
            timeout=120000,
        )
        history_thread_id = await page.evaluate("() => currentThreadId")
        history = await api_request(
            "GET",
            base_url,
            f"/api/chat/history?thread_id={history_thread_id}",
            token=token,
            timeout=30,
        )
        history.raise_for_status()
        tool_names = [
            tool_call.get("name")
            for turn in history.json().get("turns", [])
            for tool_call in turn.get("tool_calls", [])
        ]
        latency_ms = int((time.perf_counter() - started) * 1000)
        results.append(
            ProbeResult(
                provider=case.key,
                mode="browser_oauth",
                success=True,
                latency_ms=latency_ms,
                details={
                    "popup_url": popup.url if popup else None,
                    "thread_id": history_thread_id,
                    "tool_names": tool_names,
                    "assistant_text": chat_result.get("text", ""),
                },
            )
        )
        results.append(
            ProbeResult(
                provider=case.key,
                mode="browser_chat",
                success=(
                    case.expected_tool_name in tool_names
                    and case.expected_text in chat_result.get("text", "")
                ),
                latency_ms=latency_ms,
                details={
                    "thread_id": history_thread_id,
                    "tool_names": tool_names,
                    "assistant_text": chat_result.get("text", ""),
                },
            )
        )
        return results
    except Exception as exc:  # noqa: BLE001
        latency_ms = int((time.perf_counter() - started) * 1000)
        screenshot = output_dir / f"{case.key}-browser-failure.png"
        if page is not None:
            try:
                await page.screenshot(path=str(screenshot), full_page=True)
            except Exception:
                pass
        return [
            ProbeResult(
                provider=case.key,
                mode="browser_oauth",
                success=False,
                latency_ms=latency_ms,
                details={
                    "error": str(exc),
                    "screenshot": str(screenshot) if screenshot.exists() else None,
                },
            )
        ]
    finally:
        if context is not None:
            await context.close()


async def run_browser_mode(args: argparse.Namespace, stack: Any) -> list[ProbeResult]:
    cases = configured_browser_cases(args.case)
    if not cases:
        raise CanaryError(
            "No browser-consent cases are configured. Provide storage state or credentials for at least one provider."
        )

    selectors, send_chat_and_wait_for_terminal_message_fn = load_e2e_helpers(
        "SEL",
        "send_chat_and_wait_for_terminal_message",
    )
    from playwright.async_api import async_playwright

    for case in cases:
        await install_extension(
            stack.base_url,
            stack.gateway_token,
            name=case.extension_name,
            expected_display_name=case.expected_extension_name,
            install_kind=case.install_kind,
            install_url=case.install_url,
        )
        await wait_for_extension_state(
            stack.base_url,
            stack.gateway_token,
            case.expected_extension_name,
            timeout=30.0,
        )

    results: list[ProbeResult] = []
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(headless=env_str("HEADED") != "1")
        try:
            for case in cases:
                results.extend(
                    await browser_oauth_probe(
                        browser,
                        stack.base_url,
                        stack.gateway_token,
                        case,
                        selectors,
                        send_chat_and_wait_for_terminal_message_fn,
                        args.output_dir,
                    )
                )
                if any(result.provider == case.key and not result.success for result in results):
                    continue
                results.append(
                    await create_responses_probe(
                        base_url=stack.base_url,
                        token=stack.gateway_token,
                        provider=case.key,
                        prompt=case.trigger_prompt,
                        expected_tool_name=case.expected_tool_name,
                        expected_text=case.expected_text,
                    )
                )
        finally:
            await browser.close()

    return results


# ── CLI / bootstrap shared between modes ─────────────────────────────────────


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--mode",
        required=True,
        choices=sorted(MODE_CONFIG),
        help="Which flow to run: seeded token probes, or browser OAuth consent.",
    )
    parser.add_argument(
        "--venv",
        type=Path,
        default=DEFAULT_VENV,
        help=f"Virtualenv path (default: {DEFAULT_VENV})",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help=(
            "Artifacts directory. Defaults to "
            f"{DEFAULT_OUTPUT_DIR}/<mode>/ so seeded and browser runs stay separate."
        ),
    )
    parser.add_argument(
        "--playwright-install",
        choices=("auto", "with-deps", "plain", "skip"),
        default="auto",
        help="How to install Playwright browsers.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build.",
    )
    parser.add_argument(
        "--skip-python-bootstrap",
        action="store_true",
        help="Skip venv creation and pip install.",
    )
    parser.add_argument(
        "--case",
        action="append",
        help=(
            "Limit the run to selected providers. Repeat for multiple values. "
            "For seeded mode: gmail, google_calendar, github, notion. "
            "For browser mode: google, github, notion."
        ),
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="Print the configured cases for the chosen mode and exit.",
    )
    args = parser.parse_args()
    if args.output_dir is None:
        args.output_dir = DEFAULT_OUTPUT_DIR / args.mode
    _validate_case_choices(args)
    return args


def _validate_case_choices(args: argparse.Namespace) -> None:
    if not args.case:
        return
    seeded_choices = {"gmail", "google_calendar", "github", "notion"}
    browser_choices = set(BROWSER_CASES)
    allowed = seeded_choices if args.mode == "seeded" else browser_choices
    bad = [c for c in args.case if c not in allowed]
    if bad:
        raise SystemExit(
            f"--case values {bad} are not valid for --mode {args.mode}. "
            f"Allowed: {sorted(allowed)}"
        )


def _preflight_refresh_google_token() -> None:
    """Refresh the Google access token before the gateway starts.

    GitHub secrets store a static access token that expires after 1 hour.
    The mock_llm exchange endpoint returns whatever is in
    AUTH_LIVE_GOOGLE_ACCESS_TOKEN, so we must refresh it here to ensure
    the token is valid when the test runs.
    """
    import urllib.request
    import urllib.parse

    refresh_token = env_str("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    client_id = env_str("GOOGLE_OAUTH_CLIENT_ID")
    client_secret = env_str("GOOGLE_OAUTH_CLIENT_SECRET")
    if not all([refresh_token, client_id, client_secret]):
        return

    data = urllib.parse.urlencode({
        "client_id": client_id,
        "client_secret": client_secret,
        "refresh_token": refresh_token,
        "grant_type": "refresh_token",
    }).encode()
    try:
        req = urllib.request.Request("https://oauth2.googleapis.com/token", data=data)
        with urllib.request.urlopen(req, timeout=15) as resp:
            body = json.loads(resp.read())
        fresh_token = body.get("access_token")
        if fresh_token:
            os.environ["AUTH_LIVE_GOOGLE_ACCESS_TOKEN"] = fresh_token
            print(f"[preflight] Refreshed Google access token (expires_in={body.get('expires_in')}s)")
        else:
            print(f"[preflight] Google token refresh returned no access_token: {body}")
    except Exception as exc:
        print(f"[preflight] Google token refresh failed: {exc}")


async def async_main(args: argparse.Namespace) -> int:
    mode_cfg = MODE_CONFIG[args.mode]

    if args.list_cases:
        cases = (
            configured_seeded_cases(args.case)
            if args.mode == "seeded"
            else configured_browser_cases(args.case)
        )
        for case in cases:
            print(case.key)
        return 0

    if not args.skip_build:
        cargo_build()

    # Pre-flight: refresh the Google access token so the mock exchange
    # endpoint returns a fresh token (the one in secrets may be expired).
    if args.mode == "seeded":
        _preflight_refresh_google_token()

    extra_gateway_env: dict[str, str] = {}
    for env_name in mode_cfg["extra_gateway_env_names"]:
        value = env_str(env_name)
        if value:
            extra_gateway_env[env_name] = value

    stack = await start_gateway_stack(
        venv_dir=args.venv,
        owner_user_id=mode_cfg["owner_user_id"],
        temp_prefix=mode_cfg["temp_prefix"],
        gateway_token_prefix=mode_cfg["gateway_token_prefix"],
        extra_gateway_env=extra_gateway_env,
        oauth_proxy=(args.mode == "seeded"),
    )
    try:
        if args.mode == "seeded":
            results = await run_seeded_mode(args, stack)
        else:
            results = await run_browser_mode(args, stack)

        results_path = write_results(args.output_dir, results, stack.base_url)
        failures = [result for result in results if not result.success]
        if failures:
            print(f"\n{mode_cfg['failure_label']} failures written to {results_path}", flush=True)
            for failure in failures:
                print(
                    f"- {failure.provider}/{failure.mode}: {json.dumps(failure.details, default=str)}",
                    flush=True,
                )
            return 1

        print(f"\n{mode_cfg['failure_label']} passed. Results: {results_path}", flush=True)
        return 0
    finally:
        stop_gateway_stack(stack)


def main() -> int:
    args = parse_args()
    mode_cfg = MODE_CONFIG[args.mode]
    reexec_env = mode_cfg["reexec_env"]
    try:
        if args.list_cases:
            return asyncio.run(async_main(args))
        if not args.skip_python_bootstrap and os.environ.get(reexec_env) != "1":
            python = bootstrap_python(args.venv)
            install_playwright(python, args.playwright_install)
            cmd = [str(python), str(Path(__file__).resolve()), *sys.argv[1:], "--skip-python-bootstrap"]
            env = os.environ.copy()
            env[reexec_env] = "1"
            return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode
        if args.skip_python_bootstrap and not venv_python(args.venv).exists() and os.environ.get(reexec_env) != "1":
            raise CanaryError(
                f"Virtualenv Python not found at {venv_python(args.venv)}. "
                "Remove --skip-python-bootstrap or create it first."
            )
        return asyncio.run(async_main(args))
    except CanaryError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
