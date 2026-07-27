"""Shared Reborn WebUI v2 E2E harness.

The legacy Playwright suite has mature shared fixtures in ``conftest.py`` for
the ``ironclaw`` gateway. Reborn WebUI v2 is a different product surface: it
boots ``ironclaw serve``, serves the React SPA at the root path, and uses
``/api/webchat/v2/*`` endpoints. Keep that setup here so browser and served API
scenarios exercise the real Reborn binary without duplicating process plumbing.
"""

import asyncio
import json
import os
import signal
import socket
import uuid
from pathlib import Path

import httpx
import pytest

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2, wait_for_ready

USER_ID = "reborn-v2-e2e-user"
DEFAULT_PROFILE = "local-dev"
YOLO_PROFILE = "local-dev-yolo"
DEFAULT_MODEL = "mock-model"
VISION_MODEL = "gpt-4o"
ACCEPTED_SEND_OUTCOMES = {"submitted", "already_submitted"}

# Shared tenant secret for the test-tools/market-data fixture (test-tools/README.md).
# `IRONCLAW_REBORN_DEV_SECRET__<handle>` is read once at `serve` boot, so it must
# be present in the process env before start — see
# reborn_v2_private_installs_yolo_server below.
MARKET_DATA_DEV_SECRET = "e2e-market-data-shared-key"


def find_free_port() -> int:
    """Ask the OS for an available loopback port as a startup hint."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    """Signal a subprocess and wait for exit without re-reading stdio pipes."""
    if proc.returncode is not None:
        return
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        await proc.wait()
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)


def write_config_toml(
    path: Path,
    mock_llm_server: str,
    profile: str = DEFAULT_PROFILE,
    model: str = DEFAULT_MODEL,
) -> None:
    """Seed a sparse Reborn config that selects the mock OpenAI-compatible LLM."""
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "{profile}"

[identity]
default_owner = "{USER_ID}"
tenant = "reborn-v2-e2e"
default_agent = "reborn-v2-e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "{model}"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"
""",
        encoding="utf-8",
    )


async def start_reborn_webui_v2_server(
    *,
    ironclaw_reborn_binary: str,
    mock_llm_server: str,
    home_dir: Path,
    profile: str = DEFAULT_PROFILE,
    model: str = DEFAULT_MODEL,
    log_prefix: str = "reborn-v2",
    extra_env: dict[str, str] | None = None,
) -> tuple[object, str]:
    """Start ``ironclaw serve`` and return ``(process, base_url)``."""
    reborn_home = home_dir / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    write_config_toml(
        reborn_home / "config.toml",
        mock_llm_server,
        profile=profile,
        model=model,
    )

    proc = None
    last_stderr = ""
    last_port = None

    for attempt in range(1, 4):
        port = find_free_port()
        last_port = port
        stdout_path = home_dir / f"{log_prefix}-attempt-{attempt}.stdout.log"
        stderr_path = home_dir / f"{log_prefix}-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": profile,
            "IRONCLAW_REBORN_WEBUI_TOKEN": REBORN_V2_AUTH_TOKEN,
            "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
            "MOCK_LLM_API_KEY": "mock-api-key",
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_LOG": "ironclaw=warn,ironclaw_runner=warn",
            "RUST_BACKTRACE": "1",
        }
        if extra_env:
            env.update(extra_env)
        forward_coverage_env(env)

        args = [
            ironclaw_reborn_binary,
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
        ]
        if profile == YOLO_PROFILE:
            args.insert(2, "--confirm-host-access")

        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            proc = await asyncio.create_subprocess_exec(
                *args,
                stdin=asyncio.subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                env=env,
            )
        base_url = f"http://127.0.0.1:{port}"

        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            return proc, base_url
        except TimeoutError:
            if proc.returncode is None:
                await stop_process(proc, timeout=2)
            last_stderr = read_log(stderr_path)
            proc = None

    pytest.fail(
        f"Reborn WebUI v2 server failed to start after 3 attempts.\n"
        f"Last attempted port: {last_port}\n"
        f"stderr:\n{last_stderr}"
    )


async def close_reborn_server(proc) -> None:
    if proc is not None and proc.returncode is None:
        await stop_process(proc, sig=signal.SIGINT, timeout=10)
        if proc.returncode is None:
            await stop_process(proc, sig=signal.SIGTERM, timeout=5)


async def kill_reborn_server(proc) -> None:
    """Hard-kill (SIGKILL) the reborn process, skipping graceful shutdown entirely.

    Used by durability scenarios that need to prove on-disk state survives an
    unclean process death, as opposed to `close_reborn_server`'s SIGINT/SIGTERM path.
    """
    if proc is not None and proc.returncode is None:
        await stop_process(proc, sig=signal.SIGKILL, timeout=5)


