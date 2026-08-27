"""Run harvested Reborn QA traces through the full Emulate-backed path.

The trace controls model decisions only. Capability execution still crosses the
served Reborn runtime, first-party extension, credential boundary, HTTP rewrite,
and Emulate.dev provider. Assertions intentionally target those boundaries, not
the recorded model's final wording.
"""

import asyncio
import hashlib
import json
import os
import sys
from collections import Counter
from pathlib import Path
from urllib.parse import urlparse

import httpx
import pytest
from emulate_provider import (
    github_json,
    google_headers,
    slack_headers,
    slack_post,
)
from helpers import EMULATE_GITHUB_BEARER
from journey_cases import (
    PROVIDER_JOURNEY_RUN_IDS,
    PROVIDER_JOURNEY_RUNS,
    journey_order_is_reversed,
    shared_world_provider_journey_runs,
)
from provider_capability_inventory import (
    EMULATE_SUPPORTED_TOOLS,
    capability_id_to_wire_name,
)
from provider_fault_cases import PROVIDER_FAULT_CASES
from provider_fault_proxy import PROVIDER_FAULT_PROFILES
from provider_journey_github import emulate_github_supports_release_writes
from provider_journey_google import (
    assert_google_provider_baseline,
    assert_google_provider_outcome,
)
from provider_journey_slack import (
    assert_slack_provider_baseline,
    assert_slack_provider_outcome,
    cleanup_slack_provider_mutations,
    emulate_slack_channel_bearer,
)
from provider_journey_trace import (
    compile_provider_journey_trace,
    load_recorded_trace,
    recorded_provider_calls,
    recorded_trace_uses_tool_prefix,
)
from provider_journey_world import build_provider_journey_world
from provider_operation_cases import PROVIDER_OPERATION_CASES
from provider_operation_types import (
    ProviderOperationCase,
    assert_provider_request_evidence,
)
from reborn_webui_harness import (
    YOLO_PROFILE,
    capability_preview_payload,
    client_action_id,
    close_reborn_server,
    create_thread,
    enable_reborn_global_auto_approve,
    reborn_bearer_headers,
    send_message,
    start_reborn_webui_v2_server,
    wait_for_assistant_message,
)

pytest_plugins = ["reborn_webui_harness"]

ROOT = Path(__file__).resolve().parents[3]

PROVIDER_TOOL_NAMES = EMULATE_SUPPORTED_TOOLS
GOOGLE_PROVIDER_OPERATION_BEARER = "mock-token-mock_auth_code"


@pytest.fixture(scope="module")
async def reborn_qa_emulate_runtime(
    ironclaw_reborn_binary,
    mock_llm_server,
    resettable_emulate_provider_world,
    provider_fault_proxy_world,
    tmp_path_factory,
):
    """Start one Reborn process against resettable provider URLs."""
    provider_servers = resettable_emulate_provider_world.servers
    provider_proxy_servers = provider_fault_proxy_world.servers
    emulate_google_server = provider_servers["google"]
    emulate_github_server = provider_servers["github"]
    emulate_slack_server = provider_servers["slack"]
    google_proxy_server = provider_proxy_servers["google"]
    github_proxy_server = provider_proxy_servers["github"]
    slack_proxy_server = provider_proxy_servers["slack"]
    home_dir = tmp_path_factory.mktemp("reborn-qa-emulate-provider-home")
    mock_llm_address = urlparse(mock_llm_server)
    emulate_google_address = urlparse(google_proxy_server["url"])
    emulate_github_address = urlparse(github_proxy_server["url"])
    emulate_slack_address = urlparse(slack_proxy_server["url"])
    rewrite_map = ",".join(
        (
            f"oauth2.googleapis.com={mock_llm_address.hostname}:{mock_llm_address.port}",
            (
                f"www.googleapis.com={emulate_google_address.hostname}:"
                f"{emulate_google_address.port}"
            ),
            (
                f"gmail.googleapis.com={emulate_google_address.hostname}:"
                f"{emulate_google_address.port}"
            ),
            (
                f"docs.googleapis.com={emulate_google_address.hostname}:"
                f"{emulate_google_address.port}"
            ),
            (
                f"sheets.googleapis.com={emulate_google_address.hostname}:"
                f"{emulate_google_address.port}"
            ),
            (
                f"slides.googleapis.com={emulate_google_address.hostname}:"
                f"{emulate_google_address.port}"
            ),
            (
                f"api.github.com={emulate_github_address.hostname}:"
                f"{emulate_github_address.port}"
            ),
            (
                f"slack.com={emulate_slack_address.hostname}:"
                f"{emulate_slack_address.port}"
            ),
        )
    )
    proc, base_url = await start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=YOLO_PROFILE,
        log_prefix="reborn-qa-emulate-provider",
        extra_env={
            "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP": rewrite_map,
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "reborn-qa-emulate-client",
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI": (
                "http://127.0.0.1/api/reborn/product-auth/oauth/google/callback"
            ),
        },
    )
    await enable_reborn_global_auto_approve(base_url)
    slack_state = await build_provider_journey_world(
        base_url, emulate_slack_server["url"]
    )
    slack_account_fingerprints = {
        request["issued_credential_fingerprint"]
        for request in provider_fault_proxy_world.proxies["slack"].state[
            "requests"
        ]
        if request["path"] == "/api/oauth.v2.access"
        and request["issued_credential_fingerprint"] is not None
    }
    assert len(slack_account_fingerprints) == 1, (
        "Slack OAuth binding must establish exactly one provider account",
        provider_fault_proxy_world.proxies["slack"].state["requests"],
    )
    slack_account_fingerprint = next(iter(slack_account_fingerprints))
    try:
        yield {
            "base_url": base_url,
            "emulate_google_url": emulate_google_server["url"],
            "emulate_github_url": emulate_github_server["url"],
            "emulate_slack_url": emulate_slack_server["url"],
            "provider_fault_proxies": provider_fault_proxy_world.proxies,
            "slack_state": slack_state,
            "slack_account_fingerprint": slack_account_fingerprint,
        }
    finally:
        await close_reborn_server(proc)


