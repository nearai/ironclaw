"""Google Sheets full-path provider operation cases."""

import json

from emulate_provider import google_json
from provider_operation_types import (
    ProviderOperationCase,
    exact_output,
    static_provider_json_response,
)

SPREADSHEET_ID = "sheet_reborn_abc"
ADDED_SHEET = "ProviderCase"
RENAMED_SHEET = "RenamedByProviderCase"
# Deliberately below the seeded rows so a write here cannot be confused with an
# append, and so the empty-range read has somewhere to look that is genuinely
# empty rather than merely trimmed.
WRITE_RANGE = "Sheet1!A4:C4"
EMPTY_RANGE = "Sheet1!A50:C60"
WRITTEN_ROW = ["REBORN_WRITE_VALUES", "isolated", "row"]


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


async def _batch_read_empty_outcome(emulate_url: str, preview: dict) -> None:
    assert not await _values(emulate_url, EMPTY_RANGE)
    assert preview["truncated"] is False, preview
    assert json.loads(preview["output_preview"]) == {
        "value_ranges": [{"range": EMPTY_RANGE, "values": []}]
    }, preview


async def _get_spreadsheet_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    assert preview["truncated"] is False, preview
    output = json.loads(preview["output_preview"])
    assert output["spreadsheet_id"] == SPREADSHEET_ID, output
    assert output["title"] == "ABC", output
    assert [(sheet["sheet_id"], sheet["title"]) for sheet in output["sheets"]] == [
        (0, "Sheet1"),
        (7, "DeleteMe"),
    ], output


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


async def _write_values_baseline(emulate_url: str) -> None:
    await _baseline(emulate_url)
    assert not await _values(emulate_url, WRITE_RANGE), (
        "provider world already has data in the write-values target range"
    )


async def _write_values_outcome(emulate_url: str, preview: dict) -> None:
    # Assert the exact target range, not "a row exists somewhere". qa_7e could
    # not tell write_values from its sibling append_values because it only
    # checked that one marker landed in a wide range; pinning A4:C4 can only
    # be satisfied by the write.
    assert await _values(emulate_url, WRITE_RANGE) == [WRITTEN_ROW], (
        await _values(emulate_url, WRITE_RANGE)
    )
    seeded = await _values(emulate_url, "Sheet1!A1:E2")
    assert seeded[1][-1] == "REBORN_QA_SEEDED", seeded
    assert "REBORN_WRITE_VALUES" in json.dumps(preview), preview


async def _rename_sheet_outcome(emulate_url: str, preview: dict) -> None:
    spreadsheet = await _spreadsheet(emulate_url)
    titles = {
        sheet["properties"]["sheetId"]: sheet["properties"]["title"]
        for sheet in spreadsheet["sheets"]
    }
    # Same sheetId, new title: proves a rename rather than a delete + recreate.
    assert titles == {0: "Sheet1", 7: RENAMED_SHEET}, spreadsheet
    assert RENAMED_SHEET in json.dumps(preview), preview


async def _read_values_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    rendered = json.dumps(preview)
    assert "REBORN_QA_SEEDED" in rendered, preview
    assert "Company" in rendered, preview


async def _read_values_empty_outcome(emulate_url: str, preview: dict) -> None:
    # The contract is positive, not merely "no marker leaked": the model is
    # handed an explicit empty values array for the range it asked about, so it
    # can distinguish "the sheet has nothing there" from "the call failed".
    assert not await _values(emulate_url, EMPTY_RANGE)
    assert preview["truncated"] is False, preview
    assert json.loads(preview["output_preview"]) == {
        "range": EMPTY_RANGE,
        "values": [],
    }, preview


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
        case_id="google_sheets_batch_read_values_empty",
        provider_service="google",
        capability_id="google-sheets.batch_read_values",
        arguments={
            "spreadsheet_id": SPREADSHEET_ID,
            "ranges": [EMPTY_RANGE],
        },
        assert_baseline=_baseline,
        assert_outcome=_batch_read_empty_outcome,
        outcome_class="empty",
    ),
    ProviderOperationCase(
        case_id="google_sheets_get_spreadsheet",
        provider_service="google",
        capability_id="google-sheets.get_spreadsheet",
        arguments={"spreadsheet_id": SPREADSHEET_ID},
        assert_baseline=_baseline,
        assert_outcome=_get_spreadsheet_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_get_spreadsheet_empty",
        provider_service="google",
        capability_id="google-sheets.get_spreadsheet",
        arguments={"spreadsheet_id": "sheet_provider_contract_empty"},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "spreadsheet_id": "",
                "title": "",
                "url": "",
                "sheets": [],
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/v4/spreadsheets/sheet_provider_contract_empty",
            payload={},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
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
        case_id="google_sheets_write_values",
        provider_service="google",
        capability_id="google-sheets.write_values",
        arguments={
            "spreadsheet_id": SPREADSHEET_ID,
            "range": WRITE_RANGE,
            "values": [WRITTEN_ROW],
        },
        assert_baseline=_write_values_baseline,
        assert_outcome=_write_values_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_rename_sheet",
        provider_service="google",
        capability_id="google-sheets.rename_sheet",
        arguments={
            "spreadsheet_id": SPREADSHEET_ID,
            "sheet_id": 7,
            "title": RENAMED_SHEET,
        },
        assert_baseline=_baseline,
        assert_outcome=_rename_sheet_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_read_values",
        provider_service="google",
        capability_id="google-sheets.read_values",
        arguments={"spreadsheet_id": SPREADSHEET_ID, "range": "Sheet1!A1:E2"},
        assert_baseline=_baseline,
        assert_outcome=_read_values_outcome,
    ),
    ProviderOperationCase(
        case_id="google_sheets_read_values_empty",
        provider_service="google",
        capability_id="google-sheets.read_values",
        arguments={"spreadsheet_id": SPREADSHEET_ID, "range": EMPTY_RANGE},
        assert_baseline=_baseline,
        assert_outcome=_read_values_empty_outcome,
        outcome_class="empty",
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
