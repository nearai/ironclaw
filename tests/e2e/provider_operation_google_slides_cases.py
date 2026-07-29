"""Google Slides full-path provider operation cases."""

import json

from emulate_provider import google_json
from provider_operation_types import ProviderOperationCase

PRESENTATION_ID = "slides_reborn_launch"
PRESENTATION_TITLE = "Reborn Launch Review"
SLIDE_ID = "slide_main"
BODY_ID = "shape_body"
DELETE_ID = "shape_delete"
REPLACE_ID = "shape_replace"
SEEDED_TEXT = "Launch Review PLACEHOLDER DELETE"
CREATED_TITLE = "REBORN_PROVIDER_CASE_CREATED_PRESENTATION"
INSERTED_TEXT = "READY: "
REPLACEMENT_TEXT = "APPROVED"
IMAGE_URL = "https://example.com/provider-contract.png"
BATCH_TEXT = "BATCH "


async def _presentation(
    emulate_url: str, presentation_id: str = PRESENTATION_ID
) -> dict:
    result = await google_json(
        emulate_url, "GET", f"/v1/presentations/{presentation_id}"
    )
    assert isinstance(result, dict)
    return result


def _elements(presentation: dict) -> dict[str, dict]:
    return {
        element["objectId"]: element
        for slide in presentation["slides"]
        for element in slide["pageElements"]
    }


def _shape_text(element: dict) -> str:
    return "".join(
        item.get("textRun", {}).get("content", "")
        for item in element["shape"]["text"]["textElements"]
    )


def _output(preview: dict) -> dict:
    result = json.loads(preview["output_preview"])
    assert isinstance(result, dict), preview
    return result


async def _baseline(emulate_url: str) -> None:
    presentation = await _presentation(emulate_url)
    assert presentation["title"] == PRESENTATION_TITLE, presentation
    assert presentation["revisionId"] == "1", presentation
    assert [slide["objectId"] for slide in presentation["slides"]] == [
        SLIDE_ID
    ], presentation
    elements = _elements(presentation)
    assert set(elements) == {BODY_ID, DELETE_ID, REPLACE_ID}, elements
    assert _shape_text(elements[BODY_ID]) == SEEDED_TEXT, elements[BODY_ID]


async def _create_presentation_outcome(
    emulate_url: str, preview: dict
) -> None:
    output = _output(preview)
    created = await _presentation(emulate_url, output["presentation_id"])
    assert created["title"] == CREATED_TITLE, created
    assert len(created["slides"]) == 1, created
    assert output["title"] == CREATED_TITLE, output


async def _get_presentation_outcome(
    emulate_url: str, preview: dict
) -> None:
    await _baseline(emulate_url)
    output = _output(preview)
    assert output["presentation_id"] == PRESENTATION_ID, output
    assert output["revision_id"] == "1", output
    assert output["slide_count"] == 1, output
    assert SEEDED_TEXT in json.dumps(output), output


async def _thumbnail_outcome(emulate_url: str, preview: dict) -> None:
    thumbnail = await google_json(
        emulate_url,
        "GET",
        f"/v1/presentations/{PRESENTATION_ID}/pages/{SLIDE_ID}/thumbnail",
    )
    assert isinstance(thumbnail, dict)
    assert (thumbnail["width"], thumbnail["height"]) == (1600, 900), thumbnail
    output = _output(preview)
    assert output["content_url"] == thumbnail["contentUrl"], output
    assert (output["width"], output["height"]) == (1600, 900), output


async def _create_slide_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    presentation = await _presentation(emulate_url)
    assert presentation["revisionId"] == "2", presentation
    assert [slide["objectId"] for slide in presentation["slides"]] == [
        SLIDE_ID,
        output["created_object_id"],
    ], presentation


async def _delete_object_outcome(emulate_url: str, preview: dict) -> None:
    presentation = await _presentation(emulate_url)
    assert presentation["revisionId"] == "2", presentation
    assert DELETE_ID not in _elements(presentation), presentation
    assert _output(preview)["presentation_id"] == PRESENTATION_ID, preview


