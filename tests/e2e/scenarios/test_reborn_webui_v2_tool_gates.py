"""Served Reborn WebUI v2 smoke coverage for tools and run gates.

The canonical smoke module proves text-only turns. These tests exercise the
remaining live ``ironclaw serve`` path through the deterministic mock model:
tool dispatch, cancellation, approval resolution, and manual-token auth
resolution. No route or SSE frame is stubbed.

Tracks nearai/ironclaw#4633.
"""

import asyncio
import json
import re
import uuid
from urllib.parse import quote

import httpx
import pytest
from playwright.async_api import expect

from helpers import REBORN_V2_AUTH_TOKEN, SEL_V2, sse_stream, wait_for_sse_line
from reborn_webui_harness import (
    DEFAULT_PROFILE,
    YOLO_PROFILE,
    client_action_id,
    close_reborn_server,
    create_thread,
    enable_reborn_global_auto_approve,
    fetch_timeline,
    reborn_v2_browser,
    reborn_bearer_headers,
    send_message,
    start_reborn_webui_v2_server,
    wait_for_assistant_message,
)


async def _wait_for_sse_event(
    response,
    *event_types: str,
    timeout: float = 60.0,
    match=None,
) -> dict:
    """Return the first matching WebChat JSON payload from the canonical stream."""
    matched_payload = None

    def matches(line: str) -> bool:
        nonlocal matched_payload
        if not line.startswith("data:"):
            return False
        try:
            payload = json.loads(line.removeprefix("data:").strip())
        except json.JSONDecodeError:
            return False
        event_type = payload.get("type", "")
        if event_type not in event_types or (
            match is not None and not match(event_type, payload)
        ):
            return False
        matched_payload = payload
        return True

    await wait_for_sse_line(
        response,
        predicate=matches,
        timeout=timeout,
    )
    assert matched_payload is not None
    return matched_payload


def _assert_text_redacted(secret: str, value: str, *, source: str) -> None:
    if secret in value:
        raise AssertionError(f"{source} exposed the raw credential")


async def _assert_sse_redacted_until(
    response,
    secret: str,
    outcome_reached: asyncio.Event,
) -> None:
    """Inspect every frame until the run is terminal and the stream goes quiet."""
    while True:
        try:
            raw = await asyncio.wait_for(response.content.readline(), timeout=0.25)
        except asyncio.TimeoutError:
            if outcome_reached.is_set():
                return
            continue
        if not raw:
            if outcome_reached.is_set():
                return
            raise AssertionError("SSE stream closed before the run reached a terminal outcome")
        line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
        _assert_text_redacted(secret, line, source="post-submit SSE frame")


class _ArtifactNotReady(Exception):
    """Transient 404 from the run artifact endpoint.

    After a gate resolution resumes a run, the terminal turn record and its
    projection can lag the artifact read by a short window. The polling
    helper `_wait_for_run_artifact_status` treats this 404 as retryable
    rather than fatal so a briefly-missing history projection is not
    mistaken for a permanent authorization or routing failure.
    """


async def _try_fetch_run_artifact(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    run_id: str,
) -> dict:
    response = await client.get(
        f"{base_url}/api/webchat/v2/threads/{thread_id}/runs/{run_id}/artifact",
        timeout=15,
    )
    if response.status_code == 404:
        raise _ArtifactNotReady(response.text)
    assert response.status_code == 200, response.text
    return response.json()


async def _wait_for_run_artifact_status(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    run_id: str,
    expected_status: str,
    *,
    timeout: float = 60.0,
) -> dict:
    deadline = asyncio.get_running_loop().time() + timeout
    last_artifact = None
    last_not_ready: str | None = None
    while asyncio.get_running_loop().time() < deadline:
        try:
            last_artifact = await _try_fetch_run_artifact(
                client,
                base_url,
                thread_id,
                run_id,
            )
        except _ArtifactNotReady as error:
            # The projection may briefly miss the resumed run's terminal
            # records; keep polling until they land or the deadline elapses.
            # Retain the latest transient 404 detail so a later non-terminal
            # 200 followed by a timeout still surfaces the earlier miss in
            # the failure message instead of reporting transient_404=None.
            last_not_ready = str(error)
            await asyncio.sleep(0.25)
            continue
        if last_artifact.get("run", {}).get("status") == expected_status:
            return last_artifact
        await asyncio.sleep(0.25)
    raise AssertionError(
        f"Run artifact did not reach {expected_status}; "
        f"last={last_artifact}; "
        f"transient_404={last_not_ready}"
    )


