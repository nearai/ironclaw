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
EDIT_MARKER = "REBORN_PROVIDER_CASE_SLACK_EDIT"
EDITED_TEXT = f"{EDIT_MARKER} edited body"
DELETE_MARKER = "REBORN_PROVIDER_CASE_SLACK_DELETE"
ADD_REACTION_EMOJI = "white_check_mark"
REMOVE_REACTION_EMOJI = "tada"
RESOLVE_EMPTY_QUERY = "NO_SUCH_PROVIDER_CONTRACT_PERSON"
EMPTY_USER_ID = "U_EMPTY"
EMPTY_TEAM_ID = "T_EMPTY"
LIST_MEMBER_ID = "UMEMBERCONTRACT"
LIST_MEMBER_NAME = "Member Contract"
MISSING_MESSAGE_TS = "1751960000.000001"


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
    return {"conversation": await _channel_id(emulate_url)}


async def _thread_arguments(emulate_url: str) -> dict:
    root = next(
        message
        for message in await _history(emulate_url)
        if message["text"] == THREAD_ROOT_MARKER
    )
    return {
        "conversation": await _channel_id(emulate_url),
        "thread": root["ts"],
        "limit": 50,
    }


async def _user_arguments(emulate_url: str) -> dict:
    return {"user_ref": await _user_id(emulate_url)}


