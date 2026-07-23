"""Google Drive full-path provider operation cases."""

import json

from provider_operation_google_common import google_json
from provider_operation_types import ProviderOperationCase

FILE_ID = "drv_reborn_qa_brief"
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


async def _baseline(emulate_url: str) -> None:
    file = await _file(emulate_url)
    assert file["name"] == "Reborn QA Brief", file
    permissions = await _permissions(emulate_url)
    assert [permission["id"] for permission in permissions] == [
        SEEDED_PERMISSION_ID
    ], permissions


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
