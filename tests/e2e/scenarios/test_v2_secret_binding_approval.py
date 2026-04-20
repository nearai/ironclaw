"""E2E test: engine-v2 secret binding approval flow for user-authored skills.

This scenario covers the durable approval path that sits between
self-service `/api/secrets` management and runtime credential injection:

1. Start an isolated ENGINE_V2 gateway with a user-authored `github` skill
   whose credential mapping points at a localhost mock API.
2. Create a real member user through the admin API and act as that member.
3. Store `github_token` via the self-service `/api/secrets` endpoint.
4. Send an explicit `/github ...` prompt and assert the thread pauses with
   `pending_gate.gate_name == "secret_binding_approval"`.
5. Approve the gate and verify the HTTP call completes with injected auth and
   the approval appears in `/api/secrets`.
6. Re-run the same prompt and verify the persisted approval suppresses the gate.
7. Revoke the approval via `/api/secrets/{name}/approvals/revoke` and confirm
   the gate reappears on the next request.
"""

import asyncio
import json
import os
import signal
import socket
import tempfile
from pathlib import Path

import httpx
import pytest

import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import AUTH_TOKEN, api_get, api_post, wait_for_ready


ROOT = Path(__file__).resolve().parent.parent.parent.parent


def _forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def _stop_process(proc, sig=signal.SIGINT, timeout=5):
    async def _drain_pipes():
        try:
            await asyncio.wait_for(proc.communicate(), timeout=1)
        except (asyncio.TimeoutError, ValueError):
            pass

    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        await _drain_pipes()
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await proc.wait()
    await _drain_pipes()


async def _start_mock_api():
    from aiohttp import web

    received_tokens: list[str] = []
    valid_token_prefix = "ghp_"

    async def handle_issues_get(request: web.Request) -> web.Response:
        auth = request.headers.get("Authorization", "")
        if not auth.startswith("Bearer "):
            return web.json_response({"message": "Bad credentials"}, status=401)
        token = auth.split(" ", 1)[1]
        received_tokens.append(token)
        if not token.startswith(valid_token_prefix):
            return web.json_response({"message": "Bad credentials"}, status=401)
        return web.json_response(
            [
                {"number": 1, "title": "Improve onboarding funnel", "state": "open"},
                {"number": 2, "title": "Add usage analytics", "state": "open"},
            ]
        )

    async def handle_reset(_request: web.Request) -> web.Response:
        received_tokens.clear()
        return web.json_response({"ok": True})

    async def handle_received_tokens(_request: web.Request) -> web.Response:
        return web.json_response({"tokens": received_tokens})

    app = web.Application()
    app.router.add_get("/repos/{owner}/{repo}/issues", handle_issues_get)
    app.router.add_post("/__mock/reset", handle_reset)
    app.router.add_get("/__mock/received-tokens", handle_received_tokens)

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    actual_port = site._server.sockets[0].getsockname()[1]
    base_url = f"http://127.0.0.1:{actual_port}"
    return base_url, runner, received_tokens


TEST_SKILL_NAME = "github"


def _write_test_skill(skills_dir: str, mock_api_host: str) -> None:
    skill_dir = os.path.join(skills_dir, TEST_SKILL_NAME)
    os.makedirs(skill_dir, exist_ok=True)
    skill_content = f"""---
name: {TEST_SKILL_NAME}
version: \"1.0.0\"
keywords:
  - github
  - issues
  - pull request
tags:
  - github
  - api
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts:
      - \"{mock_api_host}\"
    setup_instructions: \"Open Settings → Secrets and add your github_token secret.\"
---
# GitHub API Skill

Use the `http` tool for GitHub REST API calls.
Credentials are injected automatically — never construct Authorization headers manually.
"""
    with open(os.path.join(skill_dir, "SKILL.md"), "w", encoding="utf-8") as handle:
        handle.write(skill_content)


@pytest.fixture
async def mock_api():
    base_url, runner, received_tokens = await _start_mock_api()
    try:
        yield {"url": base_url, "tokens": received_tokens}
    finally:
        await runner.cleanup()