async def enable_reborn_global_auto_approve(
    base_url: str, *, token: str = REBORN_V2_AUTH_TOKEN
) -> None:
    """Enable the Tools settings global auto-approve switch for this test user."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers(token)) as client:
        response = await client.post(
            f"{base_url}/api/webchat/v2/settings/tools",
            json={"enabled": True},
            timeout=15,
        )
        response.raise_for_status()


@pytest.fixture(scope="module")
async def reborn_v2_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start ``ironclaw serve`` with the default local-dev profile."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=DEFAULT_PROFILE,
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_yolo_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start ``ironclaw serve`` with auto-approval local-dev-yolo profile."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-yolo-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-yolo",
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_private_installs_yolo_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Yolo-profile server with the market-data tenant-shared dev secret seeded.

    Used by the private-tool-installs scenario (#5459 P1): auto-approve so
    installed third-party WASM capabilities dispatch without an approval
    gate, plus the market-data fixture's shared API key present at boot.
    """
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-private-installs-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-private-installs-yolo",
        extra_env={"IRONCLAW_REBORN_DEV_SECRET__market_data_api_key": MARKET_DATA_DEV_SECRET},
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture
async def reborn_v2_restartable_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Start/stop Reborn against one persistent home directory.

    `stop(hard=True)` SIGKILLs the process instead of shutting it down
    gracefully, for durability scenarios that need to prove on-disk state
    survives an unclean death — the caller can read the killed PID off
    `state["proc"].pid` beforehand for a post-kill leak check.
    """
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-restartable-home")
    state = {"proc": None, "base_url": None}

    async def start() -> str:
        proc, base_url = await start_reborn_webui_v2_server(
            ironclaw_reborn_binary=ironclaw_reborn_binary,
            mock_llm_server=mock_llm_server,
            home_dir=home_dir,
            profile=DEFAULT_PROFILE,
            log_prefix="reborn-v2-restartable",
        )
        state["proc"] = proc
        state["base_url"] = base_url
        return base_url

    async def stop(*, hard: bool = False) -> None:
        if hard:
            await kill_reborn_server(state["proc"])
        else:
            await close_reborn_server(state["proc"])
        state["proc"] = None

    await start()
    try:
        yield state, start, stop
    finally:
        await stop()


@pytest.fixture(scope="module")
async def reborn_v2_loop_limited_yolo_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Start Reborn yolo mode with a low planned-profile loop budget."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-loop-limited-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-loop-limited-yolo",
        extra_env={
            "IRONCLAW_REBORN_PLANNED_DEFAULT_ITERATION_LIMIT": "1",
        },
    )
    await enable_reborn_global_auto_approve(base_url)
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_vision_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start Reborn with a vision-classified model id backed by the mock LLM."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-vision-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=DEFAULT_PROFILE,
        model=VISION_MODEL,
        log_prefix="reborn-v2-vision",
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_browser():
    """Chromium instance for Reborn v2 tests, independent of the legacy gateway."""
    from playwright.async_api import Error as PlaywrightError
    from playwright.async_api import async_playwright

    headless = os.environ.get("HEADED", "").strip() not in ("1", "true")
    async with async_playwright() as p:
        browser = None
        for attempt in range(3):
            try:
                browser = await p.chromium.launch(headless=headless, timeout=60000)
                break
            except PlaywrightError:
                if attempt == 2:
                    raise
                await asyncio.sleep(1)
        yield browser
        await browser.close()


@pytest.fixture
async def reborn_v2_page(reborn_v2_server, reborn_v2_browser):
    """Fresh authenticated page on the Reborn v2 SPA."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await open_reborn_v2_page(page, reborn_v2_server)
    yield page
    await context.close()


@pytest.fixture
async def reborn_v2_yolo_page(reborn_v2_yolo_server, reborn_v2_browser):
    """Fresh authenticated yolo-profile page with downloads enabled."""
    context = await reborn_v2_browser.new_context(
        viewport={"width": 1280, "height": 720}, accept_downloads=True
    )
    page = await context.new_page()
    await open_reborn_v2_page(page, reborn_v2_yolo_server)
    yield page
    await context.close()


@pytest.fixture
async def reborn_v2_vision_page(reborn_v2_vision_server, reborn_v2_browser):
    """Fresh authenticated page backed by a vision-classified mock model."""
    context = await reborn_v2_browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await open_reborn_v2_page(page, reborn_v2_vision_server)
    yield page
    await context.close()


async def open_reborn_v2_page(page, base_url: str, path: str = "/") -> None:
    separator = "&" if "?" in path else "?"
    await page.goto(f"{base_url}{path}{separator}token={REBORN_V2_AUTH_TOKEN}")
    await page.wait_for_selector(SEL_V2["chat_composer"], timeout=15000)


def reborn_bearer_headers(token: str = REBORN_V2_AUTH_TOKEN) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