async def _search_outcome(_emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    matches = [
        match for match in output["matches"] if SEARCH_MARKER in match["text"]
    ]
    assert len(matches) == 1, output
    assert matches[0]["author"]["display_name"] == "reborn-user", matches[0]
    assert matches[0]["is_self"] is True, matches[0]


async def _list_conversations_outcome(
    emulate_url: str, preview: dict
) -> None:
    channel_id = await _channel_id(emulate_url)
    output = _output(preview)
    matches = [
        item
        for item in output["conversations"]
        if item["conversation"] == channel_id
    ]
    assert len(matches) == 1, output
    assert matches[0]["kind"] == "channel", matches[0]
    assert matches[0]["display_name"] == CHANNEL_NAME, matches[0]
    assert matches[0]["is_member"] is True, matches[0]


async def _conversation_info_outcome(
    emulate_url: str, preview: dict
) -> None:
    channel_id = await _channel_id(emulate_url)
    output = _output(preview)
    assert output["conversation"] == channel_id, output
    assert output["kind"] == "channel", output
    assert output["display_name"] == CHANNEL_NAME, output
    assert output["is_member"] is True, output


async def _history_outcome(_emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    by_text = {message["text"]: message for message in output["messages"]}
    assert THREAD_ROOT_MARKER in by_text, output
    assert by_text[THREAD_ROOT_MARKER]["is_self"] is True, output


async def _thread_outcome(_emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    by_text = {message["text"]: message for message in output["messages"]}
    assert {THREAD_ROOT_MARKER, THREAD_REPLY_MARKER} <= set(by_text), output
    assert by_text[THREAD_REPLY_MARKER]["is_self"] is True, output
    assert by_text[THREAD_REPLY_MARKER]["thread"]["thread"] == by_text[
        THREAD_ROOT_MARKER
    ]["message_ref"]["message_id"], output


async def _user_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    assert output["user_ref"] == await _user_id(emulate_url), output
    assert output["real_name"] == "QA Reviewer", output
    assert output["title"] == "Release reviewer", output


async def _whoami_outcome(emulate_url: str, preview: dict) -> None:
    async with httpx.AsyncClient(timeout=15) as client:
        identity = await slack_post(client, emulate_url, "auth.test", {})
    output = _output(preview)
    assert output == {
        "display_name": identity["user"],
        "user_ref": identity["user_id"],
    }, output


async def _send_arguments(emulate_url: str) -> dict:
    return {
        "conversation": await _channel_id(emulate_url),
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
        "message_ref": {
            "conversation": await _channel_id(emulate_url),
            "message_id": matches[0]["ts"],
        }
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


async def _thread_root(emulate_url: str) -> dict:
    return next(
        message
        for message in await _history(emulate_url)
        if message["text"] == THREAD_ROOT_MARKER
    )


async def _thread_reply(emulate_url: str) -> dict:
    root = await _thread_root(emulate_url)
    async with httpx.AsyncClient(timeout=15) as client:
        thread = await slack_post(
            client,
            emulate_url,
            "conversations.replies",
            {"channel": await _channel_id(emulate_url), "ts": root["ts"]},
        )
    return next(
        message
        for message in thread["messages"]
        if message["text"] == THREAD_REPLY_MARKER
    )


async def _message_ref_arguments(emulate_url: str, ts: str) -> dict:
    return {
        "message_ref": {
            "conversation": await _channel_id(emulate_url),
            "message_id": ts,
        }
    }


async def _post_marker_message(emulate_url: str, text: str) -> dict:
    async with httpx.AsyncClient(timeout=15) as client:
        return await slack_post(
            client,
            emulate_url,
            "chat.postMessage",
            {"channel": await _channel_id(emulate_url), "text": text},
        )


async def _delete_marker_messages(emulate_url: str, marker: str) -> None:
    channel = await _channel_id(emulate_url)
    matches = [
        message
        for message in await _history(emulate_url)
        if marker in message["text"]
    ]
    async with httpx.AsyncClient(timeout=15) as client:
        for message in matches:
            await slack_post(
                client,
                emulate_url,
                "chat.delete",
                {"channel": channel, "ts": message["ts"]},
            )


async def _assert_no_marker(emulate_url: str, marker: str) -> None:
    await _baseline(emulate_url)
    assert not [
        message
        for message in await _history(emulate_url)
        if marker in message["text"]
    ], f"provider world already contains the {marker} contract marker"


async def _root_reactions(emulate_url: str) -> list[dict]:
    root = await _thread_root(emulate_url)
    async with httpx.AsyncClient(timeout=15) as client:
        result = await slack_post(
            client,
            emulate_url,
            "reactions.get",
            {
                "channel": await _channel_id(emulate_url),
                "timestamp": root["ts"],
            },
        )
    return result["message"].get("reactions", [])


async def _remove_root_reaction_if_present(emulate_url: str, name: str) -> None:
    if not any(
        reaction["name"] == name for reaction in await _root_reactions(emulate_url)
    ):
        return
    root = await _thread_root(emulate_url)
    async with httpx.AsyncClient(timeout=15) as client:
        await slack_post(
            client,
            emulate_url,
            "reactions.remove",
            {
                "channel": await _channel_id(emulate_url),
                "timestamp": root["ts"],
                "name": name,
            },
        )


async def _edit_baseline(emulate_url: str) -> None:
    await _assert_no_marker(emulate_url, EDIT_MARKER)


async def _edit_arguments(emulate_url: str) -> dict:
    posted = await _post_marker_message(emulate_url, f"{EDIT_MARKER} original body")
    arguments = await _message_ref_arguments(emulate_url, posted["ts"])
    return {**arguments, "text": EDITED_TEXT}


async def _edit_outcome(emulate_url: str, preview: dict) -> None:
    matches = [
        message
        for message in await _history(emulate_url)
        if EDIT_MARKER in message["text"]
    ]
    assert [message["text"] for message in matches] == [EDITED_TEXT], matches
    output = _output(preview)
    assert output == {
        "message_ref": {
            "conversation": await _channel_id(emulate_url),
            "message_id": matches[0]["ts"],
        }
    }, output


async def _cleanup_edit(emulate_url: str) -> None:
    await _delete_marker_messages(emulate_url, EDIT_MARKER)


async def _delete_baseline(emulate_url: str) -> None:
    await _assert_no_marker(emulate_url, DELETE_MARKER)


async def _delete_arguments(emulate_url: str) -> dict:
    posted = await _post_marker_message(emulate_url, f"{DELETE_MARKER} to remove")
    return await _message_ref_arguments(emulate_url, posted["ts"])


async def _delete_outcome(emulate_url: str, preview: dict) -> None:
    assert not [
        message
        for message in await _history(emulate_url)
        if DELETE_MARKER in message["text"]
    ], "the deleted contract message is still on the provider timeline"
    output = _output(preview)
    assert output["deleted"] is True, output
    assert output["message_ref"]["conversation"] == await _channel_id(
        emulate_url
    ), output


async def _cleanup_delete(emulate_url: str) -> None:
    # Defensive only: the capability leg already removed the message when the
    # case passed; this fires when the capability leg itself failed.
    await _delete_marker_messages(emulate_url, DELETE_MARKER)


async def _add_reaction_baseline(emulate_url: str) -> None:
    await _baseline(emulate_url)
    assert not [
        reaction
        for reaction in await _root_reactions(emulate_url)
        if reaction["name"] == ADD_REACTION_EMOJI
    ], "provider world already carries the add-reaction contract emoji"


async def _add_reaction_arguments(emulate_url: str) -> dict:
    root = await _thread_root(emulate_url)
    arguments = await _message_ref_arguments(emulate_url, root["ts"])
    return {**arguments, "emoji": ADD_REACTION_EMOJI}


async def _add_reaction_outcome(emulate_url: str, preview: dict) -> None:
    matches = [
        reaction
        for reaction in await _root_reactions(emulate_url)
        if reaction["name"] == ADD_REACTION_EMOJI
    ]
    assert len(matches) == 1, matches
    root = await _thread_root(emulate_url)
    output = _output(preview)
    assert output == {
        "message_ref": {
            "conversation": await _channel_id(emulate_url),
            "message_id": root["ts"],
        },
        "emoji": ADD_REACTION_EMOJI,
    }, output


async def _cleanup_add_reaction(emulate_url: str) -> None:
    await _remove_root_reaction_if_present(emulate_url, ADD_REACTION_EMOJI)


async def _remove_reaction_baseline(emulate_url: str) -> None:
    await _baseline(emulate_url)
    assert not [
        reaction
        for reaction in await _root_reactions(emulate_url)
        if reaction["name"] == REMOVE_REACTION_EMOJI
    ], "provider world already carries the remove-reaction contract emoji"


async def _remove_reaction_arguments(emulate_url: str) -> dict:
    # Seed the reaction as the SAME account the capability acts as (the
    # direct-API token and the connected OAuth account are the same seeded
    # user — pinned by the whoami case), because remove_reaction only ever
    # removes the connected account's own reaction.
    root = await _thread_root(emulate_url)
    async with httpx.AsyncClient(timeout=15) as client:
        await slack_post(
            client,
            emulate_url,
            "reactions.add",
            {
                "channel": await _channel_id(emulate_url),
                "timestamp": root["ts"],
                "name": REMOVE_REACTION_EMOJI,
            },
        )
    arguments = await _message_ref_arguments(emulate_url, root["ts"])
    return {**arguments, "emoji": REMOVE_REACTION_EMOJI}


async def _remove_reaction_outcome(emulate_url: str, preview: dict) -> None:
    assert not [
        reaction
        for reaction in await _root_reactions(emulate_url)
        if reaction["name"] == REMOVE_REACTION_EMOJI
    ], "the removed contract reaction is still on the provider message"
    root = await _thread_root(emulate_url)
    output = _output(preview)
    assert output == {
        "message_ref": {
            "conversation": await _channel_id(emulate_url),
            "message_id": root["ts"],
        },
        "emoji": REMOVE_REACTION_EMOJI,
    }, output


async def _cleanup_remove_reaction(emulate_url: str) -> None:
    # Defensive only: fires when the capability leg failed after seeding.
    await _remove_root_reaction_if_present(emulate_url, REMOVE_REACTION_EMOJI)


async def _open_dm_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    async with httpx.AsyncClient(timeout=15) as client:
        direct = await slack_post(
            client,
            emulate_url,
            "conversations.open",
            {"users": await _user_id(emulate_url)},
        )
    # Re-opening directly returns the SAME conversation the capability
    # reported — provider-issued evidence plus the idempotence the op
    # documents (open twice, one DM).
    assert output == {"conversation": direct["channel"]["id"]}, (output, direct)


async def _resolve_user_outcome(emulate_url: str, preview: dict) -> None:
    reviewer = await _user_id(emulate_url)
    output = _output(preview)
    matches = [
        match for match in output["matches"] if match["user_ref"] == reviewer
    ]
    assert len(matches) == 1, output
    assert matches[0]["display_name"], matches[0]


async def _get_message_arguments(emulate_url: str) -> dict:
    # The threaded REPLY is deliberately the target: it is never on the
    # top-level timeline, so the history leg misses and the case drives the
    # conversations.replies fallback — the interesting half of get_message —
    # against the real provider fixture.
    reply = await _thread_reply(emulate_url)
    return await _message_ref_arguments(emulate_url, reply["ts"])


async def _get_message_outcome(emulate_url: str, preview: dict) -> None:
    reply = await _thread_reply(emulate_url)
    root = await _thread_root(emulate_url)
    output = _output(preview)
    message = output["message"]
    assert message["message_ref"]["message_id"] == reply["ts"], output
    assert message["text"] == THREAD_REPLY_MARKER, output
    assert message["is_self"] is True, output
    assert message["thread"]["thread"] == root["ts"], output


async def _get_message_missing_outcome(_emulate_url: str, preview: dict) -> None:
    rendered = json.dumps(preview)
    assert "messaging.unknown_message" in rendered, preview


def _setup_missing_message(proxy) -> None:
    # A ref that resolves to nothing: the history page holds no exact ts and
    # the thread lookup is empty too. The guest must report the typed miss,
    # never the neighbouring message.
    history = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body='{"ok":true,"messages":[],"has_more":false}',
    )
    replies = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body='{"ok":true,"messages":[]}',
    )
    proxy.arm(history, method="GET", path="/api/conversations.history")
    proxy.arm(replies, method="GET", path="/api/conversations.replies")


def _setup_list_members(proxy) -> None:
    # Emulate serves conversations.members as POST-only at the pinned ref
    # while the guest reads it via GET (as real Slack allows), so the proxy
    # supplies the provider-authentic page and the member's users.info
    # lookup. Both arms answer the guest's real requests through the full
    # binary path.
    members = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=json.dumps(
            {
                "ok": True,
                "members": [LIST_MEMBER_ID],
                "response_metadata": {"next_cursor": ""},
            },
            separators=(",", ":"),
        ),
    )
    user = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=json.dumps(
            {
                "ok": True,
                "user": {
                    "id": LIST_MEMBER_ID,
                    "name": "member-contract",
                    "profile": {
                        "real_name": LIST_MEMBER_NAME,
                        "display_name": LIST_MEMBER_NAME,
                    },
                    "is_bot": False,
                },
            },
            separators=(",", ":"),
        ),
    )
    proxy.arm(members, method="GET", path="/api/conversations.members")
    proxy.arm(user, method="GET", path="/api/users.info")


