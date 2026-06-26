"""Legacy tool-execution API checks ported to standalone Reborn WebUI v2."""

import asyncio
import json

import httpx

from reborn_webui_harness import (
    create_thread,
    fetch_timeline,
    reborn_bearer_headers,
    reborn_v2_yolo_server,  # noqa: F401 - imported fixture
    send_message,
    wait_for_assistant_message,
)


def _preview_payload(message: dict) -> dict | None:
    if message.get("kind") != "capability_display_preview":
        return None
    try:
        return json.loads(message.get("content") or "{}")
    except json.JSONDecodeError:
        return None


async def _wait_for_capability_preview(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    capability_id: str,
    *,
    output_fragment: str | None = None,
    timeout: float = 45.0,
) -> dict:
    last_timeline = {}
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        last_timeline = await fetch_timeline(client, base_url, thread_id)
        for message in last_timeline.get("messages", []):
            preview = _preview_payload(message)
            if not preview or preview.get("capability_id") != capability_id:
                continue
            output = (
                preview.get("output_preview")
                or preview.get("output_summary")
                or ""
            )
            if output_fragment and output_fragment.lower() not in output.lower():
                continue
            return preview
        await asyncio.sleep(0.25)

    raise AssertionError(
        f"Timed out waiting for {capability_id!r} preview in thread {thread_id}. "
        f"Last timeline: {last_timeline}"
    )


async def test_reborn_legacy_builtin_echo_tool_executes(reborn_v2_yolo_server):
    """Port of legacy builtin echo execution to Reborn's namespaced capability."""
    marker = "reborn tool execution echo 1429"
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            f"reborn builtin echo {marker}",
        )

        preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.echo",
            output_fragment=marker,
        )

    assert preview["status"] == "completed", preview
    assert marker in (preview.get("output_preview") or ""), preview


async def test_reborn_legacy_builtin_time_tool_executes(reborn_v2_yolo_server):
    """Port of legacy builtin time execution to Reborn's namespaced capability."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "reborn builtin time",
        )

        preview = await _wait_for_capability_preview(
            client,
            reborn_v2_yolo_server,
            thread_id,
            "builtin.time",
            output_fragment="utc_iso",
        )

    assert preview["status"] == "completed", preview
    assert "utc_iso" in (preview.get("output_preview") or ""), preview


async def test_reborn_legacy_non_tool_message_still_works(reborn_v2_yolo_server):
    """Port of legacy non-tool chat regression to Reborn's v2 timeline."""
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)
        await send_message(client, reborn_v2_yolo_server, thread_id, "What is 2+2?")

        assistant = await wait_for_assistant_message(
            client,
            reborn_v2_yolo_server,
            thread_id,
            timeout=30,
        )
        timeline = await fetch_timeline(client, reborn_v2_yolo_server, thread_id)

    assert "4" in (assistant.get("content") or ""), assistant
    assert [
        message
        for message in timeline.get("messages", [])
        if message.get("kind") == "capability_display_preview"
    ] == []
