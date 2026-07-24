#!/usr/bin/env python3
"""Live auth canary runner with two modes.

Starts a fresh local IronClaw instance and verifies real provider-backed auth
through either of two paths, selected by ``--mode``:

- ``seeded`` — configures required tenant provider metadata, installs the
  extension, and completes its manifest-declared setup recipe with live test
  credentials before exercising ``/v1/responses`` and the browser UI.
- ``browser`` — triggers OAuth in the browser, completes provider login/consent
  in Playwright, then verifies the derived-active extension through both the
  browser chat UI and ``/v1/responses``.

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
import subprocess
import sys
import time
import uuid
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
    complete_manual_token_setup,
    complete_oauth_flow,
    configure_admin_group,
    create_responses_probe,
    install_extension,
    select_single_oauth_account,
    start_oauth_setup,
    wait_for_extension_lifecycle,
    wait_for_oauth_flow_completed,
)
from scripts.live_canary.common import (
    DEFAULT_VENV,
    CanaryError,
    ProbeResult,
    api_request,
    bootstrap_python,
    cargo_build_reborn,
    env_secret,
    env_str,
    install_playwright,
    load_e2e_helpers,
    start_reborn_gateway_stack,
    stop_gateway_stack,
    venv_python,
    write_results,
)

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-live-canary"
GOOGLE_SCOPE_DEFAULT = " ".join(
    (
        "https://www.googleapis.com/auth/gmail.readonly",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/calendar.readonly",
        "https://www.googleapis.com/auth/calendar.events",
    )
)
REQUIRED_LIFECYCLE_TOOLS = {
    "gmail": "gmail.list_messages",
    "google-calendar": "google-calendar.list_events",
    "github": "github.get_repo",
    "notion": "notion.notion-search",
}

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
    selectors: dict[str, str],
) -> ProbeResult:
    started = time.perf_counter()
    context = None
    page = None
    try:
        thread_id = await create_reborn_thread(base_url, token)
        context, page = await open_gateway_page(
            browser,
            base_url,
            token,
            storage_state=None,
            selectors=selectors,
            thread_id=thread_id,
        )
        result = await send_reborn_browser_message(
            page,
            base_url,
            token,
            thread_id,
            probe.response_prompt,
            selectors=selectors,
            timeout=120.0,
        )
        tool_names = result["tool_names"]
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


async def configure_google_admin(base_url: str, token: str) -> None:
    """Configure the tenant once through the manifest-declared admin group."""
    client_id = env_str("GOOGLE_OAUTH_CLIENT_ID")
    client_secret = env_secret("GOOGLE_OAUTH_CLIENT_SECRET")
    if not client_id or not client_secret:
        raise CanaryError(
            "Google canary cases require GOOGLE_OAUTH_CLIENT_ID and "
            "GOOGLE_OAUTH_CLIENT_SECRET for vendor.google admin configuration"
        )
    await configure_admin_group(
        base_url,
        token,
        group_id="vendor.google",
        values={
            "google_oauth_client_id": client_id,
            "google_oauth_client_secret": client_secret,
        },
        idempotency_key="auth-live-canary-google-admin",
    )


async def setup_seeded_extension(
    base_url: str,
    token: str,
    probe: SeededProviderCase,
) -> None:
    """Drive setup solely from the package manifest's declared auth recipe."""
    package_id = probe.extension_install_name
    required_tool = REQUIRED_LIFECYCLE_TOOLS[package_id]
    installed = await install_extension(
        base_url,
        token,
        package_id=package_id,
        idempotency_key=f"auth-live-canary-seeded-{package_id}",
    )
    if installed.get("installation_state") == "active":
        await wait_for_extension_lifecycle(
            base_url,
            token,
            package_id,
            state="active",
            required_tools=(required_tool,),
        )
        return

    if probe.shared_secret_name == "google_oauth_token":
        await complete_oauth_flow(
            base_url,
            token,
            package_id=package_id,
            code=f"mock_auth_code_{package_id.replace('-', '_')}",
            callback_params={
                "scope": env_str("AUTH_LIVE_GOOGLE_SCOPES")
                or GOOGLE_SCOPE_DEFAULT,
            },
            required_tools=(required_tool,),
        )
        return

    if probe.key == "github":
        github_token = env_secret("AUTH_LIVE_GITHUB_TOKEN")
        if not github_token:
            raise CanaryError("AUTH_LIVE_GITHUB_TOKEN is required for github")
        await complete_manual_token_setup(
            base_url,
            token,
            package_id=package_id,
            value=github_token,
            required_tools=(required_tool,),
        )
        return

    raise CanaryError(
        f"Seeded setup for {package_id!r} is not a declared non-interactive "
        "auth recipe. Run its browser OAuth case instead."
    )


