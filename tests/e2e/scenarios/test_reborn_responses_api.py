"""Reborn OpenAI-compatible Responses API E2E coverage."""

import asyncio
import json
import os
import signal
import socket
from pathlib import Path

import httpx
import pytest

from helpers import REBORN_V2_AUTH_TOKEN, wait_for_ready

USER_ID = "reborn-responses-e2e-user"
PROFILE = "local-dev"


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def _forward_coverage_env(env: dict[str, str]) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def _stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    if proc.returncode is not None:
        return
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)


async def _enable_reborn_global_auto_approve(base_url: str) -> None:
    async with httpx.AsyncClient(
        headers={"Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}"}
    ) as client:
        response = await client.post(
            f"{base_url}/api/webchat/v2/settings/tools",
            json={"enabled": True},
            timeout=15,
        )
        response.raise_for_status()


def _write_config_toml(path: Path, mock_llm_server: str) -> None:
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "{PROFILE}"

[identity]
default_owner = "{USER_ID}"
tenant = "reborn-responses-e2e"
default_agent = "reborn-responses-e2e-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "mock-model"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"
""",
        encoding="utf-8",
    )


@pytest.fixture(scope="module")
async def reborn_responses_server(
    ironclaw_reborn_openai_compat_binary, mock_llm_server, tmp_path_factory
):
    """Start `ironclaw-reborn serve` with `/v1/responses` mounted."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-responses-home")
    reborn_home = home_dir / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    _write_config_toml(reborn_home / "config.toml", mock_llm_server)

    proc = None
    base_url = None
    last_stderr = ""
    last_port = None

    for attempt in range(1, 4):
        port = _find_free_port()
        last_port = port
        stdout_path = home_dir / f"reborn-responses-attempt-{attempt}.stdout.log"
        stderr_path = home_dir / f"reborn-responses-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": PROFILE,
            "IRONCLAW_REBORN_WEBUI_TOKEN": REBORN_V2_AUTH_TOKEN,
            "IRONCLAW_REBORN_WEBUI_USER_ID": USER_ID,
            "MOCK_LLM_API_KEY": "mock-api-key",
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_LOG": "ironclaw=warn,ironclaw_reborn=warn",
            "RUST_BACKTRACE": "1",
        }
        _forward_coverage_env(env)

        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            proc = await asyncio.create_subprocess_exec(
                ironclaw_reborn_openai_compat_binary,
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                str(port),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                env=env,
            )
        base_url = f"http://127.0.0.1:{port}"

        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            await _enable_reborn_global_auto_approve(base_url)
            break
        except (TimeoutError, httpx.HTTPError):
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)
            last_stderr = _read_log(stderr_path)
            proc = None
    else:
        pytest.fail(
            "Reborn Responses API server failed to start after 3 attempts.\n"
            f"Last attempted port: {last_port}\n"
            f"stderr:\n{last_stderr}"
        )

    try:
        yield base_url
    finally:
        if proc is not None and proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


@pytest.fixture()
async def reborn_responses_client(reborn_responses_server):
    async with httpx.AsyncClient(
        base_url=reborn_responses_server,
        headers={
            "Authorization": f"Bearer {REBORN_V2_AUTH_TOKEN}",
            "Content-Type": "application/json",
        },
        timeout=120,
    ) as client:
        yield client


async def create_response(
    client: httpx.AsyncClient, path: str = "/v1/responses", **payload
) -> dict:
    body = {"model": "mock-model", **payload}
    response = await client.post(path, json=body)
    assert response.status_code == 200, response.text
    return response.json()


def _function_calls(response: dict) -> list[dict]:
    return [item for item in response.get("output", []) if item.get("type") == "function_call"]


def _function_call_outputs(response: dict) -> list[dict]:
    return [
        item
        for item in response.get("output", [])
        if item.get("type") == "function_call_output"
    ]


def _output_json(response: dict) -> str:
    return json.dumps(response.get("output", []), sort_keys=True)


def _request_tool_names(request: dict) -> set[str]:
    names: set[str] = set()
    for tool in request.get("tools", []):
        function = tool.get("function")
        if isinstance(function, dict) and function.get("name"):
            names.add(function["name"])
        elif tool.get("name"):
            names.add(tool["name"])
    return names


async def _mock_chat_requests(mock_llm_server: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=10) as client:
        response = await client.get(f"{mock_llm_server}/__mock/chat_requests")
    response.raise_for_status()
    return response.json()["requests"]


async def _reset_mock_chat_requests(mock_llm_server: str) -> None:
    async with httpx.AsyncClient(timeout=10) as client:
        response = await client.post(f"{mock_llm_server}/__mock/chat_requests/reset")
    response.raise_for_status()


def _lookup_weather_tool() -> dict:
    return {
        "type": "function",
        "name": "lookup_weather",
        "description": "Look up weather for a city.",
        "parameters": {
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
        },
    }


async def test_reborn_responses_non_streaming_text_input(reborn_responses_client):
    response = await create_response(
        reborn_responses_client,
        input="Say hello in exactly 3 words",
    )
    assert response["id"].startswith("resp_")
    assert response["status"] == "completed"
    assert len(response["output"]) > 0


