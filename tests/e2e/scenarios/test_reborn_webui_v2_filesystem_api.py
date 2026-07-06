"""Served Reborn WebUI v2 project-file and filesystem-browser API tests.

These scenarios exercise the browser-facing `/api/webchat/v2/threads/*/files`
and `/api/webchat/v2/fs/*` routes through a real `ironclaw-reborn serve`
process. They replace QA-matrix coverage that used to be represented by Rust
contract tests, which are now owned by normal CI.
"""

import asyncio
from pathlib import Path

import httpx
import pytest

from helpers import REBORN_V2_AUTH_TOKEN
from reborn_webui_harness import (
    create_thread,
    reborn_bearer_headers,
    send_message,
    wait_for_assistant_message,
)

pytest_plugins = ["reborn_webui_harness"]

CSV_PATH = "/workspace/report.csv"
CSV_RELATIVE_PATH = "report.csv"
CSV_BYTES = b"name,score\nalice,90\nbob,85\n"
GENERATED_REPORT_FILES = (Path("report.csv"), Path("report.pdf"))


@pytest.fixture(autouse=True)
def cleanup_generated_report_files():
    for path in GENERATED_REPORT_FILES:
        path.unlink(missing_ok=True)
    yield
    for path in GENERATED_REPORT_FILES:
        path.unlink(missing_ok=True)


async def _produce_report_files(client: httpx.AsyncClient, base_url: str) -> str:
    thread_id = await create_thread(client, base_url)
    await send_message(
        client,
        base_url,
        thread_id,
        "Please produce a downloadable CSV and PDF report.",
    )
    await wait_for_assistant_message(client, base_url, thread_id)
    return thread_id


async def _wait_for_project_file_stat(
    client: httpx.AsyncClient,
    base_url: str,
    thread_id: str,
    path: str,
    *,
    timeout: float = 20.0,
) -> dict:
    last_response = None
    for _ in range(int(timeout * 2)):
        last_response = await client.get(
            f"{base_url}/api/webchat/v2/threads/{thread_id}/files/stat",
            params={"path": path},
            timeout=15,
        )
        if last_response.status_code == 200:
            return last_response.json()["stat"]
        await asyncio.sleep(0.5)
    pytest.fail(
        f"project file {path} was not readable; last status="
        f"{getattr(last_response, 'status_code', None)} body="
        f"{getattr(last_response, 'text', '')}"
    )


def _assert_attachment_download_headers(response: httpx.Response, filename: str) -> None:
    assert response.headers["x-content-type-options"] == "nosniff"
    disposition = response.headers["content-disposition"]
    assert disposition.startswith("attachment;")
    assert f'filename="{filename}"' in disposition


async def test_reborn_v2_project_file_routes_list_stat_and_read_served(
    reborn_v2_yolo_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _produce_report_files(client, reborn_v2_yolo_server)
        expected_stat = await _wait_for_project_file_stat(
            client, reborn_v2_yolo_server, thread_id, CSV_PATH
        )

        listed = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/threads/{thread_id}/files",
            timeout=15,
        )
        listed.raise_for_status()
        entries = listed.json()["entries"]
        report = next(entry for entry in entries if entry["name"] == "report.csv")
        assert report["path"] == CSV_PATH
        assert report["kind"] == "file"

        stat = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/threads/{thread_id}/files/stat",
            params={"path": CSV_PATH},
            timeout=15,
        )
        stat.raise_for_status()
        stat_body = stat.json()["stat"]
        assert stat_body == expected_stat
        assert stat_body["path"] == CSV_PATH
        assert stat_body["kind"] == "file"
        assert stat_body["size_bytes"] == len(CSV_BYTES)
        assert stat_body["mime_type"] == "text/csv"

        content = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/threads/{thread_id}/files/content",
            params={"path": CSV_PATH},
            timeout=15,
        )
        content.raise_for_status()
        assert content.content == CSV_BYTES
        assert content.headers["content-type"] == "text/csv"
        assert content.headers["content-length"] == str(len(CSV_BYTES))
        _assert_attachment_download_headers(content, "report.csv")


async def test_reborn_v2_filesystem_browser_mounts_list_stat_and_read_served(
    reborn_v2_yolo_server,
):
    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await _produce_report_files(client, reborn_v2_yolo_server)
        await _wait_for_project_file_stat(
            client, reborn_v2_yolo_server, thread_id, CSV_PATH
        )

        mounts = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/mounts",
            timeout=15,
        )
        mounts.raise_for_status()
        mount_ids = {mount["mount"] for mount in mounts.json()["mounts"]}
        assert "workspace" in mount_ids

        listed = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/list",
            params={"mount": "workspace"},
            timeout=15,
        )
        listed.raise_for_status()
        listed_body = listed.json()
        assert listed_body["mount"] == "workspace"
        entries = listed_body["entries"]
        report = next(entry for entry in entries if entry["name"] == "report.csv")
        assert report["path"] == CSV_RELATIVE_PATH
        assert report["kind"] == "file"
        assert not report["path"].startswith("/")

        stat = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/stat",
            params={"mount": "workspace", "path": CSV_RELATIVE_PATH},
            timeout=15,
        )
        stat.raise_for_status()
        stat_body = stat.json()["stat"]
        assert stat_body["path"] == CSV_RELATIVE_PATH
        assert stat_body["size_bytes"] == len(CSV_BYTES)
        assert stat_body["mime_type"] == "text/csv"

        content = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/content",
            params={"mount": "workspace", "path": CSV_RELATIVE_PATH},
            timeout=15,
        )
        content.raise_for_status()
        assert content.content == CSV_BYTES
        _assert_attachment_download_headers(content, "report.csv")


async def test_reborn_v2_filesystem_routes_reject_unauthorized_and_invalid_paths(
    reborn_v2_yolo_server,
):
    async with httpx.AsyncClient() as anonymous:
        for path in (
            "/api/webchat/v2/fs/mounts",
            "/api/webchat/v2/fs/list",
        ):
            response = await anonymous.get(f"{reborn_v2_yolo_server}{path}", timeout=15)
            assert response.status_code == 401

        unauthorized_project = await anonymous.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/threads/thread/files",
            timeout=15,
        )
        assert unauthorized_project.status_code == 401

    headers = reborn_bearer_headers()
    async with httpx.AsyncClient(headers=headers) as client:
        thread_id = await create_thread(client, reborn_v2_yolo_server)

        blank_project_stat = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/threads/{thread_id}/files/stat",
            params={"path": "   "},
            timeout=15,
        )
        assert blank_project_stat.status_code == 400
        assert blank_project_stat.json()["field"] == "path"

        blank_fs_content = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/content",
            params={"mount": "workspace", "path": ""},
            timeout=15,
        )
        assert blank_fs_content.status_code == 400
        assert blank_fs_content.json()["field"] == "path"

        unknown_mount = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/list",
            params={"mount": "unknown"},
            timeout=15,
        )
        assert unknown_mount.status_code == 400

        traversal = await client.get(
            f"{reborn_v2_yolo_server}/api/webchat/v2/fs/stat",
            params={"mount": "workspace", "path": "../secrets.toml"},
            timeout=15,
        )
        assert 400 <= traversal.status_code < 500
