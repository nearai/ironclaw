"""Google Docs full-path provider operation cases."""

import json

from emulate_provider import google_json
from provider_operation_types import (
    ProviderOperationCase,
    exact_output,
    exact_text_output,
    static_provider_json_response,
)

DOCUMENT_ID = "doc_reborn_strategy"
SEEDED_TEXT = (
    "NEAR AI Strategy: user-owned agents keep credentials and data under user control."
)
BATCH_MARKER = " REBORN_PROVIDER_CASE_BATCH"
CREATE_TITLE = "Reborn Provider Operation Created Document"
INSERT_MARKER = " REBORN_PROVIDER_CASE_INSERT"
REPLACEMENT = "customer-owned agents"
SEMANTIC_REPLACEMENT = "sovereign agents"
TABLE_DATA = [["Owner", "Status"], ["Ada", "Ready"]]


async def _document(emulate_url: str) -> dict:
    result = await google_json(
        emulate_url, "GET", f"/v1/documents/{DOCUMENT_ID}"
    )
    assert isinstance(result, dict)
    return result


def _document_text(document: dict) -> str:
    return "".join(
        element.get("textRun", {}).get("content", "")
        for item in document["body"]["content"]
        for element in item.get("paragraph", {}).get("elements", [])
    )


def _document_tables(document: dict) -> list[list[list[str]]]:
    tables = []
    for item in document["body"]["content"]:
        table = item.get("table")
        if table is None:
            continue
        rows = []
        for row in table.get("tableRows", []):
            cells = []
            for cell in row.get("tableCells", []):
                text = "".join(
                    element.get("textRun", {}).get("content", "")
                    for structural in cell.get("content", [])
                    for element in structural.get("paragraph", {}).get("elements", [])
                ).rstrip("\n")
                cells.append(text)
            rows.append(cells)
        tables.append(rows)
    return tables


async def _baseline(emulate_url: str) -> None:
    document = await _document(emulate_url)
    assert document["revisionId"] == "1", document
    assert _document_text(document) == SEEDED_TEXT, document


async def _get_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    rendered = json.dumps(preview)
    assert "NEAR AI Strategy" in rendered, preview
    assert DOCUMENT_ID in rendered, preview


async def _read_content_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    await exact_text_output(SEEDED_TEXT)(emulate_url, preview)


async def _inspect_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    output = _output(preview)
    assert output["document_id"] == DOCUMENT_ID, output
    assert output["elements"][0]["kind"] == "paragraph", output
    assert SEEDED_TEXT in output["elements"][0]["text"], output


async def _verify_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    output = _output(preview)
    assert output["verified"] is True, output
    assert all(check["passed"] for check in output["checks"]), output


async def _semantic_edit_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    assert output["verified"] is True, output
    document = await _document(emulate_url)
    assert SEMANTIC_REPLACEMENT in _document_text(document), document
    assert "user-owned agents" not in _document_text(document), document


async def _semantic_table_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    assert output["verified"] is True, output
    assert output["stage"] == "verified", output
    assert "failure" not in output, output
    assert output["populated_cells"] == 4, output
    document = await _document(emulate_url)
    assert TABLE_DATA in _document_tables(document), document


def _output(preview: dict) -> dict:
    assert preview["truncated"] is False, preview
    output = json.loads(preview["output_preview"])
    assert isinstance(output, dict), preview
    return output


async def _create_document_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    assert set(output) == {"document_id", "title"}, output
    assert output["document_id"], output
    assert output["title"] == CREATE_TITLE, output
    document = await google_json(
        emulate_url, "GET", f"/v1/documents/{output['document_id']}"
    )
    assert document["documentId"] == output["document_id"], document
    assert document["title"] == CREATE_TITLE, document
    assert _document_text(document) == "", document


async def _insert_text_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    assert output["document_id"] == DOCUMENT_ID, output
    assert output["revision_id"] == "2", output
    document = await _document(emulate_url)
    assert document["revisionId"] == "2", document
    assert _document_text(document) == f"{SEEDED_TEXT}{INSERT_MARKER}", document


