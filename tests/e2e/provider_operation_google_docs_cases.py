"""Google Docs full-path provider operation cases."""

import json

from provider_operation_google_common import google_json
from provider_operation_types import ProviderOperationCase

DOCUMENT_ID = "doc_reborn_strategy"
SEEDED_TEXT = (
    "NEAR AI Strategy: user-owned agents keep credentials and data under user control."
)
BATCH_MARKER = " REBORN_PROVIDER_CASE_BATCH"
REPLACEMENT = "customer-owned agents"


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


async def _baseline(emulate_url: str) -> None:
    document = await _document(emulate_url)
    assert document["revisionId"] == "1", document
    assert _document_text(document) == SEEDED_TEXT, document


async def _get_outcome(emulate_url: str, preview: dict) -> None:
    await _baseline(emulate_url)
    rendered = json.dumps(preview)
    assert "NEAR AI Strategy" in rendered, preview
    assert DOCUMENT_ID in rendered, preview


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
        case_id="google_docs_get_document",
        provider_service="google",
        capability_id="google-docs.get_document",
        arguments={"document_id": DOCUMENT_ID},
        assert_baseline=_baseline,
        assert_outcome=_get_outcome,
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
