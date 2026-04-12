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
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
E2E_DIR = ROOT / "tests" / "e2e"
DEFAULT_VENV = E2E_DIR / ".venv"
DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-browser-canary"
OWNER_USER_ID = "auth-browser-owner"
SECRETS_MASTER_KEY = (
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
)


class CanaryError(RuntimeError):
    """Browser canary failure."""


@dataclass(frozen=True)
class BrowserProviderCase:
    key: str
    extension_name: str
    expected_extension_name: str
    install_kind: str | None
    install_url: str | None
    trigger_prompt: str
    expected_tool_name: str
    expected_text: str
    auth_extension_name: str | None = None


@dataclass
class ProbeResult:
    provider: str
    mode: str
    success: bool
    latency_ms: int
    details: dict[str, Any]


CASES: dict[str, BrowserProviderCase] = {
    "google": BrowserProviderCase(
        key="google",
        extension_name="gmail",
        expected_extension_name="gmail",
        install_kind=None,
        install_url=None,
        trigger_prompt="check gmail unread",
        expected_tool_name="gmail",
        expected_text="Gmail",
        auth_extension_name="gmail",
    ),
    "notion": BrowserProviderCase(
        key="notion",
        extension_name="notion",
        expected_extension_name="notion",
        install_kind="mcp_server",
        install_url=None,
        trigger_prompt="search notion for canary",
        expected_tool_name="notion_notion_search",
        expected_text="Notion search completed successfully.",
        auth_extension_name="notion",
    ),
    "github": BrowserProviderCase(
        key="github",
        extension_name="github",
        expected_extension_name="github",
        install_kind=None,
        install_url=None,
        trigger_prompt="read github issue owner/repo#1",
        expected_tool_name="github",
        expected_text="GitHub issue lookup completed successfully.",
        auth_extension_name="github",
    ),
}