def _revision_outcome(marker: str | None = None):
    async def assert_outcome(emulate_url: str, preview: dict) -> None:
        document = await _document(emulate_url)
        assert document["revisionId"] == "2", document
        if marker is not None:
            assert marker in json.dumps(preview), preview

    return assert_outcome


async def _batch_outcome(emulate_url: str, preview: dict) -> None:
    document = await _document(emulate_url)
    assert _document_text(document) == f"{SEEDED_TEXT}{BATCH_MARKER}", document
    assert document["revisionId"] == "2", document
    assert DOCUMENT_ID in json.dumps(preview), preview


async def _delete_outcome(emulate_url: str, preview: dict) -> None:
    document = await _document(emulate_url)
    assert _document_text(document) == SEEDED_TEXT[5:], document
    assert document["revisionId"] == "2", document
    assert DOCUMENT_ID in json.dumps(preview), preview


async def _replace_outcome(emulate_url: str, preview: dict) -> None:
    document = await _document(emulate_url)
    assert REPLACEMENT in _document_text(document), document
    assert "user-owned agents" not in _document_text(document), document
    assert document["revisionId"] == "2", document
    assert DOCUMENT_ID in json.dumps(preview), preview


GOOGLE_DOCS_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="google_docs_create_document",
        provider_service="google",
        capability_id="google-docs.create_document",
        arguments={"title": CREATE_TITLE},
        assert_baseline=_baseline,
        assert_outcome=_create_document_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_get_document",
        provider_service="google",
        capability_id="google-docs.get_document",
        arguments={"document_id": DOCUMENT_ID},
        assert_baseline=_baseline,
        assert_outcome=_get_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_get_document_empty",
        provider_service="google",
        capability_id="google-docs.get_document",
        arguments={"document_id": "doc_provider_contract_empty"},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "document_id": "",
                "title": "",
                "revision_id": "",
                "body_length": 1,
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/v1/documents/doc_provider_contract_empty",
            payload={},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="google_docs_read_content",
        provider_service="google",
        capability_id="google-docs.read_content",
        arguments={"document_id": DOCUMENT_ID},
        assert_baseline=_baseline,
        assert_outcome=_read_content_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_read_content_empty",
        provider_service="google",
        capability_id="google-docs.read_content",
        arguments={"document_id": "doc_provider_contract_empty"},
        assert_baseline=_baseline,
        assert_outcome=exact_text_output(""),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/v1/documents/doc_provider_contract_empty",
            payload={},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="google_docs_inspect_document",
        provider_service="google",
        capability_id="google-docs.inspect_document",
        arguments={"document_id": DOCUMENT_ID},
        assert_baseline=_baseline,
        assert_outcome=_inspect_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_inspect_document_empty",
        provider_service="google",
        capability_id="google-docs.inspect_document",
        arguments={"document_id": "doc_provider_contract_empty"},
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "document_id": "",
                "title": "",
                "revision_id": "",
                "body_length": 1,
                "elements": [],
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/v1/documents/doc_provider_contract_empty",
            payload={},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="google_docs_verify_document",
        provider_service="google",
        capability_id="google-docs.verify_document",
        arguments={
            "document_id": DOCUMENT_ID,
            "expected_text": ["user-owned agents"],
        },
        assert_baseline=_baseline,
        assert_outcome=_verify_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_verify_document_empty",
        provider_service="google",
        capability_id="google-docs.verify_document",
        arguments={
            "document_id": "doc_provider_contract_empty",
            "expected_text": ["missing"],
        },
        assert_baseline=_baseline,
        assert_outcome=exact_output(
            {
                "document_id": "",
                "revision_id": "",
                "verified": False,
                "checks": [
                    {
                        "expectation": 'document contains text "missing"',
                        "passed": False,
                    }
                ],
            }
        ),
        outcome_class="empty",
        setup_provider_proxy=static_provider_json_response(
            method="GET",
            path="/v1/documents/doc_provider_contract_empty",
            payload={},
        ),
        expect_provider_forward=False,
        expected_proxy_profile="provider_contract_empty",
    ),
    ProviderOperationCase(
        case_id="google_docs_apply_text_edits",
        provider_service="google",
        capability_id="google-docs.apply_text_edits",
        arguments={
            "document_id": DOCUMENT_ID,
            "edits": [
                {
                    "find": "user-owned agents",
                    "replace": SEMANTIC_REPLACEMENT,
                }
            ],
        },
        assert_baseline=_baseline,
        assert_outcome=_semantic_edit_outcome,
        expected_request_count=3,
    ),
    ProviderOperationCase(
        case_id="google_docs_create_table_with_data",
        provider_service="google",
        capability_id="google-docs.create_table_with_data",
        arguments={
            "document_id": DOCUMENT_ID,
            "index": 1,
            "table_data": TABLE_DATA,
            "bold_header": True,
        },
        assert_baseline=_baseline,
        assert_outcome=_semantic_table_outcome,
        expected_request_count=7,
    ),
    ProviderOperationCase(
        case_id="google_docs_insert_text",
        provider_service="google",
        capability_id="google-docs.insert_text",
        arguments={
            "document_id": DOCUMENT_ID,
            "text": INSERT_MARKER,
            "index": -1,
        },
        assert_baseline=_baseline,
        assert_outcome=_insert_text_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_batch_update",
        provider_service="google",
        capability_id="google-docs.batch_update",
        arguments={
            "document_id": DOCUMENT_ID,
            "requests": [
                {
                    "insertText": {
                        "endOfSegmentLocation": {},
                        "text": BATCH_MARKER,
                    }
                }
            ],
        },
        assert_baseline=_baseline,
        assert_outcome=_batch_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_delete_content",
        provider_service="google",
        capability_id="google-docs.delete_content",
        arguments={
            "document_id": DOCUMENT_ID,
            "start_index": 1,
            "end_index": 6,
        },
        assert_baseline=_baseline,
        assert_outcome=_delete_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_replace_text",
        provider_service="google",
        capability_id="google-docs.replace_text",
        arguments={
            "document_id": DOCUMENT_ID,
            "find": "user-owned agents",
            "replace": REPLACEMENT,
            "match_case": True,
        },
        assert_baseline=_baseline,
        assert_outcome=_replace_outcome,
    ),
    ProviderOperationCase(
        case_id="google_docs_format_text",
        provider_service="google",
        capability_id="google-docs.format_text",
        arguments={
            "document_id": DOCUMENT_ID,
            "start_index": 1,
            "end_index": 9,
            "bold": True,
            "foreground_color": "#3367D6",
        },
        assert_baseline=_baseline,
        assert_outcome=_revision_outcome(DOCUMENT_ID),
    ),
    ProviderOperationCase(
        case_id="google_docs_format_paragraph",
        provider_service="google",
        capability_id="google-docs.format_paragraph",
        arguments={
            "document_id": DOCUMENT_ID,
            "start_index": 1,
            "end_index": 9,
            "named_style": "HEADING_2",
            "alignment": "CENTER",
        },
        assert_baseline=_baseline,
        assert_outcome=_revision_outcome(DOCUMENT_ID),
    ),
    ProviderOperationCase(
        case_id="google_docs_insert_table",
        provider_service="google",
        capability_id="google-docs.insert_table",
        arguments={
            "document_id": DOCUMENT_ID,
            "rows": 2,
            "columns": 3,
            "index": 1,
        },
        assert_baseline=_baseline,
        assert_outcome=_revision_outcome(DOCUMENT_ID),
    ),
    ProviderOperationCase(
        case_id="google_docs_create_list",
        provider_service="google",
        capability_id="google-docs.create_list",
        arguments={
            "document_id": DOCUMENT_ID,
            "start_index": 1,
            "end_index": 9,
            "bullet_preset": "BULLET_DISC_CIRCLE_SQUARE",
        },
        assert_baseline=_baseline,
        assert_outcome=_revision_outcome(DOCUMENT_ID),
    ),
)