async def run_seeded_mode(args: argparse.Namespace, stack: Any) -> list[ProbeResult]:
    probes = configured_seeded_cases(args.case)
    if not probes:
        raise CanaryError(
            "No live provider cases are configured. Set at least one AUTH_LIVE_* credential env var."
        )

    if any(probe.shared_secret_name == "google_oauth_token" for probe in probes):
        await configure_google_admin(stack.base_url, stack.gateway_token)

    # Lifecycle cases reuse the same extension as their read-only counterpart,
    # so setup each package only once.
    installed_extensions: set[str] = set()
    for probe in probes:
        if probe.extension_install_name in installed_extensions:
            continue
        await setup_seeded_extension(
            stack.base_url,
            stack.gateway_token,
            probe,
        )
        installed_extensions.add(probe.extension_install_name)

    results: list[ProbeResult] = []
    for probe in probes:
        results.append(await seeded_response_probe(stack.base_url, stack.gateway_token, probe))

    (selectors,) = load_e2e_helpers("SEL_V2")
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
                            selectors=selectors,
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
    selectors: dict[str, str],
    thread_id: str,
) -> tuple[Any, Any]:
    kwargs: dict[str, Any] = {"viewport": {"width": 1280, "height": 720}}
    if storage_state:
        kwargs["storage_state"] = storage_state
    context = await browser.new_context(**kwargs)
    page = await context.new_page()
    await page.goto(
        f"{base_url}/chat/{thread_id}?token={token}",
        timeout=15000,
    )
    await page.locator(selectors["chat_composer"]).wait_for(
        state="visible",
        timeout=15000,
    )
    return context, page


async def create_reborn_thread(base_url: str, token: str) -> str:
    response = await api_request(
        "POST",
        base_url,
        "/api/webchat/v2/threads",
        token=token,
        json_body={"client_action_id": str(uuid.uuid4())},
        timeout=30,
    )
    if not 200 <= response.status_code < 300:
        raise CanaryError(
            f"Failed to create Reborn thread: {response.status_code} {response.text}"
        )
    thread_id = (response.json().get("thread") or {}).get("thread_id")
    if not thread_id:
        raise CanaryError(f"Thread create response omitted thread_id: {response.json()!r}")
    return thread_id