async def _set_tool_permission(
    client: httpx.AsyncClient,
    base_url: str,
    capability_id: str,
    state: str,
) -> None:
    response = await client.post(
        f"{base_url}/api/webchat/v2/settings/tools/{capability_id}",
        json={"state": state},
        timeout=15,
    )
    assert response.status_code == 200, response.text


async def _read_tool_permission(
    client: httpx.AsyncClient,
    base_url: str,
    capability_id: str,
) -> tuple[str, str]:
    response = await client.get(
        f"{base_url}/api/webchat/v2/settings/tools",
        timeout=15,
    )
    assert response.status_code == 200, response.text
    key = f"tool.{capability_id}"
    entry = next(
        (item for item in response.json().get("entries", []) if item.get("key") == key),
        None,
    )
    assert entry is not None, response.text
    assert entry.get("mutable") is True, entry
    value = entry.get("value") or {}
    state = value.get("state")
    assert state in {"always_allow", "ask_each_time", "disabled"}, entry
    effective_source = value.get("effective_source")
    assert effective_source in {"default", "global", "override"}, entry
    return state, effective_source


@pytest.fixture
async def reborn_v2_echo_approval_server(reborn_v2_server):
    """Pin echo to ask-each-time and restore the shared server state afterward."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        prior_state, prior_source = await _read_tool_permission(
            client,
            reborn_v2_server,
            "builtin.echo",
        )
        restore_state = (
            "default" if prior_source in {"default", "global"} else prior_state
        )
        await _set_tool_permission(
            client,
            reborn_v2_server,
            "builtin.echo",
            "ask_each_time",
        )
        try:
            yield reborn_v2_server
        finally:
            await _set_tool_permission(
                client,
                reborn_v2_server,
                "builtin.echo",
                restore_state,
            )
            restored_state, restored_source = await _read_tool_permission(
                client,
                reborn_v2_server,
                "builtin.echo",
            )
            assert (restored_state, restored_source) == (prior_state, prior_source)


async def _set_llm_delay(mock_llm_server: str, marker: str) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_faults",
            json={
                "faults": [
                    {
                        "match": marker,
                        "actions": [{"type": "delay", "seconds": 10.0}],
                    }
                ]
            },
            timeout=10,
        )
        response.raise_for_status()


async def _wait_for_mock_request(
    mock_llm_server: str,
    marker: str,
    *,
    timeout: float = 20.0,
) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    async with httpx.AsyncClient() as client:
        while asyncio.get_running_loop().time() < deadline:
            response = await client.get(
                f"{mock_llm_server}/__mock/chat_requests",
                timeout=10,
            )
            response.raise_for_status()
            for request in response.json().get("requests", []):
                if marker in json.dumps(request):
                    return
            await asyncio.sleep(0.25)
    raise AssertionError(f"Mock LLM never received request marker {marker!r}")


def _tool_result_references(timeline: dict) -> list[dict]:
    return [
        message
        for message in timeline.get("messages", [])
        if message.get("kind") == "tool_result_reference"
    ]


@pytest.fixture(scope="module")
async def reborn_v2_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Start the default profile with QA-only run artifacts enabled."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-tool-gates-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=DEFAULT_PROFILE,
        log_prefix="reborn-v2-tool-gates",
        extra_env={"IRONCLAW_REBORN_REGRESSION_ARTIFACT_EXPORT": "true"},
    )
    try:
        yield base_url
    finally:
        await close_reborn_server(proc)


@pytest.fixture(scope="module")
async def reborn_v2_yolo_server(
    ironclaw_reborn_binary, mock_llm_server, tmp_path_factory
):
    """Start the yolo profile with QA-only run artifacts enabled."""
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-v2-tool-gates-yolo-home")
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-v2-tool-gates-yolo",
        extra_env={"IRONCLAW_REBORN_REGRESSION_ARTIFACT_EXPORT": "true"},
    )
    try:
        await enable_reborn_global_auto_approve(base_url)
        yield base_url
    finally:
        await close_reborn_server(proc)


async def test_reborn_v2_tool_turn_records_result_and_final_reply(
    reborn_v2_yolo_server,
    reborn_v2_browser,
):
    marker = f"tool-turn-{uuid.uuid4().hex[:8]}"
    oversized_output = f"{marker}-" + ("x" * (50 * 1024 + 512))
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        submitted = await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            f"reborn builtin echo {oversized_output}",
        )
        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    run_id = assistant.get("turn_run_id") or submitted.get("run_id")
    assert run_id, (assistant, submitted)

    references = _tool_result_references(timeline)
    assert references, timeline
    assert any(reference.get("tool_result_ref") for reference in references), references
    assert assistant.get("status") == "finalized", assistant
    assistant_content = assistant.get("content")
    assert isinstance(assistant_content, str), assistant
    assert assistant_content.strip(), assistant

    context = await reborn_v2_browser.new_context(viewport={"width": 1440, "height": 900})
    page = await context.new_page()
    try:
        await page.goto(
            f"{reborn_v2_yolo_server}/chat/{thread_id}"
            f"?debug=true&token={REBORN_V2_AUTH_TOKEN}"
        )
        await page.locator(SEL_V2["inspector_tab_activity"]).click()
        tool_entry = page.locator("[data-activity-kind='tool_completed']").first
        await expect(tool_entry).to_be_visible(timeout=30000)
        await tool_entry.get_by_role("button", name="Show details").click()
        detail = tool_entry.locator("[data-testid^='inspector-tool-detail-']")
        await expect(detail).to_be_visible(timeout=15000)
        await expect(detail).to_contain_text("builtin.echo")
        # The finite status set is localized, not the raw wire value.
        await expect(detail).to_contain_text("Succeeded")
        arguments = detail.get_by_text("Arguments", exact=True).locator("..").locator("pre")
        await expect(arguments).to_contain_text(marker)
        await expect(detail.get_by_text("Duration:")).to_have_count(1)
        await expect(detail.get_by_text("Output size:")).to_have_count(1)
        await expect(
            detail.get_by_text(
                re.compile(r"Output · truncated from 5[1-9],[0-9]{3} bytes")
            )
        ).to_be_visible()
        output = detail.locator("pre").nth(1)
        retained_bytes = await output.evaluate(
            "element => new TextEncoder().encode(element.textContent || '').length"
        )
        assert retained_bytes <= 50 * 1024
    finally:
        await context.close()


async def test_reborn_v2_cancel_in_flight_turn_ends_cancelled(
    reborn_v2_server,
    mock_llm_server,
):
    marker = f"cancel-in-flight-{uuid.uuid4().hex[:8]}"
    await _set_llm_delay(mock_llm_server, marker)

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_server)

        async with sse_stream(
            reborn_v2_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=100,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": f"{marker}: hold this response",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text
            run_id = submitted.json()["run_id"]
            await _wait_for_mock_request(mock_llm_server, marker)

            cancelled = await client.post(
                f"{reborn_v2_server}/api/webchat/v2/threads/{thread_id}"
                f"/runs/{run_id}/cancel",
                json={
                    "client_action_id": client_action_id(),
                    "reason": "user_requested",
                },
                timeout=15,
            )
            assert cancelled.status_code == 200, cancelled.text
            assert cancelled.json()["run_id"] == run_id

            def is_cancelled(event_type: str, payload: dict) -> bool:
                if event_type == "cancelled":
                    response = payload.get("response") or {}
                    # Cancel responses serialize the TurnStatus enum variant,
                    # while projection run statuses use lowercase wire values.
                    return (
                        response.get("run_id") == run_id
                        and response.get("status") == "Cancelled"
                    )
                # RunStatus is externally tagged, so its run_id is nested here.
                for item in payload.get("state", {}).get("items", []):
                    status = item.get("run_status") or {}
                    if (
                        status.get("run_id") == run_id
                        and status.get("status") == "cancelled"
                    ):
                        return True
                return False

            await _wait_for_sse_event(
                stream,
                "cancelled",
                "projection_snapshot",
                "projection_update",
                timeout=45,
                match=is_cancelled,
            )


async def test_reborn_v2_approval_gate_resolves_and_resumes(
    reborn_v2_echo_approval_server,
):
    marker = f"approval-{uuid.uuid4().hex[:8]}"
    base_url = reborn_v2_echo_approval_server
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, base_url)

        async with sse_stream(
            base_url,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=90,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": f"reborn builtin echo {marker}",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text

            event = await _wait_for_sse_event(stream, "gate", timeout=60)
            prompt = event["prompt"]
            assert prompt["approval_context"]["tool_name"] == "builtin.echo"

            resolved = await client.post(
                f"{base_url}/api/webchat/v2/threads/{thread_id}"
                f"/runs/{prompt['turn_run_id']}"
                f"/gates/{quote(prompt['gate_ref'], safe='')}/resolve",
                json={
                    "client_action_id": client_action_id(),
                    "resolution": "approved",
                    "always": False,
                },
                timeout=15,
            )
            assert resolved.status_code == 200, resolved.text
            assert resolved.json()["outcome"] == "resumed", resolved.text

        assistant = await wait_for_assistant_message(
            client,
            base_url,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, base_url, thread_id)

    assert assistant.get("status") == "finalized", assistant
    assert _tool_result_references(timeline), timeline


async def test_reborn_v2_approval_gate_decline_has_no_successful_tool_result(
    reborn_v2_echo_approval_server,
):
    marker = f"approval-decline-{uuid.uuid4().hex[:8]}"
    base_url = reborn_v2_echo_approval_server
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, base_url)

        async with sse_stream(
            base_url,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=90,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{base_url}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": f"reborn builtin echo {marker}",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text
            run_id = submitted.json()["run_id"]

            gate_event = await _wait_for_sse_event(
                stream,
                "gate",
                timeout=60,
                match=lambda _event_type, payload: (
                    payload.get("prompt", {}).get("turn_run_id") == run_id
                ),
            )
            prompt = gate_event["prompt"]
            assert prompt["approval_context"]["tool_name"] == "builtin.echo"

            resolved = await client.post(
                f"{base_url}/api/webchat/v2/threads/{thread_id}"
                f"/runs/{run_id}/gates/{quote(prompt['gate_ref'], safe='')}/resolve",
                json={
                    "client_action_id": client_action_id(),
                    "resolution": "declined",
                },
                timeout=15,
            )
            assert resolved.status_code == 200, resolved.text
            assert resolved.json()["outcome"] == "resumed", resolved.text

        artifact = await _wait_for_run_artifact_status(
            client,
            base_url,
            thread_id,
            run_id,
            "Completed",
        )
        assistant = await wait_for_assistant_message(
            client,
            base_url,
            thread_id,
            timeout=60,
        )
        timeline = await fetch_timeline(client, base_url, thread_id)

    assistant_content = assistant.get("content")
    assert isinstance(assistant_content, str), assistant
    assert "declined by user" in assistant_content.lower(), assistant
    assert artifact["run"]["status"] == "Completed", artifact

    references = _tool_result_references(timeline)
    assert references, timeline
    for reference in references:
        envelope = json.loads(reference["content"])
        observation = envelope["model_observation"]
        assert observation["status"] == "error", envelope
        assert observation["detail"]["failure_kind"] == "gate_declined", envelope
        assert envelope["result_ref"].startswith("result:provider-error-"), envelope


async def test_reborn_v2_manual_token_auth_gate_resolves_and_resumes(
    reborn_v2_yolo_server,
):
    raw_token = f"ghp_e2e_{uuid.uuid4().hex}"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)

        async with sse_stream(
            reborn_v2_yolo_server,
            path=f"/api/webchat/v2/threads/{thread_id}/events",
            token=REBORN_V2_AUTH_TOKEN,
            timeout=120,
        ) as stream:
            assert stream.status == 200
            submitted = await client.post(
                f"{reborn_v2_yolo_server}/api/webchat/v2/threads/{thread_id}/messages",
                json={
                    "client_action_id": client_action_id(),
                    "content": "reborn install github for auth gate",
                },
                timeout=30,
            )
            assert submitted.status_code in (200, 202), submitted.text

            event = await _wait_for_sse_event(stream, "auth_required", timeout=75)
            prompt = event["prompt"]
            assert prompt["provider"] == "github", prompt
            assert prompt["challenge_kind"] == "manual_token", prompt

            run_id = prompt["turn_run_id"]
            outcome_reached = asyncio.Event()
            sse_redaction = asyncio.create_task(
                _assert_sse_redacted_until(stream, raw_token, outcome_reached)
            )
            try:
                token_submit = await client.post(
                    f"{reborn_v2_yolo_server}"
                    "/api/reborn/product-auth/manual-token/submit",
                    json={
                        "provider": "github",
                        "account_label": "Reborn E2E GitHub",
                        "token": raw_token,
                        "thread_id": thread_id,
                        "run_id": run_id,
                        "gate_ref": prompt["auth_request_ref"],
                    },
                    timeout=15,
                )
                assert token_submit.status_code == 200, token_submit.text
                token_body = token_submit.json()
                credential_ref = token_body.get("credential_ref")
                assert isinstance(credential_ref, str), token_body
                assert credential_ref.strip(), token_body
                assert token_body["continuation"]["type"] == "turn_gate_resume"
                _assert_text_redacted(
                    raw_token,
                    token_submit.text,
                    source="manual-token response",
                )

                artifact = await _wait_for_run_artifact_status(
                    client,
                    reborn_v2_yolo_server,
                    thread_id,
                    run_id,
                    "Completed",
                    timeout=75,
                )
                assistant = await wait_for_assistant_message(
                    client,
                    reborn_v2_yolo_server,
                    thread_id,
                    timeout=75,
                )
                timeline = await fetch_timeline(
                    client,
                    reborn_v2_yolo_server,
                    thread_id,
                )
            finally:
                outcome_reached.set()
                await sse_redaction

    assert assistant.get("status") == "finalized", assistant
    assert _tool_result_references(timeline), timeline
    assert isinstance(artifact.get("logs", {}).get("entries"), list), artifact
    _assert_text_redacted(raw_token, json.dumps(timeline), source="timeline")
    _assert_text_redacted(raw_token, json.dumps(artifact), source="run artifact")


class _MockArtifactResponse:
    """Minimal stand-in for `httpx.Response` used by `_try_fetch_run_artifact`."""

    def __init__(self, status_code: int, json_payload: dict | None = None, text: str = ""):
        self.status_code = status_code
        self._json = json_payload
        self.text = text

    def json(self) -> dict:
        if self._json is None:
            raise ValueError("no json payload")
        return self._json


class _ScriptedArtifactClient:
    """Async client that replays a scripted sequence of artifact responses.

    Each entry is either an `int` status code (with empty text) or a
    `(status_code, json_payload_or_none, text)` tuple. The client records
    every call so the regression test can assert polling behavior.
    """

    def __init__(self, script: list):
        self._script = list(script)
        self.calls = 0

    async def get(self, _url: str, timeout: float = 15) -> _MockArtifactResponse:
        if not self._script:
            raise AssertionError("scripted client exhausted")
        entry = self._script.pop(0)
        self.calls += 1
        if isinstance(entry, int):
            return _MockArtifactResponse(entry, text=f"status {entry}")
        status_code, payload, text = entry
        return _MockArtifactResponse(status_code, payload, text)


async def test_wait_for_run_artifact_status_preserves_transient_404_through_timeout() -> None:
    """A 404 followed by a non-terminal 200 must still surface the 404 on timeout.

    Regression for the polling helper: previously a non-terminal 200 cleared
    `last_not_ready`, so a later timeout reported `transient_404=None` and hid
    the earlier miss. The sequence here is 404 -> non-terminal 200 -> timeout,
    which must raise an `AssertionError` whose message still includes the 404
    detail.
    """
    script = [
        404,
        (200, {"run": {"status": "Running"}}, ""),
        (200, {"run": {"status": "Running"}}, ""),
    ]
    client = _ScriptedArtifactClient(script)
    with pytest.raises(AssertionError) as exc_info:
        await _wait_for_run_artifact_status(
            client,  # type: ignore[arg-type]
            "http://server",
            "thread-1",
            "run-1",
            "Completed",
            timeout=0.6,
        )
    message = str(exc_info.value)
    assert "transient_404=status 404" in message, message
    assert "Run artifact did not reach Completed" in message, message
    # The non-terminal 200 response must be reflected as the last artifact,
    # proving the helper continued polling past the 404 instead of exiting
    # on the first transient miss.
    assert "'status': 'Running'" in message, message
