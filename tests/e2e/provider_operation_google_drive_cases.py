"""Google Drive full-path provider operation cases."""

import json

import httpx

from emulate_provider import google_headers, google_json
from provider_operation_types import ProviderOperationCase

FILE_ID = "drv_reborn_qa_brief"
CREATED_FOLDER = "REBORN_PROVIDER_CASE_CREATED_FOLDER"
UPLOADED_CONTENT = "Uploaded through the reusable provider operation runner."
UPLOADED_FILE = "REBORN_PROVIDER_CASE_UPLOADED_FILE.txt"
SEEDED_PERMISSION_ID = "perm_reborn_reader"
SHARED_DRIVE_ID = "shared_reborn_engineering"
SHARED_EMAIL = "provider-case-reader@example.com"


async def _file(emulate_url: str, *, expected_status: int = 200) -> dict:
    result = await google_json(
        emulate_url,
        "GET",
        f"/drive/v3/files/{FILE_ID}",
        expected_status=expected_status,
    )
    assert isinstance(result, dict)
    return result


async def _permissions(emulate_url: str) -> list[dict]:
    result = await google_json(
        emulate_url,
        "GET",
        f"/drive/v3/files/{FILE_ID}/permissions",
    )
    assert isinstance(result, dict)
    return result["permissions"]


async def _files_named(emulate_url: str, name: str) -> list[dict]:
    result = await google_json(
        emulate_url,
        "GET",
        "/drive/v3/files",
        params={"q": f"name = '{name}' and trashed = false", "pageSize": 100},
    )
    assert isinstance(result, dict)
    return result.get("files", [])


async def _media(emulate_url: str, file_id: str) -> str:
    async with httpx.AsyncClient(headers=google_headers(), timeout=15) as client:
        response = await client.get(
            f"{emulate_url}/drive/v3/files/{file_id}",
            params={"alt": "media"},
        )
    response.raise_for_status()
    return response.text


async def _seeded_file_baseline(emulate_url: str) -> None:
    file = await _file(emulate_url)
    assert file["name"] == "Reborn QA Brief", file


async def _baseline(emulate_url: str) -> None:
    await _seeded_file_baseline(emulate_url)
    permissions = await _permissions(emulate_url)
    assert [permission["id"] for permission in permissions] == [
        SEEDED_PERMISSION_ID
    ], permissions


async def _get_file_outcome(emulate_url: str, preview: dict) -> None:
    await _seeded_file_baseline(emulate_url)
    assert "Reborn QA Brief" in json.dumps(preview), preview


async def _update_file_outcome(emulate_url: str, preview: dict) -> None:
    file = await _file(emulate_url)
    assert file["name"] == "REBORN_PROVIDER_CASE_UPDATED_FILE", file
    assert "REBORN_PROVIDER_CASE_UPDATED_FILE" in json.dumps(preview), preview


async def _create_folder_baseline(emulate_url: str) -> None:
    assert not await _files_named(emulate_url, CREATED_FOLDER)


async def _create_folder_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _files_named(emulate_url, CREATED_FOLDER)
    assert len(matches) == 1, matches
    assert matches[0]["mimeType"] == "application/vnd.google-apps.folder", matches[0]
    assert matches[0]["parents"] == ["root"], matches[0]
    assert CREATED_FOLDER in json.dumps(preview), preview


async def _upload_file_baseline(emulate_url: str) -> None:
    assert not await _files_named(emulate_url, UPLOADED_FILE)


async def _upload_file_outcome(emulate_url: str, preview: dict) -> None:
    matches = await _files_named(emulate_url, UPLOADED_FILE)
    assert len(matches) == 1, matches
    uploaded = matches[0]
    assert uploaded["mimeType"] == "text/plain", uploaded
    assert uploaded["parents"] == ["root"], uploaded
    assert uploaded["size"] == str(len(UPLOADED_CONTENT.encode())), uploaded
    assert await _media(emulate_url, uploaded["id"]) == UPLOADED_CONTENT
    assert UPLOADED_FILE in json.dumps(preview), preview


async def _delete_outcome(emulate_url: str, preview: dict) -> None:
    missing = await _file(emulate_url, expected_status=404)
    assert missing["error"]["code"] == 404, missing
    assert FILE_ID in json.dumps(preview), preview