async def test_reborn_responses_non_streaming_messages_input(reborn_responses_client):
    response = await create_response(
        reborn_responses_client,
        input=[
            {
                "type": "message",
                "role": "user",
                "content": "What is 2+2? Reply with just the number.",
            }
        ],
    )
    assert response["status"] == "completed"
    assert len(response["output"]) > 0


async def test_reborn_responses_api_v1_alias_accepts_untyped_message_input(
    reborn_responses_client,
):
    response = await create_response(
        reborn_responses_client,
        path="/api/v1/responses",
        input=[
            {
                "role": "user",
                "content": "What is 3+3? Reply with just the number.",
            }
        ],
    )

    assert response["status"] == "completed"
    assert response["id"].startswith("resp_")
    assert len(response["output"]) > 0


async def test_reborn_responses_continue_conversation(reborn_responses_client):
    first = await create_response(reborn_responses_client, input="Say hello")
    assert first["status"] == "completed"

    second = await create_response(
        reborn_responses_client,
        input="Now say goodbye",
        previous_response_id=first["id"],
    )
    assert second["status"] == "completed"
    assert second["id"] != first["id"]


async def test_reborn_responses_get_response_by_id(reborn_responses_client):
    response = await create_response(
        reborn_responses_client, input="Remember this: the sky is blue"
    )
    retrieved = await reborn_responses_client.get(f"/v1/responses/{response['id']}")
    assert retrieved.status_code == 200, retrieved.text
    data = retrieved.json()
    assert data["id"] == response["id"]
    assert len(data["output"]) > 0


async def test_reborn_responses_streaming_raw_sse(reborn_responses_client):
    async with reborn_responses_client.stream(
        "POST",
        "/v1/responses",
        json={"model": "mock-model", "input": "Say hi", "stream": True},
    ) as response:
        assert response.status_code == 200
        raw = ""
        async for line in response.aiter_lines():
            raw += line + "\n"
            if line == "data: [DONE]":
                break

    assert "event: response.created" in raw
    assert "event: response.completed" in raw


async def test_reborn_responses_context_injection_approval_and_rejection(
    reborn_responses_client,
):
    approved = await create_response(
        reborn_responses_client,
        input="Go ahead with the transfer",
        x_context={
            "notification_response": {
                "notification_id": "msg_456",
                "action": "approved",
                "original_signal": "convert_now",
                "score": 72,
            }
        },
        stream=False,
    )
    assert approved["status"] == "completed"
    assert len(approved["output"]) > 0

    rejected = await create_response(
        reborn_responses_client,
        input="Cancel it",
        x_context={
            "notification_response": {
                "notification_id": "msg_789",
                "action": "rejected",
            }
        },
        stream=False,
    )
    assert rejected["status"] == "completed"


async def test_reborn_responses_error_no_auth(reborn_responses_server):
    async with httpx.AsyncClient(timeout=10) as client:
        response = await client.post(
            f"{reborn_responses_server}/v1/responses",
            headers={"Content-Type": "application/json"},
            json={"model": "mock-model", "input": "hello"},
        )
    assert response.status_code == 401


async def test_reborn_responses_rejects_empty_input_items(reborn_responses_client):
    response = await reborn_responses_client.post(
        "/v1/responses",
        json={"model": "mock-model", "input": []},
    )
    assert response.status_code == 400


async def test_reborn_responses_rejects_empty_text_input(reborn_responses_client):
    response = await reborn_responses_client.post(
        "/v1/responses",
        json={"model": "mock-model", "input": ""},
    )
    assert response.status_code == 400
    assert response.json()["error"]["param"] == "input"


async def test_reborn_responses_lookup_and_cancel_missing_id_match_not_found_shape(
    reborn_responses_client,
):
    retrieve = await reborn_responses_client.get("/api/v1/responses/resp_missing")
    cancel = await reborn_responses_client.post("/api/v1/responses/resp_missing/cancel")

    assert retrieve.status_code == 404
    assert cancel.status_code == 404
    assert retrieve.json() == cancel.json()


