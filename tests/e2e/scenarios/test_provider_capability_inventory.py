"""Completeness gate for shipped first-party provider capabilities."""

import json
from pathlib import Path
import tomllib

from provider_capability_inventory import (
    ALL_CLASSIFIED_CAPABILITY_IDS,
    EMULATE_SUPPORTED_TOOLS,
    INVENTORY,
    capability_id_to_wire_name,
)
from provider_operation_cases import PROVIDER_OPERATION_CASES

ROOT = Path(__file__).resolve().parents[3]
ASSET_ROOT = ROOT / "crates/ironclaw_first_party_extensions/assets"
TRACE_ROOT = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"


def _production_capability_ids() -> set[str]:
    capability_ids = set()
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        capability_ids.update(tool["id"] for tool in manifest.get("tools", []))
    return capability_ids


def _recorded_tool_evidence() -> dict[str, set[str]]:
    manifest = json.loads((TRACE_ROOT / "case-manifest.json").read_text())
    no_model_cases = set(manifest["no_model_cases"])
    evidence: dict[str, set[str]] = {}
    for case in manifest["selected_cases"]:
        if case in no_model_cases:
            continue
        trace = json.loads((TRACE_ROOT / f"{case}.json").read_text())
        for step in trace["steps"]:
            for call in step["response"].get("tool_calls", []):
                evidence.setdefault(call["name"], set()).add(case)
    return evidence


def test_every_shipped_provider_capability_has_an_owned_classification():
    """A manifest change cannot silently expand the untested product surface."""
    assert INVENTORY["schema_version"] == 1

    classified_lists = [
        INVENTORY["classifications"][classification]
        for classification in ("tested", "live_only", "unsupported")
    ] + [waiver["capabilities"] for waiver in INVENTORY["waivers"]]
    flattened = [capability for group in classified_lists for capability in group]
    duplicates = sorted(
        capability for capability in set(flattened) if flattened.count(capability) > 1
    )
    assert not duplicates, f"capabilities have multiple classifications: {duplicates}"

    production = _production_capability_ids()
    assert ALL_CLASSIFIED_CAPABILITY_IDS == production, (
        f"missing={sorted(production - ALL_CLASSIFIED_CAPABILITY_IDS)}, "
        f"stale={sorted(ALL_CLASSIFIED_CAPABILITY_IDS - production)}"
    )

    for waiver in INVENTORY["waivers"]:
        for field in ("owner", "reason", "issue", "review_condition"):
            assert waiver.get(field), f"waiver is missing {field}: {waiver}"
        assert waiver["capabilities"], f"waiver has no capabilities: {waiver}"


def test_tested_capabilities_have_full_path_evidence():
    """A tested label must point to a harvested journey or typed operation case."""
    evidence = _recorded_tool_evidence()
    operation_case_tools = {
        capability_id_to_wire_name(case.capability_id)
        for case in PROVIDER_OPERATION_CASES
    }
    missing_tested = sorted(
        EMULATE_SUPPORTED_TOOLS - evidence.keys() - operation_case_tools
    )
    assert not missing_tested, (
        f"tested capabilities lack full-path evidence: {missing_tested}"
    )
    assert operation_case_tools <= EMULATE_SUPPORTED_TOOLS