@pytest.fixture
async def reborn_qa_emulate_provider_server(
    reborn_qa_emulate_runtime,
    resettable_emulate_provider_world,
    journey_case,
):
    """Reset mutated providers while reusing the built binary and Reborn."""
    services = {str(world) for world in journey_case.mutable_provider_worlds}
    try:
        yield reborn_qa_emulate_runtime
    finally:
        await _cleanup_provider_journey_world(
            reborn_qa_emulate_runtime,
            resettable_emulate_provider_world,
            journey_case,
            services,
        )


async def _cleanup_provider_journey_world(
    runtime,
    resettable_provider_world,
    journey_case,
    services: set[str],
) -> None:
    reset_services = services - {"slack"}
    try:
        if "slack" in services:
            compiled = _compile_journey_case(
                journey_case,
                runtime["slack_state"],
            )
            await cleanup_slack_provider_mutations(
                runtime["emulate_slack_url"],
                runtime["slack_state"],
                recorded_provider_calls(compiled.trace, PROVIDER_TOOL_NAMES),
            )
    finally:
        if reset_services:
            await resettable_provider_world.reset(reset_services)


async def test_provider_journey_cleanup_resets_world_when_trace_compile_fails(
    monkeypatch,
):
    class RecordingResettableWorld:
        def __init__(self):
            self.calls = []

        async def reset(self, services):
            self.calls.append(services)

    resettable_world = RecordingResettableWorld()

    def fail_compile(*_args, **_kwargs):
        raise AssertionError("synthetic compile failure")

    monkeypatch.setattr(
        sys.modules[__name__],
        "_compile_journey_case",
        fail_compile,
    )

    with pytest.raises(AssertionError, match="synthetic compile failure"):
        await _cleanup_provider_journey_world(
            {
                "emulate_slack_url": "http://slack.invalid",
                "slack_state": {},
            },
            resettable_world,
            object(),
            {"google", "slack"},
        )

    assert resettable_world.calls == [{"google"}]


@pytest.fixture
async def reborn_provider_operation_server(
    reborn_qa_emulate_runtime,
    resettable_emulate_provider_world,
    provider_fault_proxy_world,
    operation_case,
):
    """Reuse Reborn while isolating provider state and request evidence."""
    provider_fault_proxy_world.reset()
    try:
        yield reborn_qa_emulate_runtime
    finally:
        if operation_case.cleanup_provider is not None:
            emulate_url = reborn_qa_emulate_runtime[
                f"emulate_{operation_case.provider_service}_url"
            ]
            await operation_case.cleanup_provider(emulate_url)
        provider_fault_proxy_world.reset()
        # Slack's baseline workspace is seeded once after process startup, so
        # restarting it here would erase the records later cases assert. Its
        # only mutating operation owns an explicit provider cleanup above.
        if operation_case.provider_service != "slack":
            await resettable_emulate_provider_world.reset(
                {operation_case.provider_service}
            )


