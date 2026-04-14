#!/usr/bin/env python3
"""Browser-consent auth canary.

Starts a fresh local IronClaw instance, triggers real OAuth flows in the
browser, completes provider login/consent in Playwright, then verifies the
authenticated extension through both the browser chat UI and `/v1/responses`.

This suite is intentionally separate from the seeded-token live canary:
- this one proves browser consent flow correctness
- the seeded-token runner proves credential persistence / refresh reliability
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
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.auth_registry import BROWSER_CASES, BrowserProviderCase, configured_browser_cases
from scripts.live_canary.auth_runtime import create_responses_probe, install_extension, wait_for_extension_state
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

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-browser-canary"
OWNER_USER_ID = "auth-browser-owner"


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
    send_chat_and_wait_for_terminal_message_fn: Any,
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


async def browser_probe(
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
        auth_card = await trigger_auth_card(page, selectors, case, send_chat_and_wait_for_terminal_message_fn)
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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--venv",
        type=Path,
        default=DEFAULT_VENV,
        help=f"Virtualenv path (default: {DEFAULT_VENV})",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=f"Artifacts directory (default: {DEFAULT_OUTPUT_DIR})",
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
        choices=sorted(BROWSER_CASES),
        help="Limit the run to selected browser-consent providers.",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="List configured browser-consent cases and exit.",
    )
    return parser.parse_args()


async def async_main(args: argparse.Namespace) -> int:
    cases = configured_browser_cases(args.case)
    if args.list_cases:
        for case in cases:
            print(case.key)
        return 0
    if not cases:
        raise CanaryError(
            "No browser-consent cases are configured. Provide storage state or credentials for at least one provider."
        )

    if not args.skip_build:
        cargo_build()

    selectors, send_chat_and_wait_for_terminal_message_fn = load_e2e_helpers(
        "SEL",
        "send_chat_and_wait_for_terminal_message",
    )
    from playwright.async_api import async_playwright

    extra_gateway_env: dict[str, str] = {}
    for env_name in (
        "GOOGLE_OAUTH_CLIENT_ID",
        "GOOGLE_OAUTH_CLIENT_SECRET",
        "GITHUB_OAUTH_CLIENT_ID",
        "GITHUB_OAUTH_CLIENT_SECRET",
    ):
        value = env_str(env_name)
        if value:
            extra_gateway_env[env_name] = value

    stack = await start_gateway_stack(
        venv_dir=args.venv,
        owner_user_id=OWNER_USER_ID,
        temp_prefix="ironclaw-browser-auth",
        gateway_token_prefix="browser-auth",
        extra_gateway_env=extra_gateway_env,
    )
    try:
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
                        await browser_probe(
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

        results_path = write_results(args.output_dir, results, stack.base_url)
        failures = [result for result in results if not result.success]
        if failures:
            print(f"\nBrowser auth canary failures written to {results_path}", flush=True)
            for failure in failures:
                print(
                    f"- {failure.provider}/{failure.mode}: {json.dumps(failure.details, default=str)}",
                    flush=True,
                )
            return 1

        print(f"\nBrowser auth canary passed. Results: {results_path}", flush=True)
        return 0
    finally:
        stop_gateway_stack(stack)


def main() -> int:
    args = parse_args()
    try:
        if args.list_cases:
            return asyncio.run(async_main(args))
        if not args.skip_python_bootstrap and os.environ.get("AUTH_BROWSER_CANARY_REEXEC") != "1":
            python = bootstrap_python(args.venv)
            install_playwright(python, args.playwright_install)
            cmd = [str(python), str(Path(__file__).resolve()), *sys.argv[1:], "--skip-python-bootstrap"]
            env = os.environ.copy()
            env["AUTH_BROWSER_CANARY_REEXEC"] = "1"
            return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode
        if args.skip_python_bootstrap and not venv_python(args.venv).exists():
            raise CanaryError(
                f"Virtualenv Python not found at {venv_python(args.venv)}. Remove --skip-python-bootstrap or create it first."
            )
        return asyncio.run(async_main(args))
    except CanaryError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