def _setup_empty_members(proxy) -> None:
    members = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=(
            '{"ok":true,"members":[],'
            '"response_metadata":{"next_cursor":""}}'
        ),
    )
    proxy.arm(members, method="GET", path="/api/conversations.members")


def _empty_auth_profile() -> ProviderFaultProfile:
    return ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=json.dumps(
            {
                "ok": True,
                "user_id": EMPTY_USER_ID,
                "team_id": EMPTY_TEAM_ID,
            },
            separators=(",", ":"),
        ),
    )


def _setup_empty_slack_history(proxy, endpoint: str) -> None:
    history = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body='{"ok":true,"messages":[],"has_more":false}',
    )
    proxy.arm(history, method="GET", path=f"/api/{endpoint}")
    proxy.arm(_empty_auth_profile(), method="GET", path="/api/auth.test")


def _setup_empty_history(proxy) -> None:
    _setup_empty_slack_history(proxy, "conversations.history")


def _setup_empty_thread(proxy) -> None:
    _setup_empty_slack_history(proxy, "conversations.replies")


def _setup_empty_whoami(proxy) -> None:
    user = ProviderFaultProfile(
        name="provider_contract_empty",
        action="respond",
        status=200,
        body=(
            f'{{"ok":true,"user":{{"id":"{EMPTY_USER_ID}","name":"",'
            '"profile":{"real_name":"","display_name":""},'
            '"is_bot":false}}'
        ),
    )
    proxy.arm(_empty_auth_profile(), method="GET", path="/api/auth.test")
    proxy.arm(user, method="GET", path="/api/users.info")


