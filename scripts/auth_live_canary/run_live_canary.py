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
import re
import shlex
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
E2E_DIR = ROOT / "tests" / "e2e"
DEFAULT_VENV = E2E_DIR / ".venv"
DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-live-canary"
OWNER_USER_ID = "auth-live-owner"
SECRETS_MASTER_KEY = (
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
)


class CanaryError(RuntimeError):
    """Live canary failure."""


@dataclass(frozen=True)
class ProviderProbe:
    key: str
    extension_install_name: str
    expected_display_name: str
    response_prompt: str
    expected_tool_name: str
    expected_text: str
    browser_enabled: bool = False
    install_kind: str | None = None
    install_url: str | None = None
    shared_secret_name: str | None = None
    requires_refresh_seed: bool = False


@dataclass
class ProbeResult:
    provider: str
    mode: str
    success: bool
    latency_ms: int
    details: dict[str, Any] = field(default_factory=dict)


GOOGLE_SCOPE_DEFAULT = "gmail.modify gmail.compose calendar.events"


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


def configured_provider_probes() -> list[ProviderProbe]:
    probes: list[ProviderProbe] = []

    google_access = env_str("AUTH_LIVE_GOOGLE_ACCESS_TOKEN")
    google_refresh = env_str("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    if google_refresh and not google_access:
        raise CanaryError(
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN is required when AUTH_LIVE_GOOGLE_REFRESH_TOKEN is set"
        )
    if google_access:
        probes.append(
            ProviderProbe(
                key="gmail",
                extension_install_name="gmail",
                expected_display_name="Gmail",
                response_prompt="check gmail unread",
                expected_tool_name="gmail",
                expected_text="Gmail",
                browser_enabled=True,
                shared_secret_name="google_oauth_token",
                requires_refresh_seed=bool(google_refresh),
            )
        )
        probes.append(
            ProviderProbe(
                key="google_calendar",
                extension_install_name="google_calendar",
                expected_display_name="Google Calendar",
                response_prompt="list next calendar event",
                expected_tool_name="google_calendar",
                expected_text="Calendar check completed successfully.",
                shared_secret_name="google_oauth_token",
                requires_refresh_seed=False,
            )
        )

    if env_str("AUTH_LIVE_GITHUB_TOKEN"):
        owner = required_env("AUTH_LIVE_GITHUB_OWNER")
        repo = required_env("AUTH_LIVE_GITHUB_REPO")
        issue_number = required_env("AUTH_LIVE_GITHUB_ISSUE_NUMBER")
        probes.append(
            ProviderProbe(
                key="github",
                extension_install_name="github",
                expected_display_name="GitHub",
                response_prompt=f"read github issue {owner}/{repo}#{issue_number}",
                expected_tool_name="github",
                expected_text="GitHub issue lookup completed successfully.",
                browser_enabled=True,
                shared_secret_name="github_token",
            )
        )

    if env_str("AUTH_LIVE_NOTION_ACCESS_TOKEN"):
        query = required_env("AUTH_LIVE_NOTION_QUERY")
        probes.append(
            ProviderProbe(
                key="notion",
                extension_install_name="notion",
                expected_display_name="Notion",
                response_prompt=f"search notion for {query}",
                expected_tool_name="notion_notion_search",
                expected_text="Notion search completed successfully.",
                install_kind="mcp_server",
            )
        )

    return probes


def env_str(name: str, default: str | None = None) -> str | None:
    value = os.environ.get(name, default)
    if value is None:
        return None
    value = value.strip()
    return value or None


def required_env(name: str) -> str:
    value = env_str(name)
    if not value:
        raise CanaryError(f"{name} is required for the selected live-provider case")
    return value


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


async def put_secret(
    base_url: str,
    token: str,
    *,
    user_id: str,
    name: str,
    value: str,
    provider: str | None = None,
) -> None:
    payload: dict[str, Any] = {"value": value}
    if provider is not None:
        payload["provider"] = provider
    response = await api_request(
        "PUT",
        base_url,
        f"/api/admin/users/{user_id}/secrets/{name}",
        token=token,
        json_body=payload,
    )
    if response.status_code != 200:
        raise CanaryError(f"Failed to seed secret {name}: {response.status_code} {response.text}")


async def list_extensions(base_url: str, token: str) -> list[dict[str, Any]]:
    response = await api_request("GET", base_url, "/api/extensions", token=token, timeout=30)
    response.raise_for_status()
    return response.json().get("extensions", [])


async def wait_for_extension(
    base_url: str,
    token: str,
    *,
    expected_display_name: str,
    timeout: float = 60.0,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for ext in await list_extensions(base_url, token):
            if ext.get("display_name") == expected_display_name or ext.get("name") == expected_display_name:
                return ext
        await asyncio.sleep(0.5)
    raise CanaryError(f"Timed out waiting for extension {expected_display_name}")


async def install_extension(base_url: str, token: str, probe: ProviderProbe) -> dict[str, Any]:
    payload: dict[str, Any] = {"name": probe.extension_install_name}
    if probe.install_kind is not None:
        payload["kind"] = probe.install_kind
    if probe.install_url is not None:
        payload["url"] = probe.install_url
    response = await api_request(
        "POST",
        base_url,
        "/api/extensions/install",
        token=token,
        json_body=payload,
        timeout=180,
    )
    if response.status_code != 200:
        raise CanaryError(
            f"Install failed for {probe.key}: {response.status_code} {response.text}"
        )
    body = response.json()
    if not body.get("success"):
        raise CanaryError(f"Install failed for {probe.key}: {body}")
    return await wait_for_extension(
        base_url,
        token,
        expected_display_name=probe.expected_display_name,
    )


async def activate_extension(
    base_url: str,
    token: str,
    *,
    extension_name: str,
    timeout: float = 90.0,
) -> dict[str, Any]:
    response = await api_request(
        "POST",
        base_url,
        f"/api/extensions/{extension_name}/activate",
        token=token,
        timeout=60,
    )
    if response.status_code != 200:
        raise CanaryError(
            f"Activation failed for {extension_name}: {response.status_code} {response.text}"
        )
    body = response.json()
    if body.get("auth_url"):
        raise CanaryError(
            f"Activation unexpectedly required interactive auth for {extension_name}: {body['auth_url']}"
        )

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ext = await wait_for_extension(
            base_url,
            token,
            expected_display_name=body.get("display_name") or extension_name,
            timeout=5.0,
        )
        if ext.get("authenticated") and ext.get("active"):
            return ext
        await asyncio.sleep(0.5)

    ext = await wait_for_extension(
        base_url,
        token,
        expected_display_name=body.get("display_name") or extension_name,
        timeout=5.0,
    )
    raise CanaryError(f"Extension {extension_name} did not become active: {ext}")


async def create_response_probe(
    base_url: str,
    token: str,
    probe: ProviderProbe,
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
            details={
                "status_code": response.status_code,
                "body": response.text[:1000],
            },
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
    probe: ProviderProbe,
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
        if google_refresh and google_access and env_str("AUTH_LIVE_FORCE_GOOGLE_REFRESH", "1") != "0":
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


def write_results(output_dir: Path, results: list[ProbeResult], base_url: str) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    path = output_dir / "results.json"
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "base_url": base_url,
        "results": [asdict(result) for result in results],
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


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


def load_e2e_helpers() -> tuple[Any, Any]:
    sys.path.insert(0, str(E2E_DIR))
    from helpers import open_authed_page, send_chat_and_wait_for_terminal_message

    return open_authed_page, send_chat_and_wait_for_terminal_message


async def async_main(args: argparse.Namespace) -> int:
    probes = configured_provider_probes()
    if args.case:
        allowed = set(args.case)
        probes = [probe for probe in probes if probe.key in allowed]

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

    open_authed_page_fn, send_chat_and_wait_for_terminal_message_fn = load_e2e_helpers()
    from playwright.async_api import async_playwright

    mock_llm_port = reserve_loopback_port()
    mock_llm_proc = subprocess.Popen(
        [str(python), str(E2E_DIR / "mock_llm.py"), "--port", str(mock_llm_port)],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    gateway_proc: subprocess.Popen[str] | None = None

    with tempfile.TemporaryDirectory(prefix="ironclaw-live-auth-db-") as db_tmp, tempfile.TemporaryDirectory(
        prefix="ironclaw-live-auth-home-"
    ) as home_tmp, tempfile.TemporaryDirectory(prefix="ironclaw-live-auth-tools-") as tools_tmp, tempfile.TemporaryDirectory(
        prefix="ironclaw-live-auth-channels-"
    ) as channels_tmp:
        try:
            match = wait_for_port_line(
                mock_llm_proc,
                re.compile(r"MOCK_LLM_PORT=(\d+)"),
                timeout=30.0,
            )
            mock_llm_url = f"http://127.0.0.1:{match.group(1)}"
            await wait_for_ready(f"{mock_llm_url}/v1/models", timeout=30.0)

            gateway_port = reserve_loopback_port()
            http_port = reserve_loopback_port()
            gateway_token = f"auth-live-{uuid.uuid4().hex[:12]}"
            db_path = Path(db_tmp) / "auth-live.db"
            home_dir = Path(home_tmp)
            env = {
                "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                "HOME": str(home_dir),
                "IRONCLAW_BASE_DIR": str(home_dir / ".ironclaw"),
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
                "LIBSQL_PATH": str(db_path),
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
                env["GOOGLE_OAUTH_CLIENT_ID"] = required_env("GOOGLE_OAUTH_CLIENT_ID")
            if env_str("GOOGLE_OAUTH_CLIENT_SECRET"):
                env["GOOGLE_OAUTH_CLIENT_SECRET"] = required_env("GOOGLE_OAUTH_CLIENT_SECRET")

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

            await seed_live_credentials(base_url, gateway_token, db_path)

            installed: dict[str, str] = {}
            for probe in probes:
                ext = await install_extension(base_url, gateway_token, probe)
                installed[probe.key] = ext["name"]
                await activate_extension(
                    base_url,
                    gateway_token,
                    extension_name=ext["name"],
                )

            results: list[ProbeResult] = []
            for probe in probes:
                results.append(await create_response_probe(base_url, gateway_token, probe))

            async with async_playwright() as playwright:
                browser = await playwright.chromium.launch(headless=env_str("HEADED") != "1")
                try:
                    for probe in probes:
                        if probe.browser_enabled:
                            results.append(
                                await browser_probe(
                                browser,
                                base_url,
                                gateway_token,
                                probe,
                                args.output_dir,
                                open_authed_page_fn=open_authed_page_fn,
                                send_chat_and_wait_for_terminal_message_fn=send_chat_and_wait_for_terminal_message_fn,
                            )
                        )
                finally:
                    await browser.close()

            results_path = write_results(args.output_dir, results, base_url)
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
            if gateway_proc is not None:
                stop_process(gateway_proc)
            stop_process(mock_llm_proc)


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