@pytest.fixture
async def reborn_provider_fault_server(
    reborn_qa_emulate_runtime,
    resettable_emulate_provider_world,
    provider_fault_proxy_world,
    fault_case,
):
    """Reset fault and provider state around one representative failure."""
    provider_fault_proxy_world.reset()
    try:
        yield reborn_qa_emulate_runtime
    finally:
        provider_fault_proxy_world.reset()
        await resettable_emulate_provider_world.reset(
            {fault_case.operation.provider_service}
        )


def _compile_journey_case(journey_case, slack_state: dict[str, str]):
    trace_path = ROOT / journey_case.trace
    return compile_provider_journey_trace(
        load_recorded_trace(trace_path),
        source=trace_path.name,
        facts=journey_case.replay,
        provider_tools=PROVIDER_TOOL_NAMES,
        slack_state=slack_state,
    )


async def _install_inline_trace(
    mock_llm_server: str,
    source: str,
    trace: dict,
) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": source, "trace": trace},
            timeout=15,
        )
    response.raise_for_status()


def _provider_operation_trace(case: ProviderOperationCase, arguments: dict) -> dict:
    wire_name = capability_id_to_wire_name(case.capability_id)
    return {
        "steps": [
            {
                "response": {
                    "type": "user_input",
                    "content": f"Execute provider contract {case.case_id}",
                }
            },
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": f"disclose_{case.case_id}",
                            "name": "capability_info",
                            "arguments": {"name": case.capability_id},
                        }
                    ],
                }
            },
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": f"execute_{case.case_id}",
                            "name": wire_name,
                            "arguments": arguments,
                        }
                    ],
                }
            },
            {
                "response": {
                    "type": "text",
                    "content": "Provider operation completed.",
                }
            },
        ]
    }


async def _wait_for_trace_replay(mock_llm_server: str, timeout: float = 30) -> dict:
    state = {}
    async with httpx.AsyncClient() as client:
        for _ in range(int(timeout * 2)):
            response = await client.get(
                f"{mock_llm_server}/__mock/llm_trace",
                timeout=15,
            )
            response.raise_for_status()
            state = response.json()
            assert state["error"] is None, state["error"]
            if state["complete"]:
                return state
            await asyncio.sleep(0.5)
    raise AssertionError(
        f"recorded trace did not complete within {timeout} seconds: {state}"
    )


async def _submit_and_wait_for_trace_replay(
    client,
    server: str,
    thread_id: str,
    user_input: str,
    mock_llm_server: str,
    *,
    timeout: float,
) -> dict:
    """Cancel an unfinished run so it cannot consume the next test's trace."""
    submitted = await send_message(client, server, thread_id, user_input)
    try:
        return await _wait_for_trace_replay(mock_llm_server, timeout=timeout)
    except BaseException as replay_error:
        try:
            response = await asyncio.shield(
                client.post(
                    f"{server}/api/webchat/v2/threads/{thread_id}"
                    f"/runs/{submitted['run_id']}/cancel",
                    json={
                        "client_action_id": client_action_id(),
                        "reason": "qa_trace_replay_failed",
                    },
                    timeout=15,
                )
            )
            response.raise_for_status()
        except Exception as cleanup_error:
            replay_error.add_note(f"run cancellation also failed: {cleanup_error}")
        raise


async def test_trace_replay_failure_cancels_the_submitted_run(monkeypatch):
    requests = []

    class RecordingResponse:
        def raise_for_status(self):
            return None

    class RecordingClient:
        async def post(self, url, *, json, timeout):
            requests.append((url, json, timeout))
            return RecordingResponse()

    async def fail_replay(_mock_llm_server, timeout):
        raise AssertionError(f"synthetic replay timeout after {timeout}s")

    async def accept_message(_client, _server, _thread_id, _user_input):
        return {"run_id": "run-1"}

    monkeypatch.setattr(sys.modules[__name__], "send_message", accept_message)
    monkeypatch.setattr(
        sys.modules[__name__],
        "_wait_for_trace_replay",
        fail_replay,
    )

    with pytest.raises(AssertionError, match="synthetic replay timeout"):
        await _submit_and_wait_for_trace_replay(
            RecordingClient(),
            "http://reborn.invalid",
            "thread-1",
            "user input",
            "http://mock-llm.invalid",
            timeout=120,
        )

    assert len(requests) == 1
    url, body, request_timeout = requests[0]
    assert url.endswith("/threads/thread-1/runs/run-1/cancel")
    assert body["reason"] == "qa_trace_replay_failed"
    assert request_timeout == 15


