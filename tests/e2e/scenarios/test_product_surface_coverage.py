"""Contract tests for the generated product-surface coverage artifact."""

from pathlib import Path

import pytest

from journey_cases import ALL_JOURNEY_CASES
from journey_types import ProviderJourneyCase
from product_surface_coverage import (
    COVERAGE_AXES,
    build_capability_evidence,
    build_report,
    render_markdown,
)
from provider_capability_inventory import (
    ALL_CLASSIFIED_CAPABILITY_IDS,
    CAPABILITY_OPERATION_KINDS,
)

ROOT = Path(__file__).resolve().parents[3]


def test_report_uses_complete_production_and_typed_journey_denominators():
    report = build_report()

    assert report["summary"]["capabilities"] == len(CAPABILITY_OPERATION_KINDS)
    assert report["summary"]["journeys"] == len(ALL_JOURNEY_CASES)
    assert report["summary"]["missing"] == 0
    assert report["summary"]["owned_gaps"] == 0
    assert {
        row["id"] for row in report["surfaces"] if row["kind"] == "capability"
    } == set(CAPABILITY_OPERATION_KINDS)
    assert {row["id"] for row in report["surfaces"] if row["kind"] == "journey"} == {
        case.case_id for case in ALL_JOURNEY_CASES
    }
    assert all(set(row["evidence"]) == set(COVERAGE_AXES) for row in report["surfaces"])


def test_unclassified_shipped_capability_fails_loudly():
    sabotaged_production_ids = {
        **CAPABILITY_OPERATION_KINDS,
        "sabotage.missing_classification": "read",
    }

    report = build_report(production_capabilities=sabotaged_production_ids)

    assert report["summary"]["missing"] == 1
    assert report["missing_rows"] == [
        {
            "id": "sabotage.missing_classification",
            "kind": "capability",
            "reason": "production capability has no owned classification",
        }
    ]


def test_empty_denominators_fail_instead_of_passing_vacuously():
    with pytest.raises(ValueError, match="capability denominator is empty"):
        build_report(production_capabilities={})
    with pytest.raises(ValueError, match="journey denominator is empty"):
        build_report(journey_cases=())


def test_losing_required_contract_evidence_is_not_hidden_by_journey_evidence():
    evidence = build_capability_evidence()
    capability_id = "gmail.send_message"
    assert evidence["journey"][capability_id]
    evidence["contract"][capability_id] = []

    report = build_report(capability_evidence=evidence)

    assert report["summary"]["missing"] == 1
    assert report["missing_rows"] == [
        {
            "id": capability_id,
            "kind": "capability",
            "reason": "tested capability has no contract evidence or owned gap",
        }
    ]


def test_live_and_browser_axes_require_typed_executable_evidence():
    report = build_report()

    provider_journeys = [
        case for case in ALL_JOURNEY_CASES if isinstance(case, ProviderJourneyCase)
    ]
    assert report["summary"]["evidence_rows"]["live"] >= len(provider_journeys)
    assert report["summary"]["evidence_rows"]["browser"] == 1
    browser_rows = [
        row
        for row in report["surfaces"]
        if row["evidence"]["browser"]["status"] == "covered"
    ]
    assert [row["id"] for row in browser_rows] == ["webui_text_turn_persists"]
    assert browser_rows[0]["evidence"]["browser"]["items"][0]["test"] == (
        "test_reborn_v2_ui_enter_submits_initial_and_follow_up_messages"
    )
    live_journey_rows = [
        row
        for row in report["surfaces"]
        if row["kind"] == "journey" and row["evidence"]["live"]["status"] == "scheduled"
    ]
    assert {row["id"] for row in live_journey_rows} == {
        case.case_id for case in provider_journeys
    }
    assert all(
        item["source"] == ".github/workflows/live-canary.yml"
        and item["test"] == "reborn-webui-v2-live-qa"
        and item["artifact"] == "results.json"
        for row in live_journey_rows
        for item in row["evidence"]["live"]["items"]
    )
    live_workflow = (ROOT / ".github/workflows/live-canary.yml").read_text(
        encoding="utf-8"
    )
    for case in provider_journeys:
        assert (
            f"cases: {case.case_id}" in live_workflow
            or f",{case.case_id}" in live_workflow
        )


def test_owned_gaps_and_live_only_rows_are_prominent_in_markdown():
    report = build_report()
    markdown = render_markdown(report)

    assert "## Owned gaps" in markdown
    assert "## Live-only" in markdown
    assert f"Classified capabilities: {len(ALL_CLASSIFIED_CAPABILITY_IDS)}" in markdown
    for gap in report["owned_gap_rows"]:
        assert gap["id"] in markdown


def test_reborn_e2e_generates_and_uploads_the_product_surface_artifact():
    workflow = (ROOT / ".github/workflows/reborn-e2e.yml").read_text(encoding="utf-8")

    evidence_contract_step = workflow.index(
        "Validate product-surface evidence contracts"
    )
    generation_step = workflow.index("Generate product-surface coverage matrix")
    assert evidence_contract_step < generation_step
    for test_path in (
        "tests/e2e/scenarios/test_product_surface_coverage.py",
        "tests/e2e/scenarios/test_provider_capability_inventory.py",
        "tests/e2e/scenarios/test_journey_coverage.py",
    ):
        assert test_path in workflow[evidence_contract_step:generation_step]
    assert "product_surface_coverage.py" in workflow
    assert "product-surface-coverage-${{" in workflow
    assert "artifacts/product-surface-coverage/" in workflow
    assert "$GITHUB_STEP_SUMMARY" in workflow
    assert 'source_commit="$(git rev-parse HEAD)"' in workflow
    assert generation_step < workflow.index(
        "Run WebUI, provider, and Responses API suites"
    )
    upload_step = workflow[workflow.index("Upload product-surface coverage matrix") :]
    assert "if: always()" in upload_step.split("      - name:", 1)[0]
    assert "if-no-files-found: error" in upload_step.split("      - name:", 1)[0]