async def fetch_extension_oauth_requirement(
    client: httpx.AsyncClient,
    base_url: str,
    package_id: str,
) -> dict:
    """Read the opaque OAuth requirement declared by an installed manifest."""
    response = await client.get(
        f"{base_url}/api/webchat/v2/extensions/{package_id}/setup",
        timeout=15,
    )
    response.raise_for_status()
    requirements = [
        secret
        for secret in response.json().get("secrets", [])
        if (secret.get("setup") or {}).get("kind") == "oauth"
    ]
    assert len(requirements) == 1, (
        f"expected exactly one manifest-declared OAuth requirement for {package_id}; "
        f"got {requirements}"
    )
    return requirements[0]


def client_action_id() -> str:
    """Idempotency key accepted by ``product_surface_inbound::parse_client_action_id``."""
    return str(uuid.uuid4())


async def create_thread(client: httpx.AsyncClient, base_url: str) -> str:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads",
        json={"client_action_id": client_action_id()},
        timeout=15,
    )
    response.raise_for_status()
    return response.json()["thread"]["thread_id"]


async def _submit_message(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> dict:
    response = await client.post(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
        json={"client_action_id": client_action_id(), "content": content},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text
    return response.json()


async def send_message(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> dict:
    body = await _submit_message(client, base_url, thread_id, content)
    outcome = body.get("outcome")
    assert outcome in ACCEPTED_SEND_OUTCOMES, (
        f"Message was not accepted for a run; outcome={outcome!r}, body={body}"
    )
    return body


async def fetch_timeline(client: httpx.AsyncClient, base_url: str, thread_id: str) -> dict:
    response = await client.get(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/timeline",
        timeout=15,
    )
    response.raise_for_status()
    return response.json()


async def wait_for_assistant_message(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 45.0,
) -> dict:
    """Poll the timeline until a finalized assistant message appears."""
    last_timeline: dict = {}
    for _ in range(int(timeout * 2)):
        try:
            last_timeline = await fetch_timeline(client, base_url, thread_id)
        except httpx.HTTPError:
            await asyncio.sleep(0.5)
            continue
        finalized = [
            message
            for message in last_timeline.get("messages", [])
            if message.get("kind") == "assistant"
            and message.get("status") == "finalized"
            and (message.get("content") or "").strip()
        ]
        if finalized:
            return finalized[-1]
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"Timed out waiting for a finalized assistant message in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )


def capability_preview_payload(message: dict) -> dict | None:
    """Parse a `capability_display_preview` timeline message's JSON content.

    Returns `None` for any other message kind.
    """
    if message.get("kind") != "capability_display_preview":
        return None
    content = message.get("content")
    assert isinstance(content, str), f"preview content must be a string: {message!r}"
    try:
        return json.loads(content)
    except json.JSONDecodeError as error:
        raise AssertionError(f"preview content is not valid JSON: {content!r}") from error


async def wait_for_capability_preview(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    capability_id: str,
    *,
    output_fragment: str | None = None,
    timeout: float = 45.0,
) -> dict:
    """Poll the timeline until a `capability_display_preview` for `capability_id`
    appears (optionally containing `output_fragment` in its output)."""
    last_timeline: dict = {}
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        last_timeline = await fetch_timeline(client, base_url, thread_id)
        for message in last_timeline.get("messages", []):
            preview = capability_preview_payload(message)
            if not preview or preview.get("capability_id") != capability_id:
                continue
            output = preview.get("output_preview") or preview.get("output_summary") or ""
            if output_fragment and output_fragment.lower() not in output.lower():
                continue
            return preview
        await asyncio.sleep(0.25)

    raise AssertionError(
        f"Timed out waiting for {capability_id!r} preview in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )


def finalized_assistant_count(timeline: dict) -> int:
    return sum(
        1
        for message in timeline.get("messages", [])
        if message.get("kind") == "assistant"
        and message.get("status") == "finalized"
        and (message.get("content") or "").strip()
    )


async def send_and_settle(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    content: str,
    expected: int,
) -> None:
    """Send a text turn and wait until ``expected`` assistant replies finalize."""
    submit_body: dict = {}
    last_submit_error = None
    for _ in range(12):
        try:
            submit_body = await _submit_message(client, base_url, thread_id, content)
            last_submit_error = None
        except httpx.HTTPError as error:
            last_submit_error = error
            await asyncio.sleep(0.5)
            continue
        outcome = submit_body.get("outcome")
        if outcome in ACCEPTED_SEND_OUTCOMES:
            break
        if outcome == "rejected_busy":
            await asyncio.sleep(0.5)
            continue
        raise AssertionError(
            f"Message was not accepted for a run; outcome={outcome!r}, body={submit_body}"
        )
    else:
        raise AssertionError(
            f"Thread {thread_id} remained busy before accepting a new turn; "
            f"last submit response: {submit_body}; last submit error: {last_submit_error!r}"
        )

    for _ in range(90):
        try:
            timeline = await fetch_timeline(client, base_url, thread_id)
        except httpx.HTTPError:
            await asyncio.sleep(0.5)
            continue
        if finalized_assistant_count(timeline) >= expected:
            return
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Thread {thread_id} did not reach {expected} finalized assistant replies; "
        f"submit response: {submit_body}"
    )
