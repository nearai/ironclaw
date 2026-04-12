#!/usr/bin/env python3
"""Fresh-machine live auth canary.

Starts a clean local IronClaw instance, seeds real provider credentials into a
fresh database, then verifies provider-backed auth flows through both:
- the OpenAI Responses-compatible API (`/v1/responses`)
- the browser gateway UI (Playwright)

The LLM itself stays deterministic by reusing `tests/e2e/mock_llm.py` for tool
selection. The external dependency under test is the real provider API and the
stored credential/refresh behavior, not model output drift.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.auth_registry import SeededProviderCase, configured_seeded_cases
from scripts.live_canary.auth_runtime import activate_extension, install_extension, put_secret
from scripts.live_canary.common import (
    DEFAULT_SECRETS_MASTER_KEY,
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
OWNER_USER_ID = "auth-live-owner"
GOOGLE_SCOPE_DEFAULT = "gmail.modify gmail.compose calendar.events"


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


async def create_response_probe(
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
        and probe.expected_text in response_text
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


async def browser_probe(
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
            and probe.expected_text in result.get("text", "")
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


async def seed_live_credentials(base_url: str, token: str, db_path: Path) -> None:
    google_access = env_str("AUTH_LIVE_GOOGLE_ACCESS_TOKEN")
    google_refresh = env_str("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    if google_refresh and not google_access:
        raise CanaryError(
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN is required when AUTH_LIVE_GOOGLE_REFRESH_TOKEN is set"
        )
    if google_access or google_refresh:
        if google_access:
            await put_secret(
                base_url,
                token,
                user_id=OWNER_USER_ID,
                name="google_oauth_token",
                value=google_access,
                provider="google",
            )
        if google_refresh:
            await put_secret(
                base_url,
                token,
                user_id=OWNER_USER_ID,
                name="google_oauth_token_refresh_token",
                value=google_refresh,
                provider="google",
            )
        await put_secret(
            base_url,
            token,
            user_id=OWNER_USER_ID,
            name="google_oauth_token_scopes",
            value=env_str("AUTH_LIVE_GOOGLE_SCOPES", GOOGLE_SCOPE_DEFAULT),
            provider="google",
        )
        if google_refresh and env_str("AUTH_LIVE_FORCE_GOOGLE_REFRESH", "1") != "0":
            expire_secret_in_db(db_path, OWNER_USER_ID, "google_oauth_token")

    github_token = env_str("AUTH_LIVE_GITHUB_TOKEN")
    if github_token:
        await put_secret(
            base_url,
            token,
            user_id=OWNER_USER_ID,
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
            base_url,
            token,
            user_id=OWNER_USER_ID,
            name="mcp_notion_access_token",
            value=notion_access,
            provider="mcp:notion",
        )
    if notion_refresh:
        await put_secret(
            base_url,
            token,
            user_id=OWNER_USER_ID,
            name="mcp_notion_access_token_refresh_token",
            value=notion_refresh,
            provider="mcp:notion",
        )


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
        choices=("gmail", "google_calendar", "github", "notion"),
        help="Limit the run to specific providers. Repeat for multiple values.",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="Print the configured live-provider cases and exit.",
    )
    return parser.parse_args()


async def async_main(args: argparse.Namespace) -> int:
    probes = configured_seeded_cases(args.case)
    if args.list_cases:
        for probe in probes:
            print(probe.key)
        return 0

    if not probes:
        raise CanaryError(
            "No live provider cases are configured. Set at least one AUTH_LIVE_* credential env var."
        )

    if not args.skip_build:
        cargo_build()

    open_authed_page_fn, send_chat_and_wait_for_terminal_message_fn = load_e2e_helpers(
        "open_authed_page",
        "send_chat_and_wait_for_terminal_message",
    )
    from playwright.async_api import async_playwright

    extra_gateway_env: dict[str, str] = {}
    for env_name in ("GOOGLE_OAUTH_CLIENT_ID", "GOOGLE_OAUTH_CLIENT_SECRET"):
        value = env_str(env_name)
        if value:
            extra_gateway_env[env_name] = value

    stack = await start_gateway_stack(
        venv_dir=args.venv,
        owner_user_id=OWNER_USER_ID,
        secrets_master_key=DEFAULT_SECRETS_MASTER_KEY,
        temp_prefix="ironclaw-live-auth",
        gateway_token_prefix="auth-live",
        extra_gateway_env=extra_gateway_env,
    )
    try:
        await seed_live_credentials(stack.base_url, stack.gateway_token, stack.db_path)

        for probe in probes:
            ext = await install_extension(
                stack.base_url,
                stack.gateway_token,
                name=probe.extension_install_name,
                expected_display_name=probe.expected_display_name,
                install_kind=probe.install_kind,
                install_url=probe.install_url,
            )
            await activate_extension(
                stack.base_url,
                stack.gateway_token,
                extension_name=ext["name"],
                expected_display_name=ext.get("display_name") or probe.expected_display_name,
            )

        results: list[ProbeResult] = []
        for probe in probes:
            results.append(await create_response_probe(stack.base_url, stack.gateway_token, probe))

        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(headless=env_str("HEADED") != "1")
            try:
                for probe in probes:
                    if probe.browser_enabled:
                        results.append(
                            await browser_probe(
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

        results_path = write_results(args.output_dir, results, stack.base_url)
        failures = [result for result in results if not result.success]
        if failures:
            print(f"\nLive auth canary failures written to {results_path}", flush=True)
            for failure in failures:
                print(
                    f"- {failure.provider}/{failure.mode}: {json.dumps(failure.details, default=str)}",
                    flush=True,
                )
            return 1

        print(f"\nLive auth canary passed. Results: {results_path}", flush=True)
        return 0
    finally:
        stop_gateway_stack(stack)


def main() -> int:
    args = parse_args()
    try:
        if args.list_cases:
            return asyncio.run(async_main(args))
        if not args.skip_python_bootstrap and os.environ.get("AUTH_LIVE_CANARY_REEXEC") != "1":
            python = bootstrap_python(args.venv)
            install_playwright(python, args.playwright_install)
            cmd = [str(python), str(Path(__file__).resolve()), *sys.argv[1:], "--skip-python-bootstrap"]
            env = os.environ.copy()
            env["AUTH_LIVE_CANARY_REEXEC"] = "1"
            return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode
        if args.skip_python_bootstrap:
            python = venv_python(args.venv)
            if not python.exists() and os.environ.get("AUTH_LIVE_CANARY_REEXEC") != "1":
                raise CanaryError(
                    f"Virtualenv Python not found at {python}. Remove --skip-python-bootstrap or create it first."
                )
        return asyncio.run(async_main(args))
    except CanaryError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