def run(cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    rendered = " ".join(shlex.quote(part) for part in cmd)
    print(f"+ {rendered}", flush=True)
    subprocess.run(cmd, cwd=cwd or ROOT, env=env, check=True)


def venv_python(venv_dir: Path) -> Path:
    if os.name == "nt":
        return venv_dir / "Scripts" / "python.exe"
    return venv_dir / "bin" / "python"


def bootstrap_python(venv_dir: Path) -> Path:
    if not venv_dir.exists():
        run([sys.executable, "-m", "venv", str(venv_dir)])
    python = venv_python(venv_dir)
    run([str(python), "-m", "pip", "install", "--upgrade", "pip"])
    run([str(python), "-m", "pip", "install", "-e", str(E2E_DIR)])
    return python


def install_playwright(python: Path, mode: str) -> None:
    resolved = mode
    if mode == "auto":
        resolved = "with-deps" if os.environ.get("CI") else "plain"
    if resolved == "skip":
        return
    cmd = [str(python), "-m", "playwright", "install"]
    if resolved == "with-deps":
        cmd.append("--with-deps")
    cmd.append("chromium")
    run(cmd, cwd=E2E_DIR)


def cargo_build() -> None:
    run(["cargo", "build", "--no-default-features", "--features", "libsql"], cwd=ROOT)


def reserve_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_for_port_line(proc: subprocess.Popen[str], pattern: re.Pattern[str], timeout: float) -> re.Match[str]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = proc.stdout.readline()
        if not line:
            if proc.poll() is not None:
                raise CanaryError("mock_llm.py exited before printing its port")
            time.sleep(0.1)
            continue
        match = pattern.search(line)
        if match:
            return match
    raise CanaryError("Timed out waiting for mock_llm.py port announcement")


async def wait_for_ready(url: str, timeout: float = 60.0, interval: float = 0.5) -> None:
    import httpx

    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient(timeout=10.0) as client:
        while time.monotonic() < deadline:
            try:
                response = await client.get(url)
                if response.status_code == 200:
                    return
            except httpx.HTTPError:
                pass
            await asyncio.sleep(interval)
    raise CanaryError(f"Timed out waiting for readiness: {url}")


def stop_process(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    proc.send_signal(signal.SIGINT)
    try:
        proc.wait(timeout=10)
        return
    except subprocess.TimeoutExpired:
        proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def env_str(name: str, default: str | None = None) -> str | None:
    value = os.environ.get(name, default)
    if value is None:
        return None
    value = value.strip()
    return value or None


def storage_state_path(case_key: str) -> str | None:
    return env_str(f"AUTH_BROWSER_{case_key.upper()}_STORAGE_STATE_PATH")


def provider_username(case_key: str) -> str | None:
    return env_str(f"AUTH_BROWSER_{case_key.upper()}_USERNAME")


def provider_password(case_key: str) -> str | None:
    return env_str(f"AUTH_BROWSER_{case_key.upper()}_PASSWORD")


def github_issue_prompt() -> str | None:
    owner = env_str("AUTH_BROWSER_GITHUB_OWNER")
    repo = env_str("AUTH_BROWSER_GITHUB_REPO")
    issue_number = env_str("AUTH_BROWSER_GITHUB_ISSUE_NUMBER")
    if not owner or not repo or not issue_number:
        return None
    return f"read github issue {owner}/{repo}#{issue_number}"


def configured_cases(selected: list[str] | None) -> list[BrowserProviderCase]:
    names = selected or list(CASES)
    cases: list[BrowserProviderCase] = []
    for name in names:
        case = CASES[name]
        if name == "github":
            if not env_str("GITHUB_OAUTH_CLIENT_ID") or not env_str("GITHUB_OAUTH_CLIENT_SECRET"):
                continue
            prompt = github_issue_prompt()
            if not prompt:
                continue
            case = replace(case, trigger_prompt=prompt)
        if storage_state_path(name) or provider_username(name):
            cases.append(case)
    return cases


def load_e2e_helpers() -> tuple[Any, Any]:
    sys.path.insert(0, str(E2E_DIR))
    from helpers import SEL, send_chat_and_wait_for_terminal_message

    return SEL, send_chat_and_wait_for_terminal_message


async def api_request(
    method: str,
    base_url: str,
    path: str,
    *,
    token: str,
    json_body: Any | None = None,
    timeout: float = 30.0,
) -> Any:
    import httpx

    headers = {"Authorization": f"Bearer {token}"}
    async with httpx.AsyncClient(timeout=timeout) as client:
        response = await client.request(
            method,
            f"{base_url}{path}",
            headers=headers,
            json=json_body,
        )
    return response


async def install_extension(base_url: str, token: str, case: BrowserProviderCase) -> None:
    payload: dict[str, Any] = {"name": case.extension_name}
    if case.install_kind:
        payload["kind"] = case.install_kind
    if case.install_url:
        payload["url"] = case.install_url
    response = await api_request(
        "POST",
        base_url,
        "/api/extensions/install",
        token=token,
        json_body=payload,
        timeout=180,
    )
    if response.status_code != 200:
        raise CanaryError(f"Install failed for {case.key}: {response.status_code} {response.text}")
    body = response.json()
    if not body.get("success"):
        raise CanaryError(f"Install failed for {case.key}: {body}")


async def get_extension(base_url: str, token: str, name: str) -> dict[str, Any] | None:
    response = await api_request("GET", base_url, "/api/extensions", token=token, timeout=30)
    response.raise_for_status()
    for extension in response.json().get("extensions", []):
        if extension["name"] == name:
            return extension
    return None


async def wait_for_extension_state(
    base_url: str,
    token: str,
    name: str,
    *,
    authenticated: bool | None = None,
    active: bool | None = None,
    timeout: float = 60.0,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        extension = await get_extension(base_url, token, name)
        if extension is not None:
            if authenticated is not None and extension.get("authenticated") != authenticated:
                await asyncio.sleep(0.5)
                continue
            if active is not None and extension.get("active") != active:
                await asyncio.sleep(0.5)
                continue
            return extension
        await asyncio.sleep(0.5)
    raise CanaryError(f"Timed out waiting for extension state: {name}")


async def open_gateway_page(browser: Any, base_url: str, token: str, storage_state: str | None) -> tuple[Any, Any]:
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


async def responses_probe(base_url: str, token: str, case: BrowserProviderCase) -> ProbeResult:
    started = time.perf_counter()
    response = await api_request(
        "POST",
        base_url,
        "/v1/responses",
        token=token,
        json_body={"model": "default", "input": case.trigger_prompt},
        timeout=180,
    )
    latency_ms = int((time.perf_counter() - started) * 1000)
    if response.status_code != 200:
        return ProbeResult(
            provider=case.key,
            mode="responses_api",
            success=False,
            latency_ms=latency_ms,
            details={"status_code": response.status_code, "body": response.text[:1000]},
        )
    body = response.json()
    tool_names = [item.get("name") for item in body.get("output", []) if item.get("type") == "function_call"]
    tool_outputs = [item.get("output", "") for item in body.get("output", []) if item.get("type") == "function_call_output"]
    text = "\n".join(
        content.get("text", "")
        for item in body.get("output", [])
        if item.get("type") == "message"
        for content in item.get("content", [])
        if content.get("type") == "output_text"
    )
    success = (
        body.get("status") == "completed"
        and case.expected_tool_name in tool_names
        and bool(tool_outputs)
        and case.expected_text in text
    )
    return ProbeResult(
        provider=case.key,
        mode="responses_api",
        success=success,
        latency_ms=latency_ms,
        details={
            "status": body.get("status"),
            "tool_names": tool_names,
            "tool_outputs": tool_outputs,
            "response_text": text,
            "error": body.get("error"),
        },
    )


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


def write_results(output_dir: Path, results: list[ProbeResult], base_url: str) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    results_path = output_dir / "results.json"
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "base_url": base_url,
        "results": [asdict(result) for result in results],
    }
    results_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return results_path


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
        choices=sorted(CASES),
        help="Limit the run to selected browser-consent providers.",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="List configured browser-consent cases and exit.",
    )
    return parser.parse_args()


async def async_main(args: argparse.Namespace) -> int:
    cases = configured_cases(args.case)
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

    selectors, send_chat_and_wait_for_terminal_message_fn = load_e2e_helpers()
    from playwright.async_api import async_playwright

    mock_llm_port = reserve_loopback_port()
    mock_llm_proc = subprocess.Popen(
        [str(venv_python(args.venv)), str(E2E_DIR / "mock_llm.py"), "--port", str(mock_llm_port)],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    gateway_proc: subprocess.Popen[str] | None = None

    with tempfile.TemporaryDirectory(prefix="ironclaw-browser-auth-db-") as db_tmp, tempfile.TemporaryDirectory(
        prefix="ironclaw-browser-auth-home-"
    ) as home_tmp, tempfile.TemporaryDirectory(prefix="ironclaw-browser-auth-tools-") as tools_tmp, tempfile.TemporaryDirectory(
        prefix="ironclaw-browser-auth-channels-"
    ) as channels_tmp:
        try:
            match = wait_for_port_line(mock_llm_proc, re.compile(r"MOCK_LLM_PORT=(\d+)"), 30.0)
            mock_llm_url = f"http://127.0.0.1:{match.group(1)}"
            await wait_for_ready(f"{mock_llm_url}/v1/models", timeout=30.0)

            gateway_port = reserve_loopback_port()
            http_port = reserve_loopback_port()
            gateway_token = f"browser-auth-{int(time.time())}"
            env = {
                "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                "HOME": home_tmp,
                "IRONCLAW_BASE_DIR": str(Path(home_tmp) / ".ironclaw"),
                "RUST_LOG": os.environ.get("RUST_LOG", "ironclaw=info"),
                "RUST_BACKTRACE": "1",
                "IRONCLAW_OWNER_ID": OWNER_USER_ID,
                "GATEWAY_ENABLED": "true",
                "GATEWAY_HOST": "127.0.0.1",
                "GATEWAY_PORT": str(gateway_port),
                "GATEWAY_AUTH_TOKEN": gateway_token,
                "GATEWAY_USER_ID": OWNER_USER_ID,
                "HTTP_HOST": "127.0.0.1",
                "HTTP_PORT": str(http_port),
                "CLI_ENABLED": "false",
                "LLM_BACKEND": "openai_compatible",
                "LLM_BASE_URL": mock_llm_url,
                "LLM_MODEL": "mock-model",
                "DATABASE_BACKEND": "libsql",
                "LIBSQL_PATH": str(Path(db_tmp) / "browser-auth.db"),
                "SECRETS_MASTER_KEY": SECRETS_MASTER_KEY,
                "SANDBOX_ENABLED": "false",
                "SKILLS_ENABLED": "true",
                "ROUTINES_ENABLED": "false",
                "HEARTBEAT_ENABLED": "false",
                "EMBEDDING_ENABLED": "false",
                "WASM_ENABLED": "true",
                "WASM_TOOLS_DIR": tools_tmp,
                "WASM_CHANNELS_DIR": channels_tmp,
                "ONBOARD_COMPLETED": "true",
            }
            if env_str("GOOGLE_OAUTH_CLIENT_ID"):
                env["GOOGLE_OAUTH_CLIENT_ID"] = env_str("GOOGLE_OAUTH_CLIENT_ID") or ""
            if env_str("GOOGLE_OAUTH_CLIENT_SECRET"):
                env["GOOGLE_OAUTH_CLIENT_SECRET"] = env_str("GOOGLE_OAUTH_CLIENT_SECRET") or ""
            if env_str("GITHUB_OAUTH_CLIENT_ID"):
                env["GITHUB_OAUTH_CLIENT_ID"] = env_str("GITHUB_OAUTH_CLIENT_ID") or ""
            if env_str("GITHUB_OAUTH_CLIENT_SECRET"):
                env["GITHUB_OAUTH_CLIENT_SECRET"] = env_str("GITHUB_OAUTH_CLIENT_SECRET") or ""

            ironclaw_binary = ROOT / "target" / "debug" / "ironclaw"
            gateway_proc = subprocess.Popen(
                [str(ironclaw_binary), "--no-onboard"],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                env=env,
            )
            base_url = f"http://127.0.0.1:{gateway_port}"
            await wait_for_ready(f"{base_url}/api/health", timeout=60.0)

            for case in cases:
                await install_extension(base_url, gateway_token, case)
                await wait_for_extension_state(base_url, gateway_token, case.expected_extension_name, timeout=30.0)

            results: list[ProbeResult] = []
            async with async_playwright() as playwright:
                browser = await playwright.chromium.launch(headless=env_str("HEADED") != "1")
                try:
                    for case in cases:
                        results.extend(
                            await browser_probe(
                                browser,
                                base_url,
                                gateway_token,
                                case,
                                selectors,
                                send_chat_and_wait_for_terminal_message_fn,
                                args.output_dir,
                            )
                        )
                        if any(result.provider == case.key and not result.success for result in results):
                            continue
                        results.append(await responses_probe(base_url, gateway_token, case))
                finally:
                    await browser.close()

            results_path = write_results(args.output_dir, results, base_url)
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
            if gateway_proc is not None:
                stop_process(gateway_proc)
            stop_process(mock_llm_proc)


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
