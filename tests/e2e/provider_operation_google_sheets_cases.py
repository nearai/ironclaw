"""Google Sheets full-path provider operation cases."""

import json

from provider_operation_google_common import google_json
from provider_operation_types import ProviderOperationCase

SPREADSHEET_ID = "sheet_reborn_abc"
ADDED_SHEET = "ProviderCase"


async def _spreadsheet(emulate_url: str) -> dict:
    result = await google_json(
        emulate_url, "GET", f"/v4/spreadsheets/{SPREADSHEET_ID}"
    )
    assert isinstance(result, dict)
    return result


async def _values(emulate_url: str, range_name: str) -> list[list]:
    result = await google_json(
        emulate_url,
        "GET",
        f"/v4/spreadsheets/{SPREADSHEET_ID}/values/{range_name}",
    )
    assert isinstance(result, dict)
    return result.get("values", [])


async def _baseline(emulate_url: str) -> None:
    spreadsheet = await _spreadsheet(emulate_url)
    sheets = spreadsheet["sheets"]
    assert [
        (sheet["properties"]["sheetId"], sheet["properties"]["title"])
        for sheet in sheets
    ] == [(0, "Sheet1"), (7, "DeleteMe")], sheets
    values = await _values(emulate_url, "Sheet1!A1:E2")
    assert values[1][-1] == "REBORN_QA_SEEDED", values


async def _batch_read_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    rendered = json.dumps(preview)
    assert "REBORN_QA_SEEDED" in rendered, preview
    assert "NEAR AI" in rendered, preview


async def _clear_outcome(emulate_url: str, preview: dict) -> None:
    values = await _values(emulate_url, "Sheet1!A1:E2")
    assert values == [
        ["Company", "Contact", "Source", "Status", "QA Marker"]
    ], values
    assert "Sheet1!A2:E2" in json.dumps(preview), preview


async def _add_sheet_outcome(emulate_url: str, preview: dict) -> None:
    spreadsheet = await _spreadsheet(emulate_url)
    matches = [
        sheet
        for sheet in spreadsheet["sheets"]
        if sheet["properties"]["title"] == ADDED_SHEET
    ]
    assert len(matches) == 1, spreadsheet
    assert matches[0]["properties"]["sheetId"] == 8, matches[0]
    assert ADDED_SHEET in json.dumps(preview), preview


async def _delete_sheet_outcome(emulate_url: str, preview: dict) -> None:
    spreadsheet = await _spreadsheet(emulate_url)
    assert [
        sheet["properties"]["sheetId"] for sheet in spreadsheet["sheets"]
    ] == [0], spreadsheet
    assert SPREADSHEET_ID in json.dumps(preview), preview


async def _format_cells_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    assert SPREADSHEET_ID in json.dumps(preview), preview


GOOGLE_SHEETS_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="google_sheets_batch_read_values",
        provider_service="google",
        capability_id="google-sheets.batch_read_values",
        arguments={
            "spreadsheet_id": SPREADSHEET_ID,
            "ranges": ["Sheet1!A1:B2", "Sheet1!E1:E2"],
        },
        assert_baseline=_baseline,
        assert_outcome=_batch_read_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_clear_values",
        provider_service="google",
        capability_id="google-sheets.clear_values",
        arguments={
            "spreadsheet_id": SPREADSHEET_ID,
            "range": "Sheet1!A2:E2",
        },
        assert_baseline=_baseline,
        assert_outcome=_clear_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_add_sheet",
        provider_service="google",
        capability_id="google-sheets.add_sheet",
        arguments={"spreadsheet_id": SPREADSHEET_ID, "title": ADDED_SHEET},
        assert_baseline=_baseline,
        assert_outcome=_add_sheet_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_delete_sheet",
        provider_service="google",
        capability_id="google-sheets.delete_sheet",
        arguments={"spreadsheet_id": SPREADSHEET_ID, "sheet_id": 7},
        assert_baseline=_baseline,
        assert_outcome=_delete_sheet_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_format_cells",
        provider_service="google",
        capability_id="google-sheets.format_cells",
        arguments={
            "spreadsheet_id": SPREADSHEET_ID,
            "sheet_id": 0,
            "start_row": 0,
            "end_row": 1,
            "start_column": 0,
            "end_column": 5,
            "bold": True,
            "background_color": "#D9EAD3",
            "horizontal_alignment": "CENTER",
        },
        assert_baseline=_baseline,
        assert_outcome=_format_cells_outcome,
    ),
)
