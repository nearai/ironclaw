"""Slack provider operation contracts owned by the Slack test module."""

import json

import httpx
from emulate_provider import slack_post
from provider_fault_proxy import ProviderFaultProfile
from provider_operation_types import (
    ProviderOperationCase,
    exact_output,
    static_provider_json_response,
)

CHANNEL_NAME = "reborn-alerts"
REVIEWER_NAME = "qa-reviewer"
SEARCH_MARKER = "ENTITYMSG_1784643032040"
THREAD_ROOT_MARKER = "QA10 thread root"
THREAD_REPLY_MARKER = "QA10 visible thread reply"
SEND_MARKER = "REBORN_PROVIDER_CASE_SLACK_SEND"


async def _channel_id(emulate_url: str) -> str:
    async with httpx.AsyncClient(timeout=15) as client:
        result = await slack_post(
            client,
            emulate_url,
            "conversations.list",
            {"types": "public_channel"},
        )
    return next(
        channel["id"]
        for channel in result["channels"]
        if channel["name"] == CHANNEL_NAME
    )


async def _user_id(emulate_url: str) -> str:
    async with httpx.AsyncClient(timeout=15) as client:
        result = await slack_post(client, emulate_url, "users.list")
    return next(
        user["id"]
        for user in result["members"]
        if user["name"] == REVIEWER_NAME
    )


async def _history(emulate_url: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=15) as client:
        result = await slack_post(
            client,
            emulate_url,
            "conversations.history",
            {"channel": await _channel_id(emulate_url), "limit": 200},
        )
    return result["messages"]


def _output(preview: dict) -> dict:
    assert preview["truncated"] is False, preview
    result = json.loads(preview["output_preview"])
    assert isinstance(result, dict), preview
    return result


async def _baseline(emulate_url: str) -> None:
    assert await _channel_id(emulate_url)
    assert await _user_id(emulate_url)


async def _conversation_arguments(emulate_url: str) -> dict:
    return {"channel": await _channel_id(emulate_url)}


async def _thread_arguments(emulate_url: str) -> dict:
    root = next(
        message
        for message in await _history(emulate_url)
        if message["text"] == THREAD_ROOT_MARKER
    )
    return {
        "channel": await _channel_id(emulate_url),
        "thread_ts": root["ts"],
        "limit": 50,
    }


async def _user_arguments(emulate_url: str) -> dict:
    return {"user_id": await _user_id(emulate_url)}