async def _insert_text_outcome(emulate_url: str, preview: dict) -> None:
    presentation = await _presentation(emulate_url)
    assert _shape_text(_elements(presentation)[BODY_ID]) == (
        INSERTED_TEXT + SEEDED_TEXT
    ), presentation
    assert presentation["revisionId"] == "2", presentation
    assert _output(preview)["presentation_id"] == PRESENTATION_ID, preview


async def _delete_text_outcome(emulate_url: str, preview: dict) -> None:
    presentation = await _presentation(emulate_url)
    assert _shape_text(_elements(presentation)[BODY_ID]) == SEEDED_TEXT[7:]
    assert presentation["revisionId"] == "2", presentation
    assert _output(preview)["presentation_id"] == PRESENTATION_ID, preview


async def _replace_text_outcome(emulate_url: str, preview: dict) -> None:
    presentation = await _presentation(emulate_url)
    text = _shape_text(_elements(presentation)[BODY_ID])
    assert text == SEEDED_TEXT.replace("PLACEHOLDER", REPLACEMENT_TEXT), text
    assert _output(preview)["occurrences_changed"] == 2, preview


async def _create_shape_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    element = _elements(await _presentation(emulate_url))[
        output["created_object_id"]
    ]
    assert element["shape"]["shapeType"] == "RECTANGLE", element
    assert element["size"]["width"]["magnitude"] == 1524000, element


async def _insert_image_outcome(emulate_url: str, preview: dict) -> None:
    output = _output(preview)
    element = _elements(await _presentation(emulate_url))[
        output["created_object_id"]
    ]
    assert element["image"]["contentUrl"] == IMAGE_URL, element
    assert element["size"]["width"]["magnitude"] == 1270000, element


async def _format_text_outcome(emulate_url: str, preview: dict) -> None:
    element = _elements(await _presentation(emulate_url))[BODY_ID]
    styled_runs = [
        item["textRun"]["style"]
        for item in element["shape"]["text"]["textElements"]
        if "textRun" in item and item["textRun"]["style"]
    ]
    assert styled_runs == [
        {
            "bold": True,
            "fontSize": {"magnitude": 24, "unit": "PT"},
            "fontFamily": "Inter",
        }
    ], styled_runs
    assert _output(preview)["presentation_id"] == PRESENTATION_ID, preview


async def _format_paragraph_outcome(
    emulate_url: str, preview: dict
) -> None:
    element = _elements(await _presentation(emulate_url))[BODY_ID]
    styles = [
        item["paragraphMarker"]["style"]
        for item in element["shape"]["text"]["textElements"]
        if item.get("paragraphMarker", {}).get("style")
    ]
    assert styles == [{"alignment": "CENTER"}], styles
    assert _output(preview)["presentation_id"] == PRESENTATION_ID, preview


async def _replace_shape_outcome(emulate_url: str, preview: dict) -> None:
    element = _elements(await _presentation(emulate_url))[REPLACE_ID]
    assert element["image"]["contentUrl"] == IMAGE_URL, element
    assert _output(preview)["occurrences_changed"] == 1, preview


async def _batch_outcome(emulate_url: str, preview: dict) -> None:
    presentation = await _presentation(emulate_url)
    element = _elements(presentation)[BODY_ID]
    assert _shape_text(element) == BATCH_TEXT + SEEDED_TEXT, element
    styles = [
        item["textRun"]["style"]
        for item in element["shape"]["text"]["textElements"]
        if "textRun" in item and item["textRun"]["style"]
    ]
    assert {"bold": True} in styles, styles
    assert presentation["revisionId"] == "2", presentation
    output = _output(preview)
    assert output["presentation_id"] == PRESENTATION_ID, output
    assert len(output["replies"]) == 2, output