async def send_reborn_browser_message(
    page: Any,
    base_url: str,
    token: str,
    thread_id: str,
    prompt: str,
    *,
    selectors: dict[str, str],
    timeout: float,
) -> dict[str, Any]:
    composer = page.locator(selectors["chat_composer"])
    await composer.fill(prompt)
    await composer.press("Enter")

    deadline = time.monotonic() + timeout
    last_timeline: dict[str, Any] = {}
    while time.monotonic() < deadline:
        response = await api_request(
            "GET",
            base_url,
            f"/api/webchat/v2/threads/{thread_id}/timeline",
            token=token,
            timeout=30,
        )
        if 200 <= response.status_code < 300:
            last_timeline = response.json()
            finalized = [
                item
                for item in last_timeline.get("messages", [])
                if item.get("kind") == "assistant"
                and item.get("status") == "finalized"
                and (item.get("content") or "").strip()
            ]
            if finalized:
                tool_names: list[str] = []
                for item in last_timeline.get("messages", []):
                    if item.get("kind") != "capability_display_preview":
                        continue
                    try:
                        preview = json.loads(item.get("content") or "{}")
                    except json.JSONDecodeError:
                        continue
                    capability_id = preview.get("capability_id")
                    if isinstance(capability_id, str):
                        tool_names.append(capability_id)
                return {
                    "role": "assistant",
                    "text": finalized[-1]["content"],
                    "tool_names": tool_names,
                }
        await asyncio.sleep(0.5)
    raise CanaryError(
        f"Timed out waiting for Reborn browser reply in {thread_id}; "
        f"last timeline={last_timeline!r}"
    )


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

    # Account picker — when storage_state carries a logged-in session,
    # Google often lands on "Choose an account" instead of jumping straight
    # to consent. Try a sequence of selectors so we cope with Google's UI
    # changes; the picker rows are sometimes div[role="link"], sometimes
    # role="button", and the text node is sometimes a child of the
    # clickable element. Clicking an email-looking child works because
    # Playwright bubbles the click to the nearest interactive ancestor.
    try:
        await popup.get_by_text(
            re.compile(r"Choose an account", re.I)
        ).first.wait_for(state="visible", timeout=5000)
        print("[auth-canary] account picker detected, attempting click", flush=True)
        # Strategies, in order — first that produces a visible match wins.
        candidates = []
        if username:
            candidates.append(popup.get_by_text(username, exact=False).first)
            candidates.append(
                popup.locator(f'[data-identifier="{username}"]').first
            )
        # Generic fallback when no username is configured: pick the first
        # visible interactive element (link/button) whose accessible text
        # looks like an email address. Filtering by ARIA role excludes
        # spurious matches against `<style>` blocks (CSS at-rules contain
        # `@`) and other non-clickable text nodes that would otherwise
        # match a naive `:has-text` filter.
        email_pattern = re.compile(r"\S+@\S+\.\S+")
        candidates.append(
            popup.get_by_role("link")
            .filter(has_text=email_pattern)
            .filter(has_not_text=re.compile(r"Use another account", re.I))
            .first
        )
        candidates.append(
            popup.get_by_role("button")
            .filter(has_text=email_pattern)
            .filter(has_not_text=re.compile(r"Use another account", re.I))
            .first
        )
        for idx, candidate in enumerate(candidates):
            try:
                await candidate.wait_for(state="visible", timeout=3000)
                await candidate.click(timeout=3000)
                print(
                    f"[auth-canary] account picker: clicked candidate {idx}",
                    flush=True,
                )
                await popup.wait_for_load_state("domcontentloaded", timeout=15000)
                break
            except Exception as exc:
                print(
                    f"[auth-canary] account picker candidate {idx} skipped: {exc}",
                    flush=True,
                )
                continue
    except Exception:
        # No picker visible — proceed to email/password/consent.
        pass

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

    # Notion's MCP consent screen gates the Continue button behind an
    # "I recognize and trust this URL" checkbox — confirmed via the
    # canary's CI screenshot. Without ticking it, Continue stays disabled
    # and the handler's button click is a no-op.
    try:
        consent_checkbox = popup.get_by_text(
            re.compile(r"I recognize and trust this URL", re.I)
        ).first
        await consent_checkbox.wait_for(state="visible", timeout=5000)
        print("[auth-canary] notion 'trust URL' checkbox detected", flush=True)
        await consent_checkbox.click(timeout=3000)
        print("[auth-canary] notion 'trust URL' checkbox clicked", flush=True)
    except Exception as exc:
        print(f"[auth-canary] notion checkbox skipped: {exc}", flush=True)

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
        if (
            "/api/reborn/product-auth/oauth/" in url
            and "/callback" in url
        ) or "connected" in url.lower():
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
    output_dir: Path,
) -> list[ProbeResult]:
    storage_state = storage_state_path(case.key)
    context = None
    page = None
    popup = None
    results: list[ProbeResult] = []
    started = time.perf_counter()
    try:
        package_id = case.auth_extension_name or case.extension_name
        requirement, started_flow = await start_oauth_setup(
            base_url,
            token,
            package_id=package_id,
        )
        thread_id = await create_reborn_thread(base_url, token)
        context, page = await open_gateway_page(
            browser,
            base_url,
            token,
            storage_state,
            selectors,
            thread_id,
        )

        # Open the OAuth URL directly in a popup and complete provider login.
        popup = await page.context.new_page()
        await popup.goto(started_flow["authorization_url"], timeout=30000)
        await complete_provider_auth(popup, case, output_dir)
        invocation_id = started_flow["callback_scope"]["invocation_id"]
        await wait_for_oauth_flow_completed(
            base_url,
            token,
            package_id=package_id,
            flow_id=started_flow["flow_id"],
            invocation_id=invocation_id,
        )
        await select_single_oauth_account(
            base_url,
            token,
            package_id=package_id,
            provider=requirement["provider"],
            invocation_id=invocation_id,
        )

        await wait_for_extension_lifecycle(
            base_url,
            token,
            package_id,
            state="active",
            required_tools=(REQUIRED_LIFECYCLE_TOOLS[package_id],),
            timeout=60.0,
        )

        chat_result = await send_reborn_browser_message(
            page,
            base_url,
            token,
            thread_id,
            case.trigger_prompt,
            selectors=selectors,
            timeout=120.0,
        )
        tool_names = chat_result["tool_names"]
        latency_ms = int((time.perf_counter() - started) * 1000)
        results.append(
            ProbeResult(
                provider=case.key,
                mode="browser_oauth",
                success=True,
                latency_ms=latency_ms,
                details={
                    "popup_url": popup.url if popup else None,
                    "thread_id": thread_id,
                    "tool_names": tool_names,
                    "assistant_text": chat_result.get("text", ""),
                },
            )
        )
        # Case-insensitive substring match on the assistant text — real LLM
        # responses vary in capitalization ("Inbox" vs "inbox", "Gmail" vs
        # "gmail") and the canary's value is in confirming the tool ran and
        # the response references it, not in matching exact wording.
        results.append(
            ProbeResult(
                provider=case.key,
                mode="browser_chat",
                success=(
                    case.expected_tool_name in tool_names
                    and case.expected_text.lower()
                    in chat_result.get("text", "").lower()
                ),
                latency_ms=latency_ms,
                details={
                    "thread_id": thread_id,
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

    (selectors,) = load_e2e_helpers("SEL_V2")
    from playwright.async_api import async_playwright

    for case in cases:
        await install_extension(
            stack.base_url,
            stack.gateway_token,
            package_id=case.extension_name,
            idempotency_key=(
                f"auth-live-canary-browser-{case.extension_name}"
            ),
        )
        await wait_for_extension_lifecycle(
            stack.base_url,
            stack.gateway_token,
            case.expected_extension_name,
            state="setup_needed",
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
            "For seeded mode, read-only cases (run by default when --case is "
            "omitted): gmail, google_calendar, github. "
            "Mutating lifecycle cases — must be opted in explicitly, never "
            "run by default: gmail_roundtrip, google_calendar_lifecycle, "
            "notion_search_lifecycle (currently rejected because Notion is "
            "browser-OAuth only). "
            "For browser mode: google, notion. "
            "(github browser coverage is intentionally absent — the github "
            "WASM tool is PAT-only, not OAuth; see SEEDED_CASES instead.)"
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
    seeded_choices = {
        "gmail", "google_calendar", "github", "notion",
        "gmail_roundtrip",
        "google_calendar_lifecycle", "notion_search_lifecycle",
    }
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

    CI may store an access token that expires before the canary starts.
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
        cargo_build_reborn()

    # Pre-flight: refresh expired access tokens so seeded values are fresh.
    if args.mode == "seeded":
        _preflight_refresh_google_token()

    extra_gateway_env: dict[str, str] = {}
    for env_name in mode_cfg["extra_gateway_env_names"]:
        value = env_str(env_name)
        if value:
            extra_gateway_env[env_name] = value

    stack = await start_reborn_gateway_stack(
        venv_dir=args.venv,
        owner_user_id=mode_cfg["owner_user_id"],
        temp_prefix=mode_cfg["temp_prefix"],
        gateway_token_prefix=mode_cfg["gateway_token_prefix"],
        extra_gateway_env=extra_gateway_env,
        rewrite_google_oauth=(args.mode == "seeded"),
        log_dir=args.output_dir,
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


# Secrets that the CI workflow materialises to per-secret files under
# `$RUNNER_TEMP/auth-secrets/` instead of declaring as job-level `env:`,
# so that accidental log-masking bypasses and subprocess env dumps can't
# spill them. See `.github/workflows/live-canary.yml` — the Materialize
# step writes each file and exports `<NAME>_PATH`. `_hydrate_secrets`
# below reads each file back into `os.environ` so downstream code and
# subprocesses (notably `mock_llm.py`, which inherits the parent env)
# see the raw value without every call site having to know about the
# path-based alternative.
_HYDRATED_SECRET_NAMES: tuple[str, ...] = (
    "AUTH_LIVE_GOOGLE_ACCESS_TOKEN",
    "AUTH_LIVE_GOOGLE_REFRESH_TOKEN",
    "AUTH_LIVE_GITHUB_TOKEN",
    "AUTH_LIVE_NOTION_ACCESS_TOKEN",
    "AUTH_LIVE_NOTION_REFRESH_TOKEN",
    "AUTH_LIVE_NOTION_CLIENT_SECRET",
    "GOOGLE_OAUTH_CLIENT_SECRET",
    "GITHUB_OAUTH_CLIENT_SECRET",
    "AUTH_BROWSER_GOOGLE_PASSWORD",
    "AUTH_BROWSER_GITHUB_PASSWORD",
    "AUTH_BROWSER_NOTION_PASSWORD",
)


def _hydrate_secrets() -> None:
    """Read each `<NAME>_PATH`-materialised secret into `os.environ`.

    Leaves any secret that is already set directly in env untouched —
    that's the local-dev path via `config.env`. In CI the job env
    deliberately omits the raw values; the Materialize step writes
    them to files and sets `<NAME>_PATH`, and this function pulls them
    back into the parent Python's env so the rest of the harness (and
    `mock_llm.py` as a subprocess) keeps working unchanged.
    """
    for name in _HYDRATED_SECRET_NAMES:
        if os.environ.get(name):
            continue
        value = env_secret(name)
        if value is not None:
            os.environ[name] = value


def main() -> int:
    args = parse_args()
    mode_cfg = MODE_CONFIG[args.mode]
    reexec_env = mode_cfg["reexec_env"]
    try:
        _hydrate_secrets()
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