async def _search_outcome(_emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    matches = [
        match for match in output["matches"] if SEARCH_MARKER in match["text"]
    ]
    assert len(matches) == 1, output
    assert matches[0]["user_display_name"] == "reborn-user", matches[0]


async def _list_conversations_outcome(
    emulate_url: str, preview: dict
) -> None:
    channel_id = await _channel_id(emulate_url)
    output = _output(preview)
    matches = [
        item
        for item in output["conversations"]
        if item["id"] == channel_id
    ]
    assert len(matches) == 1, output
    assert matches[0]["name"] == CHANNEL_NAME, matches[0]
    assert matches[0]["is_member"] is True, matches[0]


async def _conversation_info_outcome(
    emulate_url: str, preview: dict
) -> None:
    channel_id = await _channel_id(emulate_url)
    output = _output(preview)
    assert output["conversation"]["id"] == channel_id, output
    assert output["conversation"]["name"] == CHANNEL_NAME, output
    assert output["conversation"]["is_member"] is True, output


async def _history_outcome(_emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    by_text = {message["text"]: message for message in output["messages"]}
    assert THREAD_ROOT_MARKER in by_text, output
    assert by_text[THREAD_ROOT_MARKER]["is_current_user"] is True, output
    assert output["current_user_id"], output


async def _thread_outcome(_emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    by_text = {message["text"]: message for message in output["messages"]}
    assert {THREAD_ROOT_MARKER, THREAD_REPLY_MARKER} <= set(by_text), output
    assert by_text[THREAD_REPLY_MARKER]["is_current_user"] is True, output


async def _user_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    assert output["user"]["id"] == await _user_id(emulate_url), output
    assert output["user"]["real_name"] == "QA Reviewer", output
    assert output["user"]["title"] == "Release reviewer", output


async def _whoami_outcome(emulate_url: str, preview: dict) -> None:
    async with httpx.AsyncClient() as client:
        identity = await slack_post(client, emulate_url, "auth.test", {})
    output = _output(preview)
    assert output == {
        "ok": True,
        "team_id": identity["team_id"],
        "user_display_name": identity["user"],
        "user_id": identity["user_id"],
    }, output


async def _send_arguments(emulate_url: str) -> dict:
    return {
        "channel": await _channel_id(emulate_url),
        "text": SEND_MARKER,
    }


async def _send_baseline(emulate_url: str) -> None:
    await _baseline(emulate_url)
    assert not [
        message
        for message in await _history(emulate_url)
        if message["text"] == SEND_MARKER
    ], "provider world already contains the send-message contract marker"


async def _send_outcome(emulate_url: str, preview: dict) -> None:
    matches = [
        message
        for message in await _history(emulate_url)
        if message["text"] == SEND_MARKER
    ]
    assert len(matches) == 1, matches
    output = _output(preview)
    assert output == {
        "ok": True,
        "channel": await _channel_id(emulate_url),
        "ts": matches[0]["ts"],
    }, output


async def _cleanup_send(emulate_url: str) -> None:
    channel = await _channel_id(emulate_url)
    matches = [
        message
        for message in await _history(emulate_url)
        if message["text"] == SEND_MARKER
    ]
    async with httpx.AsyncClient(timeout=15) as client:
        for message in matches:
            await slack_post(
                client,
                emulate_url,
                "chat.delete",
                {"channel": channel, "ts": message["ts"]},
            )


def _setup_empty_slack_history(proxy, endpoint: str) -> None:
    history = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body='{"ok":true,"messages":[],"has_more":false}',
    )
    auth = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body='{"ok":true,"user_id":"U_EMPTY","team_id":"T_EMPTY"}',
    )
    proxy.arm(history, method="GET", path=f"/api/{endpoint}")
    proxy.arm(auth, method="GET", path="/api/auth.test")


def _setup_empty_history(proxy) -> None:
    _setup_empty_slack_history(proxy, "conversations.history")


def _setup_empty_thread(proxy) -> None:
    _setup_empty_slack_history(proxy, "conversations.replies")


def _setup_empty_whoami(proxy) -> None:
    auth = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body='{"ok":true,"user_id":"U_EMPTY","team_id":"T_EMPTY"}',
    )
    user = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=(
            '{"ok":true,"user":{"id":"U_EMPTY","name":"",'
            '"profile":{"real_name":"","display_name":""},'
            '"is_bot":false}}'
        ),
    )
    proxy.arm(auth, method="GET", path="/api/auth.test")
    proxy.arm(user, method="GET", path="/api/users.info")