async def test_reborn_responses_repeated_external_tools_round_trip(
    reborn_responses_client, mock_llm_server
):
    await _reset_mock_chat_requests(mock_llm_server)

    tools = [
        _lookup_weather_tool(),
        {
            "type": "function",
            "name": "lookup_time",
            "description": "Look up local time for a city.",
            "parameters": {
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"],
            },
        },
        {
            "type": "function",
            "name": "lookup_fact",
            "description": "Look up a brief fact about a topic.",
            "parameters": {
                "type": "object",
                "properties": {"topic": {"type": "string"}},
                "required": ["topic"],
            },
        },
    ]

    response = await create_response(
        reborn_responses_client,
        input="Run reborn external tool loop for Boston.",
        tools=tools,
    )

    outputs = {
        "lookup_weather": "weather:sunny 72F",
        "lookup_time": "time:09:30",
        "lookup_fact": "fact:harbor",
    }
    seen_call_ids: set[str] = set()
    seen_tool_calls: list[str] = []
    for _ in range(5):
        calls = [
            call for call in _function_calls(response) if call["call_id"] not in seen_call_ids
        ]
        if not calls:
            break

        submitted_outputs = []
        for call in calls:
            tool_name = call["name"]
            assert tool_name in outputs, response
            assert tool_name not in seen_tool_calls, response
            arguments = json.loads(call["arguments"])
            expected_arg = "topic" if tool_name == "lookup_fact" else "city"
            assert arguments[expected_arg] == "Boston"

            seen_call_ids.add(call["call_id"])
            seen_tool_calls.append(tool_name)
            submitted_outputs.append(
                {
                    "type": "function_call_output",
                    "call_id": call["call_id"],
                    "output": outputs[tool_name],
                }
            )

        response = await create_response(
            reborn_responses_client,
            previous_response_id=response["id"],
            input=submitted_outputs,
        )

    assert seen_tool_calls == ["lookup_weather", "lookup_time", "lookup_fact"]

    rendered_output = _output_json(response)
    assert "Reborn external tool loop complete" in rendered_output
    for tool_output in outputs.values():
        assert tool_output in rendered_output

    chat_requests = await _mock_chat_requests(mock_llm_server)
    assert len(chat_requests) >= 4

    expected_tool_names = set(outputs.keys())
    assert expected_tool_names.issubset(_request_tool_names(chat_requests[0]))

    forwarded_messages = json.dumps(
        [request.get("messages", []) for request in chat_requests],
        sort_keys=True,
    )
    assert "Run reborn external tool loop for Boston." in forwarded_messages
    for tool_output in outputs.values():
        assert tool_output in forwarded_messages


async def test_reborn_responses_external_tool_failure_output_reaches_llm(
    reborn_responses_client, mock_llm_server
):
    await _reset_mock_chat_requests(mock_llm_server)

    response = await create_response(
        reborn_responses_client,
        input="Run reborn external tool failure for Boston.",
        tools=[_lookup_weather_tool()],
    )
    calls = _function_calls(response)
    assert len(calls) == 1, response
    call = calls[0]
    assert call["name"] == "lookup_weather"
    assert json.loads(call["arguments"]) == {"city": "Boston"}

    failure_output = "ERROR: upstream weather service timed out"
    final = await create_response(
        reborn_responses_client,
        previous_response_id=response["id"],
        input=[
            {
                "type": "function_call_output",
                "call_id": call["call_id"],
                "output": failure_output,
            }
        ],
    )

    assert final["status"] == "completed"
    rendered_output = _output_json(final)
    assert "Reborn external tool failure observed" in rendered_output
    assert failure_output in rendered_output

    chat_requests = await _mock_chat_requests(mock_llm_server)
    forwarded_messages = json.dumps(
        [request.get("messages", []) for request in chat_requests],
        sort_keys=True,
    )
    assert failure_output in forwarded_messages


async def test_reborn_responses_rejects_wrong_external_tool_call_id(
    reborn_responses_client,
):
    response = await create_response(
        reborn_responses_client,
        input="Run reborn external tool failure for Boston.",
        tools=[_lookup_weather_tool()],
    )
    calls = _function_calls(response)
    assert len(calls) == 1, response
    assert calls[0]["call_id"] != "call_not_pending"

    rejected = await reborn_responses_client.post(
        "/v1/responses",
        json={
            "model": "mock-model",
            "previous_response_id": response["id"],
            "input": [
                {
                    "type": "function_call_output",
                    "call_id": "call_not_pending",
                    "output": "weather:sunny 72F",
                }
            ],
        },
    )

    assert rejected.status_code == 400
    body = rejected.json()
    assert body["error"]["param"] == "input.call_id"
    assert body["error"]["code"] == "invalid_request"


async def test_reborn_responses_mixed_internal_and_external_tools_same_assistant_response(
    reborn_responses_client,
    mock_llm_server,
):
    await _reset_mock_chat_requests(mock_llm_server)

    response = await create_response(
        reborn_responses_client,
        input="Run reborn mixed internal external tools for Boston.",
        tools=[_lookup_weather_tool()],
    )

    calls = _function_calls(response)
    call_names = [call["name"] for call in calls]
    assert "lookup_weather" in call_names, response

    weather_call = next(call for call in calls if call["name"] == "lookup_weather")
    assert json.loads(weather_call["arguments"]) == {"city": "Boston"}

    output_items = _function_call_outputs(response)
    assert not any(
        item.get("call_id") == weather_call["call_id"] for item in output_items
    ), response

    chat_requests = await _mock_chat_requests(mock_llm_server)
    assert len(chat_requests) >= 1
    initial_tools = _request_tool_names(chat_requests[0])
    assert "builtin__echo" in initial_tools
    assert "lookup_weather" in initial_tools

    forwarded_messages = json.dumps(
        [request.get("messages", []) for request in chat_requests],
        sort_keys=True,
    )
    assert "Run reborn mixed internal external tools for Boston." in forwarded_messages
