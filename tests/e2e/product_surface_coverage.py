#!/usr/bin/env python3
"""Generate the product-surface coverage matrix from executable inventories."""

from __future__ import annotations

import argparse
import json
import subprocess
from collections import defaultdict
from collections.abc import Iterable, Mapping
from pathlib import Path

from journey_cases import ALL_JOURNEY_CASES
from journey_types import ProductJourneyCase, ProviderJourneyCase
from provider_capability_inventory import (
    CAPABILITY_OPERATION_KINDS,
    INTEGRATION_EVIDENCE,
    INVENTORY,
    JOURNEY_EVIDENCE,
    capability_id_to_wire_name,
)
from provider_fault_cases import PROVIDER_FAULT_CASES
from provider_operation_cases import PROVIDER_OPERATION_CASES

COVERAGE_AXES = ("contract", "journey", "faults", "browser", "live")
ROOT = Path(__file__).resolve().parents[2]
FULL_PATH_SOURCE = "tests/e2e/scenarios/test_reborn_qa_trace_full_path.py"


def _classification_by_capability() -> dict[str, str]:
    classifications: dict[str, str] = {}
    for classification in ("tested", "live_only", "unsupported"):
        for capability_id in INVENTORY["classifications"][classification]:
            classifications[capability_id] = classification
    for waiver in INVENTORY.get("waivers", []):
        for capability_id in waiver["capabilities"]:
            classifications[capability_id] = "waived"
    return classifications


def _trace_capabilities(case: ProviderJourneyCase) -> set[str]:
    trace = json.loads((ROOT / case.trace).read_text(encoding="utf-8"))
    wire_names = {
        call["name"]
        for step in trace["steps"]
        for call in step["response"].get("tool_calls", [])
    }
    return {
        capability_id
        for capability_id in CAPABILITY_OPERATION_KINDS
        if capability_id_to_wire_name(capability_id) in wire_names
    }


def _evidence(**fields: str) -> dict[str, str]:
    return {field: value for field, value in fields.items() if value}


def build_capability_evidence() -> dict[str, dict[str, list[dict[str, str]]]]:
    evidence = {axis: defaultdict(list) for axis in COVERAGE_AXES}

    for case in PROVIDER_OPERATION_CASES:
        evidence["contract"][case.capability_id].append(
            _evidence(
                tier="provider_contract",
                source=FULL_PATH_SOURCE,
                test="test_provider_operation_case_executes_with_provider_readback",
                case_id=case.case_id,
            )
        )
    for item in INTEGRATION_EVIDENCE:
        evidence["contract"][item["capability"]].append(
            _evidence(
                tier="integration",
                source=item["source"],
                test=item["test"],
                target=item["target"],
            )
        )
    for item in JOURNEY_EVIDENCE:
        reference = _evidence(
            tier="journey_readback",
            source=item["source"],
            test=item["test"],
            assertion=item["assertion"],
        )
        evidence["contract"][item["capability"]].append(reference)
        evidence["journey"][item["capability"]].append(reference)

    for fault in PROVIDER_FAULT_CASES:
        evidence["faults"][fault.operation.capability_id].append(
            _evidence(
                tier="representative_fault",
                source=FULL_PATH_SOURCE,
                test="test_provider_fault_profile_preserves_safe_operation_outcomes",
                case_id=fault.case_id,
            )
        )

    for case in ALL_JOURNEY_CASES:
        reference = _evidence(
            tier="journey",
            source=case.evidence.source,
            test=case.evidence.test,
            case_id=case.case_id,
        )
        capabilities: set[str] = set()
        if isinstance(case, ProviderJourneyCase):
            capabilities = _trace_capabilities(case)
        for capability_id in capabilities:
            evidence["journey"][capability_id].append(reference)
            if isinstance(case, ProviderJourneyCase):
                evidence["live"][capability_id].append(
                    _evidence(
                        tier="scheduled_live",
                        source=case.live_evidence.workflow,
                        test=case.live_evidence.job,
                        case_id=case.live_evidence.case_id,
                        artifact=case.live_evidence.artifact,
                    )
                )

    return {
        axis: {
            capability_id: [
                json.loads(reference)
                for reference in sorted(
                    {json.dumps(item, sort_keys=True) for item in references}
                )
            ]
            for capability_id, references in by_capability.items()
        }
        for axis, by_capability in evidence.items()
    }


def _owned_gaps() -> dict[str, list[dict[str, str]]]:
    gaps: dict[str, list[dict[str, str]]] = defaultdict(list)
    for item in INVENTORY.get("coverage_backlog", []):
        metadata = {
            field: str(item[field])
            for field in ("rule", "owner", "reason", "issue", "review_condition")
        }
        for capability_id in item["capabilities"]:
            gaps[capability_id].append(metadata)
    return gaps