@pytest.fixture
async def binding_approval_server(ironclaw_binary, mock_llm_server, mock_api):
    mock_api_url = mock_api["url"]
    mock_api_host = mock_api_url.replace("http://", "")

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/set_github_api_url",
            json={"url": mock_api_url},
            timeout=15,
        )
        assert response.status_code == 200

    db_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-v2-binding-approval-db-")
    home_tmpdir = tempfile.TemporaryDirectory(prefix="ironclaw-v2-binding-approval-home-")
    home_dir = home_tmpdir.name
    skills_dir = os.path.join(home_dir, ".ironclaw", "skills")
    os.makedirs(skills_dir, exist_ok=True)
    _write_test_skill(skills_dir, mock_api_host)

    socks = []
    for _ in range(2):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.bind(("127.0.0.1", 0))
        socks.append(sock)
    gateway_port = socks[0].getsockname()[1]
    http_port = socks[1].getsockname()[1]
    for sock in socks:
        sock.close()

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": home_dir,
        "IRONCLAW_BASE_DIR": os.path.join(home_dir, ".ironclaw"),
        "RUST_LOG": "ironclaw=debug",
        "RUST_BACKTRACE": "1",
        "ENGINE_V2": "true",
        "AGENT_AUTO_APPROVE_TOOLS": "true",
        "HTTP_ALLOW_LOCALHOST": "true",
        "SECRETS_MASTER_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "GATEWAY_ENABLED": "true",
        "GATEWAY_HOST": "127.0.0.1",
        "GATEWAY_PORT": str(gateway_port),
        "GATEWAY_AUTH_TOKEN": AUTH_TOKEN,
        "GATEWAY_USER_ID": "e2e-v2-binding-admin",
        "IRONCLAW_OWNER_ID": "e2e-v2-binding-admin",
        "HTTP_HOST": "127.0.0.1",
        "HTTP_PORT": str(http_port),
        "CLI_ENABLED": "false",
        "LLM_BACKEND": "openai_compatible",
        "LLM_BASE_URL": mock_llm_server,
        "LLM_MODEL": "mock-model",
        "DATABASE_BACKEND": "libsql",
        "LIBSQL_PATH": os.path.join(db_tmpdir.name, "binding-approval.db"),
        "SANDBOX_ENABLED": "false",
        "SKILLS_ENABLED": "true",
        "ROUTINES_ENABLED": "false",
        "HEARTBEAT_ENABLED": "false",
        "EMBEDDING_ENABLED": "false",
        "WASM_ENABLED": "false",
        "ONBOARD_COMPLETED": "true",
    }
    _forward_coverage_env(env)

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
        yield {"base_url": base_url, "mock_api_host": mock_api_host}
    except TimeoutError:
        if proc.returncode is None:
            await _stop_process(proc, timeout=2)
        stderr_bytes = b""
        if proc.stderr:
            try:
                stderr_bytes = await asyncio.wait_for(proc.stderr.read(8192), timeout=2)
            except asyncio.TimeoutError:
                pass
        pytest.fail(
            f"binding approval server failed to start on port {gateway_port}.\n"
            f"stderr: {stderr_bytes.decode('utf-8', errors='replace')}"
        )
    finally:
        if proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)
        db_tmpdir.cleanup()
        home_tmpdir.cleanup()


async def _api_request(
    method: str,
    base_url: str,
    path: str,
    *,
    token: str,
    json_body: dict | None = None,
    timeout: float = 20.0,
) -> httpx.Response:
    async with httpx.AsyncClient() as client:
        response = await client.request(
            method,
            f"{base_url}{path}",
            headers={"Authorization": f"Bearer {token}"},
            json=json_body,
            timeout=timeout,
        )
        return response


async def _create_member_user(base_url: str) -> dict:
    response = await api_post(
        base_url,
        "/api/admin/users",
        json={
            "display_name": "Binding Approval Member",
            "email": "binding-approval-member@example.test",
            "role": "member",
        },
        timeout=20,
    )
    assert response.status_code == 200, response.text
    return response.json()


async def _put_secret(base_url: str, token: str, name: str, value: str) -> dict:
    response = await _api_request(
        "PUT",
        base_url,
        f"/api/secrets/{name}",
        token=token,
        json_body={"value": value, "provider": "github"},
        timeout=20,
    )
    assert response.status_code == 200, response.text
    return response.json()