async def _fetch_all_timeline_pages_with_retry(
    client: httpx.AsyncClient, server: str, thread_id: str
) -> dict:
    timeline = None
    messages = []
    cursor = None
    seen_cursors = set()

    while True:
        params = {"limit": 200}
        if cursor is not None:
            params["cursor"] = cursor

        for _ in range(20):
            response = await client.get(
                f"{server}/api/webchat/v2/threads/{thread_id}/timeline",
                params=params,
                timeout=15,
            )
            if response.status_code != 429:
                response.raise_for_status()
                page = response.json()
                break
            await asyncio.sleep(0.5)
        else:
            raise AssertionError(
                "timeline remained rate-limited after replay completed"
            )

        if timeline is None:
            timeline = page
        messages = [*page.get("messages", []), *messages]
        cursor = page.get("next_cursor")
        if cursor is None:
            timeline["messages"] = messages
            timeline["next_cursor"] = None
            return timeline
        assert isinstance(cursor, str) and cursor, page
        assert cursor not in seen_cursors, f"timeline cursor repeated: {cursor}"
        seen_cursors.add(cursor)


async def test_slack_mutation_cleanup_covers_thread_replies(
    resettable_emulate_provider_world,
) -> None:
    """Threaded sends must be visible to baseline checks and cleanup."""
    emulate_url = resettable_emulate_provider_world.servers["slack"]["url"]
    async with httpx.AsyncClient(timeout=15) as client:
        channels = await slack_post(
            client,
            emulate_url,
            "conversations.list",
            {"types": "public_channel", "exclude_archived": True},
        )
        channel = next(
            item for item in channels["channels"] if item["name"] == "reborn-alerts"
        )
        root = await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {"channel": channel["id"], "text": "thread cleanup contract root"},
        )
        reply = await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {
                "channel": channel["id"],
                "thread_ts": root["ts"],
                "text": "thread cleanup contract reply",
            },
        )

    trace = {
        "steps": [
            {
                "response": {
                    "tool_calls": [
                        {
                            "name": "slack__send_message",
                            "arguments": {
                                "channel": channel["id"],
                                "thread_ts": root["ts"],
                                "text": reply["message"]["text"],
                            },
                        }
                    ]
                }
            }
        ]
    }
    slack_state = {"channel_id": channel["id"]}
    try:
        calls = recorded_provider_calls(trace, PROVIDER_TOOL_NAMES)
        await assert_slack_provider_outcome(emulate_url, slack_state, calls)
        with pytest.raises(AssertionError, match="already contains"):
            await assert_slack_provider_baseline(emulate_url, slack_state, calls)

        await cleanup_slack_provider_mutations(emulate_url, slack_state, calls)
        await assert_slack_provider_baseline(emulate_url, slack_state, calls)
    finally:
        async with httpx.AsyncClient(timeout=15) as client:
            for message in (reply, root):
                await slack_post(
                    client,
                    emulate_url,
                    "chat.delete",
                    {"channel": channel["id"], "ts": message["ts"]},
                    expect_ok=False,
                )


@pytest.mark.timeout(150)
@pytest.mark.parametrize(
    "journey_case", PROVIDER_JOURNEY_RUNS, ids=PROVIDER_JOURNEY_RUN_IDS
)
async def test_qa_journey_provider_leg_replays_through_emulate(
    reborn_qa_emulate_provider_server,
    mock_llm_server,
    journey_case,
):
    """Every harvested provider journey executes through standalone Reborn."""
    await _replay_qa_journey_provider_leg(
        reborn_qa_emulate_provider_server,
        mock_llm_server,
        journey_case,
    )