def _waivers() -> dict[str, list[dict[str, str]]]:
    waivers: dict[str, list[dict[str, str]]] = defaultdict(list)
    for item in INVENTORY.get("waivers", []):
        metadata = {
            field: str(item[field])
            for field in ("owner", "reason", "issue", "review_condition")
        }
        for capability_id in item["capabilities"]:
            waivers[capability_id].append(metadata)
    return waivers


def _journey_row(case: ProviderJourneyCase | ProductJourneyCase) -> dict:
    reference = _evidence(
        tier="journey",
        source=case.evidence.source,
        test=case.evidence.test,
        case_id=case.case_id,
    )
    evidence = {
        axis: {
            "status": "covered" if axis == "journey" else "not_applicable",
            "items": [reference] if axis == "journey" else [],
        }
        for axis in COVERAGE_AXES
    }
    if isinstance(case, ProductJourneyCase) and case.browser_evidence is not None:
        evidence["browser"] = {
            "status": "covered",
            "items": [
                _evidence(
                    tier="browser",
                    source=case.browser_evidence.source,
                    test=case.browser_evidence.test,
                    case_id=case.case_id,
                )
            ],
        }
    if isinstance(case, ProviderJourneyCase):
        evidence["live"] = {
            "status": "scheduled",
            "items": [
                _evidence(
                    tier="scheduled_live",
                    source=case.live_evidence.workflow,
                    test=case.live_evidence.job,
                    case_id=case.live_evidence.case_id,
                    artifact=case.live_evidence.artifact,
                )
            ],
        }
    return {
        "id": case.case_id,
        "kind": "journey",
        "classification": "tested",
        "evidence": evidence,
        "owned_gaps": [],
        "waivers": [],
    }


def build_report(
    *,
    production_capabilities: Mapping[str, str] | None = None,
    capability_evidence: Mapping[str, Mapping[str, list[dict[str, str]]]] | None = None,
    journey_cases: Iterable[ProviderJourneyCase | ProductJourneyCase] | None = None,
) -> dict:
    """Build a deterministic report; callers may inject a sabotage denominator."""
    production = dict(
        CAPABILITY_OPERATION_KINDS
        if production_capabilities is None
        else production_capabilities
    )
    journeys = tuple(ALL_JOURNEY_CASES if journey_cases is None else journey_cases)
    if not production:
        raise ValueError("production capability denominator is empty")
    if not journeys:
        raise ValueError("typed journey denominator is empty")
    classifications = _classification_by_capability()
    evidence_maps = dict(
        build_capability_evidence()
        if capability_evidence is None
        else capability_evidence
    )
    owned_gaps = _owned_gaps()
    waivers = _waivers()
    missing_rows = []
    capability_rows = []

    for capability_id in sorted(production):
        classification = classifications.get(capability_id)
        raw_evidence = {
            axis: evidence_maps.get(axis, {}).get(capability_id, [])
            for axis in COVERAGE_AXES
        }
        if classification is None:
            missing_rows.append(
                {
                    "id": capability_id,
                    "kind": "capability",
                    "reason": "production capability has no owned classification",
                }
            )
            classification = "missing"
        contract_status = "covered" if raw_evidence["contract"] else "missing"
        if classification == "unsupported":
            contract_status = "not_applicable"
        elif classification == "live_only":
            contract_status = "live_only"
        elif classification == "waived" or owned_gaps.get(capability_id):
            contract_status = "covered" if raw_evidence["contract"] else "waived"
        if classification == "tested" and contract_status == "missing":
            missing_rows.append(
                {
                    "id": capability_id,
                    "kind": "capability",
                    "reason": "tested capability has no contract evidence or owned gap",
                }
            )
        evidence = {}
        for axis in COVERAGE_AXES:
            status = "covered" if raw_evidence[axis] else "not_applicable"
            if axis == "contract":
                status = contract_status
            elif axis == "faults" and raw_evidence[axis]:
                status = "representative"
            elif axis == "live" and raw_evidence[axis]:
                status = "scheduled"
            evidence[axis] = {"status": status, "items": raw_evidence[axis]}
        capability_rows.append(
            {
                "id": capability_id,
                "kind": "capability",
                "operation_kind": production[capability_id],
                "classification": classification,
                "evidence": evidence,
                "owned_gaps": owned_gaps.get(capability_id, []),
                "waivers": waivers.get(capability_id, []),
            }
        )

    journey_rows = [_journey_row(case) for case in journeys]
    surfaces = [*capability_rows, *journey_rows]
    owned_gap_rows = [
        {"id": row["id"], "kind": row["kind"], "gaps": row["owned_gaps"]}
        for row in surfaces
        if row["owned_gaps"]
    ]
    live_only_rows = [
        {"id": row["id"], "kind": row["kind"]}
        for row in surfaces
        if row["classification"] == "live_only"
    ]
    waiver_rows = [
        {"id": row["id"], "kind": row["kind"], "waivers": row["waivers"]}
        for row in surfaces
        if row["waivers"]
    ]
    return {
        "schema_version": 1,
        "axes": list(COVERAGE_AXES),
        "denominators": {
            "capabilities": (
                "crates/ironclaw_first_party_extensions/assets/*/manifest.toml"
            ),
            "journeys": "tests/e2e/journey_cases.py::ALL_JOURNEY_CASES",
            "faults": "tests/e2e/provider_fault_cases.py::PROVIDER_FAULT_CASES",
            "classifications": ("tests/e2e/fixtures/provider_capability_coverage.toml"),
        },
        "summary": {
            "capabilities": len(capability_rows),
            "journeys": len(journey_rows),
            "classified_capabilities": sum(
                row["classification"] != "missing" for row in capability_rows
            ),
            "missing": len(missing_rows),
            "owned_gaps": len(owned_gap_rows),
            "waivers": len(waiver_rows),
            "live_only": len(live_only_rows),
            "evidence_rows": {
                axis: sum(bool(row["evidence"][axis]["items"]) for row in surfaces)
                for axis in COVERAGE_AXES
            },
        },
        "surfaces": surfaces,
        "missing_rows": missing_rows,
        "owned_gap_rows": owned_gap_rows,
        "waiver_rows": waiver_rows,
        "live_only_rows": live_only_rows,
    }