async def _list_secrets(base_url: str, token: str) -> dict:
    response = await _api_request("GET", base_url, "/api/secrets", token=token, timeout=20)
    assert response.status_code == 200, response.text
    return response.json()


async def _revoke_approval(base_url: str, token: str, secret_name: str, approval_id: str) -> dict:
    response = await _api_request(
        "POST",
        base_url,
        f"/api/secrets/{secret_name}/approvals/revoke",
        token=token,
        json_body={"approval_id": approval_id},
        timeout=20,
    )
    assert response.status_code == 200, response.text
    return response.json()


async def _list_skills(base_url: str, token: str) -> dict:
    response = await api_get(base_url, "/api/skills", token=token, timeout=20)
    assert response.status_code == 200, response.text
    return response.json()


async def _new_thread(base_url: str, token: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", token=token, timeout=15)
    assert response.status_code == 200, response.text
    return response.json()["id"]


async def _send_prompt(base_url: str, token: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        token=token,
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code == 202, response.text


async def _history(base_url: str, token: str, thread_id: str) -> dict:
    response = await api_get(
        base_url,
        f"/api/chat/history?thread_id={thread_id}",
        token=token,
        timeout=15,
    )
    assert response.status_code == 200, response.text
    return response.json()


async def _wait_for_pending_gate_named(
    base_url: str,
    token: str,
    thread_id: str,
    *,
    gate_name: str,
    timeout: float = 45.0,
) -> dict:
    last_history = {}
    for _ in range(int(timeout * 2)):
        history = await _history(base_url, token, thread_id)
        last_history = history
        pending_gate = history.get("pending_gate") or {}
        if (pending_gate.get("gate_name") or "").lower() == gate_name.lower():
            return pending_gate
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for pending gate {gate_name!r} on thread {thread_id}. "
        f"Last history: {json.dumps(last_history)[:2000]}"
    )


async def _wait_for_response_without_pending(
    base_url: str,
    token: str,
    thread_id: str,
    *,
    expect_substring: str,
    timeout: float = 45.0,
) -> dict:
    last_history = {}
    for _ in range(int(timeout * 2)):
        history = await _history(base_url, token, thread_id)
        last_history = history
        pending_gate = history.get("pending_gate")
        if pending_gate:
            raise AssertionError(
                f"Unexpected pending gate while waiting for response: {pending_gate}"
            )
        turns = history.get("turns", [])
        if turns:
            response = turns[-1].get("response") or ""
            if expect_substring.lower() in response.lower():
                return history
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for response containing {expect_substring!r}. "
        f"Last history: {json.dumps(last_history)[:2000]}"
    )


async def _resolve_approval_gate(base_url: str, token: str, thread_id: str, request_id: str) -> dict:
    response = await _api_request(
        "POST",
        base_url,
        "/api/chat/gate/resolve",
        token=token,
        json_body={
            "thread_id": thread_id,
            "request_id": request_id,
            "resolution": "approved",
            "always": False,
        },
        timeout=20,
    )
    assert response.status_code == 200, response.text
    return response.json()


async def _resolve_auth_gate(
    base_url: str,
    token: str,
    thread_id: str,
    request_id: str,
    credential: str,
) -> dict:
    response = await _api_request(
        "POST",
        base_url,
        "/api/chat/gate/resolve",
        token=token,
        json_body={
            "thread_id": thread_id,
            "request_id": request_id,
            "resolution": "credential_provided",
            "token": credential,
        },
        timeout=20,
    )
    assert response.status_code == 200, response.text
    return response.json()


def _find_secret(secrets_payload: dict, name: str) -> dict:
    for secret in secrets_payload.get("secrets", []):
        if secret.get("name") == name:
            return secret
    raise AssertionError(f"Secret {name!r} not found in payload: {secrets_payload}")