async def _replay_qa_journey_provider_leg(
    provider_server,
    mock_llm_server,
    journey_case,
) -> None:
    """Replay one provider leg against the caller-selected provider lifecycle."""
    server = provider_server["base_url"]
    trace_path = ROOT / journey_case.trace
    recorded_trace = load_recorded_trace(trace_path)
    if recorded_trace_uses_tool_prefix(recorded_trace, "google-sheets__"):
        async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
            emulate_google_url = provider_server["emulate_google_url"]
            response = await client.get(
                f"{emulate_google_url}/v4/spreadsheets/sheet_reborn_abc"
            )
        if response.status_code == 404:
            pytest.skip("Emulate 0.7.0 does not expose the Google Sheets API")
    if recorded_trace_uses_tool_prefix(recorded_trace, "slack__"):
        async with httpx.AsyncClient(headers=slack_headers(), timeout=15) as client:
            emulate_slack_url = provider_server["emulate_slack_url"]
            response = await client.get(f"{emulate_slack_url}/api/auth.test")
        if response.status_code == 404:
            pytest.skip("Emulate 0.7.0 does not expose Slack Web API GET routes")
    compiled = compile_provider_journey_trace(
        recorded_trace,
        source=trace_path.name,
        facts=journey_case.replay,
        provider_tools=PROVIDER_TOOL_NAMES,
        slack_state=provider_server["slack_state"],
    )
    trace = compiled.trace
    await _install_inline_trace(mock_llm_server, compiled.source, trace)
    user_input = trace["steps"][0]["response"]["content"]
    expected_calls = recorded_provider_calls(trace, PROVIDER_TOOL_NAMES)

    await assert_google_provider_baseline(
        provider_server["emulate_google_url"], expected_calls
    )
    await assert_slack_provider_baseline(
        provider_server["emulate_slack_url"],
        provider_server["slack_state"],
        expected_calls,
    )

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, server)

        replay_timeout = journey_case.replay.timeout_seconds
        replay = await _submit_and_wait_for_trace_replay(
            client,
            server,
            thread_id,
            user_input,
            mock_llm_server,
            timeout=replay_timeout,
        )
        assistant = await wait_for_assistant_message(
            client, server, thread_id, timeout=replay_timeout
        )
        timeline = await _fetch_all_timeline_pages_with_retry(client, server, thread_id)
        previews = [
            preview
            for message in timeline.get("messages", [])
            if (preview := capability_preview_payload(message)) is not None
        ]
        expected_counts = Counter(
            call["name"].replace("__", ".") for call in expected_calls
        )
        actual_counts = Counter(
            preview["capability_id"]
            for preview in previews
            if preview["capability_id"] in expected_counts
        )
        assert actual_counts == expected_counts, (actual_counts, expected_counts)
        expected_failure = journey_case.replay.expected_capability_failure
        for preview in previews:
            if preview["capability_id"] not in expected_counts:
                continue
            output = json.dumps(preview).lower()
            if expected_failure is not None:
                assert preview["status"] == "failed", json.dumps(preview)
                assert expected_failure in output, preview
                continue
            assert preview["status"] == "completed", json.dumps(preview)
            assert "auth_required" not in output, preview
            assert "not found" not in output, preview

        if expected_failure is not None:
            assert expected_failure in assistant["content"]

    await assert_google_provider_outcome(
        provider_server["emulate_google_url"], expected_calls
    )
    await assert_slack_provider_outcome(
        provider_server["emulate_slack_url"],
        provider_server["slack_state"],
        expected_calls,
    )

    assert replay == {
        "source": compiled.source,
        "next_response": len(trace["steps"]) - 1,
        "response_count": len(trace["steps"]) - 1,
        "complete": True,
        "error": None,
    }