SLACK_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="slack_search_messages",
        provider_service="slack",
        capability_id="slack.search_messages",
        arguments={"query": SEARCH_MARKER, "count": 20},
        assert_baseline=_baseline,
        assert_outcome=_search_outcome,
        expected_request_count=2,
    ),
    ProviderOperationCase(
        case_id="slack_search_messages_empty",
        provider_service="slack",
        capability_id="slack.search_messages",
        arguments={"query": "NO_SUCH_PROVIDER_CONTRACT_MESSAGE", "count": 20},
        assert_baseline=_baseline,
        assert_outcome=exact_output({"ok": True, "total": 0, "matches": []}),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/api/search.messages",
            payload={"ok": True, "messages": {"total": 0, "matches": []}},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_list_conversations",
        provider_service="slack",
        capability_id="slack.list_conversations",
        arguments={"types": "public_channel", "limit": 200},
        assert_baseline=_baseline,
        assert_outcome=_list_conversations_outcome,
    ),
    ProviderOperationCase(
        case_id="slack_list_conversations_empty",
        provider_service="slack",
        capability_id="slack.list_conversations",
        arguments={"types": "mpim", "limit": 200},
        assert_baseline=_baseline,
        assert_outcome=exact_output({"ok": True, "conversations": []}),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/api/conversations.list",
            payload={
                "ok": True,
                "channels": [],
                "response_metadata": {"next_cursor": ""},
            },
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_get_conversation_info",
        provider_service="slack",
        capability_id="slack.get_conversation_info",
        arguments=_conversation_arguments,
        assert_baseline=_baseline,
        assert_outcome=_conversation_info_outcome,
    ),
    ProviderOperationCase(
        case_id="slack_get_conversation_info_empty",
        provider_service="slack",
        capability_id="slack.get_conversation_info",
        arguments={"channel": "C_EMPTY"},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "ok": True,
                "conversation": {
                    "id": "C_EMPTY",
                    "is_channel": False,
                    "is_private": False,
                    "is_im": False,
                    "is_mpim": False,
                },
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/api/conversations.info",
            payload={"ok": True, "channel": {"id": "C_EMPTY"}},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_get_conversation_history",
        provider_service="slack",
        capability_id="slack.get_conversation_history",
        arguments=_conversation_arguments,
        assert_baseline=_baseline,
        assert_outcome=_history_outcome,
        expected_request_count=3,
    ),
    ProviderOperationCase(
        case_id="slack_get_conversation_history_empty",
        provider_service="slack",
        capability_id="slack.get_conversation_history",
        arguments={"channel": "C_EMPTY", "limit": 50},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "ok": True,
                "messages": [],
                "has_more": False,
                "current_user_id": "U_EMPTY",
            }
        ),
        outcome_class="empty",
        expected_request_count=2,
        setup_provider_proxy=_setup_empty_history,
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_get_thread_replies",
        provider_service="slack",
        capability_id="slack.get_thread_replies",
        arguments=_thread_arguments,
        assert_baseline=_baseline,
        assert_outcome=_thread_outcome,
        expected_request_count=3,
    ),
    ProviderOperationCase(
        case_id="slack_get_thread_replies_empty",
        provider_service="slack",
        capability_id="slack.get_thread_replies",
        arguments={"channel": "C_EMPTY", "thread_ts": "0.000001", "limit": 50},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "ok": True,
                "messages": [],
                "has_more": False,
                "current_user_id": "U_EMPTY",
            }
        ),
        outcome_class="empty",
        expected_request_count=2,
        setup_provider_proxy=_setup_empty_thread,
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_get_user_info",
        provider_service="slack",
        capability_id="slack.get_user_info",
        arguments=_user_arguments,
        assert_baseline=_baseline,
        assert_outcome=_user_outcome,
    ),
    ProviderOperationCase(
        case_id="slack_get_user_info_empty",
        provider_service="slack",
        capability_id="slack.get_user_info",
        arguments={"user_id": "U_EMPTY"},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "ok": True,
                "user": {
                    "id": "U_EMPTY",
                    "name": "",
                    "real_name": "",
                    "display_name": "",
                    "is_bot": False,
                },
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/api/users.info",
            payload={
                "ok": True,
                "user": {
                    "id": "U_EMPTY",
                    "name": "",
                    "profile": {"real_name": "", "display_name": ""},
                    "is_bot": False,
                },
            },
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_whoami",
        provider_service="slack",
        capability_id="slack.whoami",
        arguments={},
        assert_baseline=_baseline,
        assert_outcome=_whoami_outcome,
        expected_request_count=2,
    ),
    ProviderOperationCase(
        case_id="slack_whoami_empty",
        provider_service="slack",
        capability_id="slack.whoami",
        arguments={},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {"ok": True, "user_id": "U_EMPTY", "team_id": "T_EMPTY"}
        ),
        outcome_class="empty",
        expected_request_count=2,
        setup_provider_proxy=_setup_empty_whoami,
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_send_message",
        provider_service="slack",
        capability_id="slack.send_message",
        arguments=_send_arguments,
        assert_baseline=_send_baseline,
        assert_outcome=_send_outcome,
        cleanup_provider=_cleanup_send,
    ),
)