GOOGLE_SLIDES_PROVIDER_OPERATION_CASES = (
    ProviderOperationCase(
        case_id="google_slides_create_presentation",
        provider_service="google",
        capability_id="google-slides.create_presentation",
        arguments={"title": CREATED_TITLE},
        assert_baseline=_baseline,
        assert_outcome=_create_presentation_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_get_presentation",
        provider_service="google",
        capability_id="google-slides.get_presentation",
        arguments={"presentation_id": PRESENTATION_ID},
        assert_baseline=_baseline,
        assert_outcome=_get_presentation_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_get_thumbnail",
        provider_service="google",
        capability_id="google-slides.get_thumbnail",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "slide_object_id": SLIDE_ID,
        },
        assert_baseline=_baseline,
        assert_outcome=_thumbnail_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_create_slide",
        provider_service="google",
        capability_id="google-slides.create_slide",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "insertion_index": 1,
            "layout": "BLANK",
        },
        assert_baseline=_baseline,
        assert_outcome=_create_slide_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_delete_object",
        provider_service="google",
        capability_id="google-slides.delete_object",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "object_id": DELETE_ID,
        },
        assert_baseline=_baseline,
        assert_outcome=_delete_object_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_insert_text",
        provider_service="google",
        capability_id="google-slides.insert_text",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "object_id": BODY_ID,
            "text": INSERTED_TEXT,
            "insertion_index": 0,
        },
        assert_baseline=_baseline,
        assert_outcome=_insert_text_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_delete_text",
        provider_service="google",
        capability_id="google-slides.delete_text",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "object_id": BODY_ID,
            "start_index": 0,
            "end_index": 7,
        },
        assert_baseline=_baseline,
        assert_outcome=_delete_text_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_replace_all_text",
        provider_service="google",
        capability_id="google-slides.replace_all_text",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "find": "PLACEHOLDER",
            "replace": REPLACEMENT_TEXT,
            "match_case": True,
        },
        assert_baseline=_baseline,
        assert_outcome=_replace_text_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_create_shape",
        provider_service="google",
        capability_id="google-slides.create_shape",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "slide_object_id": SLIDE_ID,
            "shape_type": "RECTANGLE",
            "x": 10,
            "y": 20,
            "width": 120,
            "height": 40,
        },
        assert_baseline=_baseline,
        assert_outcome=_create_shape_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_insert_image",
        provider_service="google",
        capability_id="google-slides.insert_image",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "slide_object_id": SLIDE_ID,
            "image_url": IMAGE_URL,
            "x": 5,
            "y": 10,
            "width": 100,
            "height": 50,
        },
        assert_baseline=_baseline,
        assert_outcome=_insert_image_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_format_text",
        provider_service="google",
        capability_id="google-slides.format_text",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "object_id": BODY_ID,
            "start_index": 0,
            "end_index": 6,
            "bold": True,
            "font_size": 24,
            "font_family": "Inter",
        },
        assert_baseline=_baseline,
        assert_outcome=_format_text_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_format_paragraph",
        provider_service="google",
        capability_id="google-slides.format_paragraph",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "object_id": BODY_ID,
            "alignment": "CENTER",
            "start_index": 0,
            "end_index": len(SEEDED_TEXT),
        },
        assert_baseline=_baseline,
        assert_outcome=_format_paragraph_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_replace_shapes_with_image",
        provider_service="google",
        capability_id="google-slides.replace_shapes_with_image",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "find": "IMAGE_PLACEHOLDER",
            "image_url": IMAGE_URL,
            "match_case": True,
        },
        assert_baseline=_baseline,
        assert_outcome=_replace_shape_outcome,
    ),
    ProviderOperationCase(
        case_id="google_slides_batch_update",
        provider_service="google",
        capability_id="google-slides.batch_update",
        arguments={
            "presentation_id": PRESENTATION_ID,
            "requests": [
                {
                    "insertText": {
                        "objectId": BODY_ID,
                        "text": BATCH_TEXT,
                        "insertionIndex": 0,
                    }
                },
                {
                    "updateTextStyle": {
                        "objectId": BODY_ID,
                        "textRange": {
                            "type": "FIXED_RANGE",
                            "startIndex": 0,
                            "endIndex": len(BATCH_TEXT),
                        },
                        "style": {"bold": True},
                        "fields": "bold",
                    }
                },
            ],
        },
        assert_baseline=_baseline,
        assert_outcome=_batch_outcome,
    ),
)