@pytest.mark.shared_world
async def test_mutating_qa_journeys_replay_in_reverse_against_shared_provider_world(
    reborn_qa_emulate_runtime,
    resettable_emulate_provider_world,
    mock_llm_server,
):
    """Reverse mutating journeys while preserving every prior provider effect."""
    # CI sets the switch explicitly. Failing closed prevents a workflow edit
    # from silently turning this expensive proof into another forward replay.
    assert journey_order_is_reversed(), (
        "shared-world reverse replay requires IRONCLAW_JOURNEY_ORDER=reverse"
    )
    journeys, _ = shared_world_provider_journey_runs(reverse=True)
    mutable_services = {
        str(world) for case in journeys for world in case.mutable_provider_worlds
    }
    reset_services = mutable_services - {"slack"}
    try:
        for journey_case in journeys:
            await _replay_qa_journey_provider_leg(
                reborn_qa_emulate_runtime,
                mock_llm_server,
                journey_case,
            )
    finally:
        # Cleanup belongs after the sequence: doing any of it inside the loop
        # would restore isolation and make reversed order unable to expose
        # cross-journey state leakage.
        try:
            if "slack" in mutable_services:
                for journey_case in journeys:
                    if any(
                        str(world) == "slack"
                        for world in journey_case.mutable_provider_worlds
                    ):
                        compiled = _compile_journey_case(
                            journey_case,
                            reborn_qa_emulate_runtime["slack_state"],
                        )
                        await cleanup_slack_provider_mutations(
                            reborn_qa_emulate_runtime["emulate_slack_url"],
                            reborn_qa_emulate_runtime["slack_state"],
                            recorded_provider_calls(
                                compiled.trace, PROVIDER_TOOL_NAMES
                            ),
                        )
        finally:
            if reset_services:
                await resettable_emulate_provider_world.reset(reset_services)


def _provider_operation_cases_for_shard():
    shard = os.environ.get("IRONCLAW_PROVIDER_OPERATION_SHARD")
    if shard is None:
        return PROVIDER_OPERATION_CASES

    try:
        index_text, total_text = shard.split("/", 1)
        index, total = int(index_text), int(total_text)
    except ValueError as error:
        raise ValueError(
            "IRONCLAW_PROVIDER_OPERATION_SHARD must use INDEX/TOTAL"
        ) from error
    if total < 1 or index < 0 or index >= total:
        raise ValueError(
            "IRONCLAW_PROVIDER_OPERATION_SHARD requires 0 <= INDEX < TOTAL"
        )
    return [
        case
        for position, case in enumerate(PROVIDER_OPERATION_CASES)
        if position % total == index
    ]


@pytest.mark.timeout(150)
@pytest.mark.parametrize(
    "operation_case",
    _provider_operation_cases_for_shard(),
    ids=lambda case: case.case_id,
)
async def test_provider_operation_case_executes_with_provider_readback(
    reborn_provider_operation_server,
    mock_llm_server,
    operation_case,
):
    """Typed operation cases cross Reborn and prove provider-observable results."""
    server = reborn_provider_operation_server["base_url"]
    emulate_url = reborn_provider_operation_server[
        f"emulate_{operation_case.provider_service}_url"
    ]
    if (
        operation_case.provider_service == "github"
        and not await emulate_github_supports_release_writes(emulate_url)
    ):
        pytest.skip("Selected Emulate GitHub fixture does not expose repo write APIs")
    source = f"provider-operation-{operation_case.case_id}.json"
    await operation_case.assert_baseline(emulate_url)
    arguments = await operation_case.resolve_arguments(emulate_url)
    proxy = reborn_provider_operation_server["provider_fault_proxies"][
        operation_case.provider_service
    ]
    if operation_case.setup_provider_proxy is not None:
        operation_case.setup_provider_proxy(proxy)
    trace = _provider_operation_trace(operation_case, arguments)
    if operation_case.expected_failed_tool_result_contains is not None:
        # Same mechanism the fault cases below use: without the hint the
        # replayer treats any failed capability result as a replay error.
        trace["steps"][-1]["request_hint"] = {
            "expected_failed_tool_result_contains": (
                operation_case.expected_failed_tool_result_contains
            )
        }
    await _install_inline_trace(mock_llm_server, source, trace)

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, server)
        replay = await _submit_and_wait_for_trace_replay(
            client,
            server,
            thread_id,
            trace["steps"][0]["response"]["content"],
            mock_llm_server,
            timeout=120,
        )
        await wait_for_assistant_message(client, server, thread_id, timeout=120)
        timeline = await _fetch_all_timeline_pages_with_retry(client, server, thread_id)

    matches = [
        preview
        for message in timeline.get("messages", [])
        if (preview := capability_preview_payload(message)) is not None
        and preview["capability_id"] == operation_case.capability_id
    ]
    assert len(matches) == 1, matches
    # Almost every case completes; a single-item read's `empty` class is its
    # typed model-visible miss, declared via `expected_status="failed"` (see
    # `ExpectedCapabilityStatus` in provider_operation_types.py).
    assert matches[0]["status"] == operation_case.expected_status, matches[0]
    await operation_case.assert_outcome(emulate_url, matches[0])
    expected_bearer = {
        # The full-path OAuth exchange deliberately returns this account token;
        # EMULATE_GOOGLE_BEARER is used only by the test's provider readback.
        "google": GOOGLE_PROVIDER_OPERATION_BEARER,
    }.get(operation_case.provider_service)
    assert_provider_request_evidence(
        operation_case,
        proxy.state["requests"],
        expected_bearer=expected_bearer,
        expected_credential_fingerprint=(
            hashlib.sha256(f"token {EMULATE_GITHUB_BEARER}".encode()).hexdigest()[:12]
            if operation_case.provider_service == "github"
            else reborn_provider_operation_server["slack_account_fingerprint"]
            if operation_case.provider_service == "slack"
            else None
        ),
        # Product Slack delivery uses the separately configured channel token
        # while extension operations use the caller's OAuth account. Exclude
        # only that known channel credential, then require every observed
        # operation request to share one non-null provider account.
        excluded_bearers=(
            (emulate_slack_channel_bearer(),)
            if operation_case.provider_service == "slack"
            else ()
        ),
    )
    assert replay == {
        "source": source,
        "next_response": 3,
        "response_count": 3,
        "complete": True,
        "error": None,
    }