SLACK_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="slack_search_messages",
        provider_service="slack",
        capability_id="slack.search_messages",
        arguments={"query": SEARCH_MARKER, "limit": 20},
        assert_baseline=_baseline,
        assert_outcome=_search_outcome,
        expected_request_count=3,
    ),
    ProviderOperationCase(
        case_id="slack_search_messages_empty",
        provider_service="slack",
        capability_id="slack.search_messages",
        arguments={"query": "NO_SUCH_PROVIDER_CONTRACT_MESSAGE", "limit": 20},
        assert_baseline=_baseline,
        assert_outcome=exact_output({"matches": [], "total": 0}),
        outcome_class="empty",
        expected_request_count=2,
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/api/search.messages",
            payload={"ok": True, "messages": {"total": 0, "matches": []}},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
        expected_forwarded_request_count=1,
        expected_profile_request_count=1,
    ),
    ProviderOperationCase(
        case_id="slack_list_conversations",
        provider_service="slack",
        capability_id="slack.list_conversations",
        arguments={"kinds": ["channel"], "limit": 200},
        assert_baseline=_baseline,
        assert_outcome=_list_conversations_outcome,
    ),
    ProviderOperationCase(
        case_id="slack_list_conversations_empty",
        provider_service="slack",
        capability_id="slack.list_conversations",
        arguments={"kinds": ["group_dm"], "limit": 200},
        assert_baseline=_baseline,
        assert_outcome=exact_output({"conversations": []}),
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
        arguments={"conversation": "C_EMPTY"},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "conversation": "C_EMPTY",
                "kind": "other",
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
        arguments={"conversation": "C_EMPTY", "limit": 50},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "messages": [],
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
        arguments={"conversation": "C_EMPTY", "thread": "0.000001", "limit": 50},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "messages": [],
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
        arguments={"user_ref": EMPTY_USER_ID},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "user_ref": EMPTY_USER_ID,
                "is_bot": False,
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/api/users.info",
            payload={
                "ok": True,
                "user": {
                    "id": EMPTY_USER_ID,
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
            {
                "user_ref": EMPTY_USER_ID,
            }
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
    ProviderOperationCase(
        case_id="slack_edit_message",
        provider_service="slack",
        capability_id="slack.edit_message",
        arguments=_edit_arguments,
        assert_baseline=_edit_baseline,
        assert_outcome=_edit_outcome,
        cleanup_provider=_cleanup_edit,
    ),
    ProviderOperationCase(
        case_id="slack_delete_message",
        provider_service="slack",
        capability_id="slack.delete_message",
        arguments=_delete_arguments,
        assert_baseline=_delete_baseline,
        assert_outcome=_delete_outcome,
        cleanup_provider=_cleanup_delete,
    ),
    ProviderOperationCase(
        case_id="slack_add_reaction",
        provider_service="slack",
        capability_id="slack.add_reaction",
        arguments=_add_reaction_arguments,
        assert_baseline=_add_reaction_baseline,
        assert_outcome=_add_reaction_outcome,
        cleanup_provider=_cleanup_add_reaction,
    ),
    ProviderOperationCase(
        case_id="slack_remove_reaction",
        provider_service="slack",
        capability_id="slack.remove_reaction",
        arguments=_remove_reaction_arguments,
        assert_baseline=_remove_reaction_baseline,
        assert_outcome=_remove_reaction_outcome,
        cleanup_provider=_cleanup_remove_reaction,
    ),
    ProviderOperationCase(
        case_id="slack_open_dm",
        provider_service="slack",
        capability_id="slack.open_dm",
        arguments=_user_arguments,
        assert_baseline=_baseline,
        assert_outcome=_open_dm_outcome,
    ),
    ProviderOperationCase(
        case_id="slack_get_message",
        provider_service="slack",
        capability_id="slack.get_message",
        arguments=_get_message_arguments,
        assert_baseline=_baseline,
        assert_outcome=_get_message_outcome,
        # history miss + replies fallback + auth.test/users.info enrichment.
        expected_request_count=4,
    ),
    ProviderOperationCase(
        case_id="slack_get_message_empty",
        provider_service="slack",
        capability_id="slack.get_message",
        arguments={
            "message_ref": {
                "conversation": "C_EMPTY",
                "message_id": MISSING_MESSAGE_TS,
            }
        },
        assert_baseline=_baseline,
        assert_outcome=_get_message_missing_outcome,
        outcome_class="empty",
        # get_message's canonical output REQUIRES the message, so its
        # no-result shape is the typed model-visible miss, not an empty
        # completed payload — see ExpectedCapabilityStatus.
        expected_status="failed",
        expected_failed_tool_result_contains="messaging.unknown_message",
        expected_request_count=2,
        setup_provider_proxy=_setup_missing_message,
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_resolve_user",
        provider_service="slack",
        capability_id="slack.resolve_user",
        arguments={"query": REVIEWER_NAME, "limit": 200},
        assert_baseline=_baseline,
        assert_outcome=_resolve_user_outcome,
    ),
    ProviderOperationCase(
        case_id="slack_resolve_user_empty",
        provider_service="slack",
        capability_id="slack.resolve_user",
        arguments={"query": RESOLVE_EMPTY_QUERY, "limit": 200},
        assert_baseline=_baseline,
        assert_outcome=exact_output({"matches": []}),
        outcome_class="empty",
    ),
    ProviderOperationCase(
        case_id="slack_list_members",
        provider_service="slack",
        capability_id="slack.list_members",
        arguments={"conversation": "C_MEMBERS_CONTRACT", "limit": 50},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "members": [
                    {
                        "user_ref": LIST_MEMBER_ID,
                        "display_name": LIST_MEMBER_NAME,
                    }
                ]
            }
        ),
        expected_request_count=2,
        setup_provider_proxy=_setup_list_members,
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="slack_list_members_empty",
        provider_service="slack",
        capability_id="slack.list_members",
        arguments={"conversation": "C_EMPTY", "limit": 50},
        assert_baseline=_baseline,
        assert_outcome=exact_output({"members": []}),
        outcome_class="empty",
        setup_provider_proxy=_setup_empty_members,
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
)