def _evidence_marker(cell: Mapping[str, object]) -> str:
    items = list(cell["items"])
    status = str(cell["status"])
    return f"{status} ({len(items)})" if items else status


def render_markdown(report: dict) -> str:
    summary = report["summary"]
    lines = [
        "# Product-surface coverage",
        "",
        f"Classified capabilities: {summary['classified_capabilities']}",
        f"Typed journeys: {summary['journeys']}",
        f"Missing rows: {summary['missing']}",
        f"Owned gaps: {summary['owned_gaps']}",
        f"Waivers: {summary['waivers']}",
        f"Live-only rows: {summary['live_only']}",
        "Evidence-bearing rows: "
        + ", ".join(
            f"{axis}={summary['evidence_rows'][axis]}" for axis in COVERAGE_AXES
        ),
        "",
        "| Surface | Kind | Classification | Contract | Journey | Faults | Browser | Live |",
        "|---|---|---|---:|---:|---:|---:|---:|",
    ]
    for row in report["surfaces"]:
        markers = [_evidence_marker(row["evidence"][axis]) for axis in COVERAGE_AXES]
        lines.append(
            f"| `{row['id']}` | {row['kind']} | {row['classification']} | "
            + " | ".join(markers)
            + " |"
        )

    lines.extend(["", "## Missing", ""])
    if report["missing_rows"]:
        lines.extend(
            f"- `{row['id']}` ({row['kind']}): {row['reason']}"
            for row in report["missing_rows"]
        )
    else:
        lines.append("- None.")

    lines.extend(["", "## Owned gaps", ""])
    if report["owned_gap_rows"]:
        for row in report["owned_gap_rows"]:
            for gap in row["gaps"]:
                lines.append(
                    f"- `{row['id']}` — {gap['rule']} — {gap['owner']} — "
                    f"{gap['issue']}: {gap['reason']} "
                    f"(review: {gap['review_condition']})"
                )
    else:
        lines.append("- None.")

    lines.extend(["", "## Waivers", ""])
    if report["waiver_rows"]:
        for row in report["waiver_rows"]:
            for waiver in row["waivers"]:
                lines.append(
                    f"- `{row['id']}` — {waiver['owner']} — {waiver['issue']}: "
                    f"{waiver['reason']} (review: {waiver['review_condition']})"
                )
    else:
        lines.append("- None.")

    lines.extend(["", "## Live-only", ""])
    if report["live_only_rows"]:
        lines.extend(f"- `{row['id']}`" for row in report["live_only_rows"])
    else:
        lines.append("- None.")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    args = parser.parse_args()

    source_commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    report = {"source_commit": source_commit, **build_report()}
    for path in (args.json, args.markdown):
        path.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    args.markdown.write_text(render_markdown(report), encoding="utf-8")
    print(
        "product-surface coverage: "
        f"{report['summary']['capabilities']} capabilities, "
        f"{report['summary']['journeys']} journeys, "
        f"{report['summary']['missing']} missing"
    )
    return 1 if report["missing_rows"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