@pytest.mark.timeout(150)
@pytest.mark.parametrize(
    "fault_case",
    PROVIDER_FAULT_CASES,
    ids=lambda case: case.case_id,
)
async def test_provider_fault_profile_preserves_safe_operation_outcomes(
    reborn_provider_fault_server,
    mock_llm_server,
    fault_case,
):
    """Faults stay model-visible and never create an unproven duplicate effect."""
    operation = fault_case.operation
    server = reborn_provider_fault_server["base_url"]
    emulate_url = reborn_provider_fault_server[
        f"emulate_{operation.provider_service}_url"
    ]
    proxy = reborn_provider_fault_server["provider_fault_proxies"][
        operation.provider_service
    ]

    await operation.assert_baseline(emulate_url)
    arguments = await operation.resolve_arguments(emulate_url)
    trace = _provider_operation_trace(operation, arguments)
    trace["steps"][-1]["request_hint"] = {
        "expected_failed_tool_result_contains": fault_case.expected_tool_result
    }
    source = f"provider-fault-{fault_case.case_id}.json"
    await _install_inline_trace(mock_llm_server, source, trace)
    profile = PROVIDER_FAULT_PROFILES[fault_case.profile]
    proxy.arm(
        profile,
        method=fault_case.method,
        path=fault_case.path,
    )

    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, server)
        replay = await _submit_and_wait_for_trace_replay(
            client,
            server,
            thread_id,
            trace["steps"][0]["response"]["content"],
            mock_llm_server,
            timeout=120,
        )
        await wait_for_assistant_message(client, server, thread_id, timeout=120)
        timeline = await _fetch_all_timeline_pages_with_retry(client, server, thread_id)

    matches = [
        preview
        for message in timeline.get("messages", [])
        if (preview := capability_preview_payload(message)) is not None
        and preview["capability_id"] == operation.capability_id
    ]
    assert len(matches) == 1, matches
    assert matches[0]["status"] == "failed", matches[0]
    if fault_case.expected_preview_error is not None:
        assert fault_case.expected_preview_error in json.dumps(matches[0]), matches[0]

    attempts = [
        attempt
        for attempt in proxy.state["requests"]
        if attempt["method"] == fault_case.method and attempt["path"] == fault_case.path
    ]
    assert len(attempts) == 1, attempts
    assert attempts[0]["forwarded"] is fault_case.expected_forwarded
    assert attempts[0]["responded"] is (profile.action == "respond")

    async with httpx.AsyncClient(timeout=15) as provider_client:
        issues = await github_json(
            provider_client,
            emulate_url,
            "GET",
            "/repos/nearai/ironclaw/issues",
        )
    assert isinstance(issues, list)
    if fault_case.expected_outcome == "committed_without_ack":
        expected_title = arguments["title"]
        assert [issue["title"] for issue in issues].count(expected_title) == 1, issues
    else:
        assert len(issues) == 1, issues
        attempted_title = arguments.get("title")
        if attempted_title is not None:
            assert issues[0]["title"] != attempted_title, issues

    assert replay == {
        "source": source,
        "next_response": 3,
        "response_count": 3,
        "complete": True,
        "error": None,
    }