async def _trash_outcome(emulate_url: str, preview: dict) -> None:
    file = await _file(emulate_url)
    assert file["trashed"] is True, file
    assert FILE_ID in json.dumps(preview), preview


async def _list_permissions_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    rendered = json.dumps(preview)
    assert SEEDED_PERMISSION_ID in rendered, preview
    assert "seeded-reader@example.com" in rendered, preview


async def _share_outcome(emulate_url: str, preview: dict) -> None:
    permissions = await _permissions(emulate_url)
    matches = [
        permission
        for permission in permissions
        if permission.get("emailAddress") == SHARED_EMAIL
    ]
    assert len(matches) == 1, permissions
    assert matches[0]["role"] == "reader", matches[0]
    assert SHARED_EMAIL in json.dumps(preview), preview


async def _remove_permission_outcome(emulate_url: str, preview: dict) -> None:
    assert not await _permissions(emulate_url)
    assert SEEDED_PERMISSION_ID in json.dumps(preview), preview


async def _shared_drives_outcome(emulate_url: str, preview: dict) -> None:
    result = await google_json(emulate_url, "GET", "/drive/v3/drives")
    assert isinstance(result, dict)
    assert result["drives"] == [
        {
            "kind": "drive#drive",
            "id": SHARED_DRIVE_ID,
            "name": "Reborn Engineering",
        }
    ], result
    rendered = json.dumps(preview)
    assert SHARED_DRIVE_ID in rendered, preview
    assert "Reborn Engineering" in rendered, preview


GOOGLE_DRIVE_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="google_drive_get_file",
        provider_service="google",
        capability_id="google-drive.get_file",
        arguments={"file_id": FILE_ID},
        assert_baseline=_seeded_file_baseline,
        assert_outcome=_get_file_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_update_file",
        provider_service="google",
        capability_id="google-drive.update_file",
        arguments={
            "file_id": FILE_ID,
            "name": "REBORN_PROVIDER_CASE_UPDATED_FILE",
        },
        assert_baseline=_seeded_file_baseline,
        assert_outcome=_update_file_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_create_folder",
        provider_service="google",
        capability_id="google-drive.create_folder",
        arguments={"name": CREATED_FOLDER, "parent_id": "root"},
        assert_baseline=_create_folder_baseline,
        assert_outcome=_create_folder_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_upload_file",
        provider_service="google",
        capability_id="google-drive.upload_file",
        arguments={
            "name": UPLOADED_FILE,
            "content": UPLOADED_CONTENT,
            "mime_type": "text/plain",
            "parent_id": "root",
        },
        assert_baseline=_upload_file_baseline,
        assert_outcome=_upload_file_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_delete_file",
        provider_service="google",
        capability_id="google-drive.delete_file",
        arguments={"file_id": FILE_ID},
        assert_baseline=_baseline,
        assert_outcome=_delete_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_trash_file",
        provider_service="google",
        capability_id="google-drive.trash_file",
        arguments={"file_id": FILE_ID},
        assert_baseline=_baseline,
        assert_outcome=_trash_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_list_permissions",
        provider_service="google",
        capability_id="google-drive.list_permissions",
        arguments={"file_id": FILE_ID},
        assert_baseline=_baseline,
        assert_outcome=_list_permissions_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_share_file",
        provider_service="google",
        capability_id="google-drive.share_file",
        arguments={
            "file_id": FILE_ID,
            "email": SHARED_EMAIL,
            "role": "reader",
            "message": "Provider contract test",
        },
        assert_baseline=_baseline,
        assert_outcome=_share_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_remove_permission",
        provider_service="google",
        capability_id="google-drive.remove_permission",
        arguments={
            "file_id": FILE_ID,
            "permission_id": SEEDED_PERMISSION_ID,
        },
        assert_baseline=_baseline,
        assert_outcome=_remove_permission_outcome,
    ),
    ProviderOperationCase(
        case_id="google_drive_list_shared_drives",
        provider_service="google",
        capability_id="google-drive.list_shared_drives",
        arguments={"page_size": 10},
        assert_baseline=_baseline,
        assert_outcome=_shared_drives_outcome,
    ),
)