async def test_owner_secret_binding_approval_round_trip(binding_approval_server, mock_api):
    base_url = binding_approval_server["base_url"]
    mock_api_host = binding_approval_server["mock_api_host"]

    user_token = AUTH_TOKEN
    github_token = "ghp_binding_approval_smoke"

    skills_payload = await _list_skills(base_url, user_token)
    skill_names = [skill.get("name") for skill in skills_payload.get("skills", [])]
    assert TEST_SKILL_NAME in skill_names, skill_names

    await _api_request(
        "POST",
        mock_api["url"],
        "/__mock/reset",
        token=AUTH_TOKEN,
        json_body={},
        timeout=10,
    )

    first_thread = await _new_thread(base_url, user_token)
    await _send_prompt(
        base_url,
        user_token,
        first_thread,
        "/github list issues in nearai/ironclaw repo",
    )

    auth_gate = await _wait_for_pending_gate_named(
        base_url,
        user_token,
        first_thread,
        gate_name="authentication",
        timeout=60,
    )
    auth_resume_kind = auth_gate.get("resume_kind") or {}
    assert "Authentication" in auth_resume_kind, auth_gate

    auth_result = await _resolve_auth_gate(
        base_url,
        user_token,
        first_thread,
        auth_gate["request_id"],
        github_token,
    )
    assert auth_result.get("success") is True, auth_result

    pending_gate = await _wait_for_pending_gate_named(
        base_url,
        user_token,
        first_thread,
        gate_name="secret_binding_approval",
        timeout=60,
    )
    resume_kind = pending_gate.get("resume_kind") or {}
    assert "Approval" in resume_kind, pending_gate
    description = pending_gate.get("description") or ""
    assert "github_token" in description, pending_gate
    assert TEST_SKILL_NAME in description, pending_gate
    assert "127.0.0.1" in description or mock_api_host.split(":", 1)[0] in description, pending_gate
    assert mock_api["tokens"] == [], mock_api["tokens"]

    await _resolve_approval_gate(base_url, user_token, first_thread, pending_gate["request_id"])
    history_after_approve = await _wait_for_response_without_pending(
        base_url,
        user_token,
        first_thread,
        expect_substring="Improve onboarding funnel",
        timeout=60,
    )
    assert "Improve onboarding funnel" in history_after_approve["turns"][-1]["response"]
    assert github_token in mock_api["tokens"], mock_api["tokens"]

    secrets_after_approve = await _list_secrets(base_url, user_token)
    github_secret_after = _find_secret(secrets_after_approve, "github_token")
    approvals = github_secret_after.get("approvals") or []
    assert len(approvals) == 1, github_secret_after
    approval = approvals[0]
    assert approval["artifact_kind"] == "skill", approval
    assert approval["artifact_name"] == TEST_SKILL_NAME, approval
    assert approval["host"] == mock_api_host.split(":", 1)[0], approval
    assert approval["risk"] == "normal", approval

    mock_api["tokens"].clear()
    second_thread = await _new_thread(base_url, user_token)
    await _send_prompt(
        base_url,
        user_token,
        second_thread,
        "/github list issues in nearai/ironclaw repo",
    )
    second_history = await _wait_for_response_without_pending(
        base_url,
        user_token,
        second_thread,
        expect_substring="Improve onboarding funnel",
        timeout=60,
    )
    assert "Improve onboarding funnel" in second_history["turns"][-1]["response"]
    assert github_token in mock_api["tokens"], mock_api["tokens"]

    await _revoke_approval(base_url, user_token, "github_token", approval["approval_id"])
    secrets_after_revoke = await _list_secrets(base_url, user_token)
    github_secret_after_revoke = _find_secret(secrets_after_revoke, "github_token")
    assert github_secret_after_revoke.get("approvals") in (None, []), github_secret_after_revoke

    mock_api["tokens"].clear()
    third_thread = await _new_thread(base_url, user_token)
    await _send_prompt(
        base_url,
        user_token,
        third_thread,
        "/github list issues in nearai/ironclaw repo",
    )
    pending_again = await _wait_for_pending_gate_named(
        base_url,
        user_token,
        third_thread,
        gate_name="secret_binding_approval",
        timeout=60,
    )
    assert pending_again["request_id"] != pending_gate["request_id"], pending_again
    assert mock_api["tokens"] == [], mock_api["tokens"]
