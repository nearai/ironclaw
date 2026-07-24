"""Completeness gate for shipped first-party provider capabilities."""

import json
from pathlib import Path
import re
import tomllib

import pytest

from provider_capability_inventory import (
    ALL_CLASSIFIED_CAPABILITY_IDS,
    EMULATE_SUPPORTED_TOOLS,
    INTEGRATION_EVIDENCE,
    INTEGRATION_EVIDENCE_CAPABILITY_IDS,
    INVENTORY,
    TESTED_CAPABILITY_IDS,
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
    # Quarantined traces encode the retired activation flow; their fixtures
    # live under quarantined_retired_activation/ and are not replayable here.
    quarantined = set(manifest.get("quarantined_model_cases", []))
    evidence: dict[str, set[str]] = {}
    for case in manifest["selected_cases"]:
        if case in no_model_cases or case in quarantined:
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
    ] + [waiver["capabilities"] for waiver in INVENTORY.get("waivers", [])]
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

    for waiver in INVENTORY.get("waivers", []):
        for field in ("owner", "reason", "issue", "review_condition"):
            assert waiver.get(field), f"waiver is missing {field}: {waiver}"
        assert waiver["capabilities"], f"waiver has no capabilities: {waiver}"


def _cargo_test_targets() -> dict[str, str]:
    with (ROOT / "Cargo.toml").open("rb") as cargo_file:
        manifest = tomllib.load(cargo_file)
    return {
        target["name"]: target["path"]
        for target in manifest.get("test", [])
    }


def _assert_integration_evidence_is_executable(
    evidence: dict, targets: dict[str, str]
) -> None:
    required = {"capability", "target", "source", "test"}
    assert set(evidence) == required, (
        f"integration evidence fields must be exactly {sorted(required)}: "
        f"{evidence}"
    )

    assert evidence["target"] in targets, (
        f"unknown Cargo test target {evidence['target']!r}: {evidence}"
    )
    assert targets[evidence["target"]] == evidence["source"], (
        f"Cargo target {evidence['target']!r} points to "
        f"{targets[evidence['target']]!r}, not {evidence['source']!r}"
    )

    source = ROOT / evidence["source"]
    assert source.is_file(), f"integration evidence source is missing: {source}"
    _assert_executable_test_declaration(
        source.read_text(), evidence["test"], evidence["source"]
    )


def _assert_executable_test_declaration(
    source: str, test_name: str, source_label: str
) -> None:
    declaration = re.compile(
        rf"(?P<attributes>(?:^[ \t]*#\s*\[[^\n]+\][ \t]*\n)+)"
        rf"^[ \t]*(?:pub\s+)?(?:async\s+)?fn\s+{re.escape(test_name)}\s*\(",
        re.MULTILINE,
    ).search(source)
    assert declaration, (
        f"integration test {test_name!r} is missing from {source_label}"
    )

    attributes = set(
        re.findall(
            r"#\s*\[\s*([A-Za-z_][A-Za-z0-9_:]*)",
            declaration.group("attributes"),
        )
    )
    assert attributes & {"test", "tokio::test"}, (
        f"integration test {test_name!r} lacks a test attribute in "
        f"{source_label}"
    )
    disabling_attributes = sorted(attributes & {"cfg", "cfg_attr", "ignore"})
    assert not disabling_attributes, (
        f"integration test {test_name!r} is disabled by test-level attributes "
        f"{disabling_attributes} in {source_label}"
    )


@pytest.mark.parametrize(
    ("disabling_attribute", "expected_attribute"),
    [
        ("#[ignore]", "ignore"),
        ('#[cfg(feature = "disabled-evidence")]', "cfg"),
    ],
)
def test_executable_evidence_rejects_disabled_tests(
    disabling_attribute: str, expected_attribute: str
):
    source = (
        f"{disabling_attribute}\n"
        "#[tokio::test]\n"
        "async fn disabled_evidence() {}\n"
    )
    with pytest.raises(
        AssertionError,
        match=rf"disabled by test-level attributes .*{expected_attribute}",
    ):
        _assert_executable_test_declaration(
            source, "disabled_evidence", "synthetic.rs"
        )


def test_tested_capabilities_have_executable_evidence_at_the_correct_seam():
    """A tested label must point to executable evidence at the correct seam."""
    evidence = _recorded_tool_evidence()
    operation_case_tools = {
        capability_id_to_wire_name(case.capability_id)
        for case in PROVIDER_OPERATION_CASES
    }
    integration_capabilities = [
        entry["capability"] for entry in INTEGRATION_EVIDENCE
    ]
    duplicates = sorted(
        capability
        for capability in set(integration_capabilities)
        if integration_capabilities.count(capability) > 1
    )
    assert not duplicates, f"duplicate integration evidence: {duplicates}"
    assert INTEGRATION_EVIDENCE_CAPABILITY_IDS <= TESTED_CAPABILITY_IDS, (
        "integration evidence for untested capabilities: "
        f"{sorted(INTEGRATION_EVIDENCE_CAPABILITY_IDS - TESTED_CAPABILITY_IDS)}"
    )
    cargo_targets = _cargo_test_targets()
    for integration_evidence in INTEGRATION_EVIDENCE:
        _assert_integration_evidence_is_executable(
            integration_evidence, cargo_targets
        )

    missing_tested = sorted(
        EMULATE_SUPPORTED_TOOLS - evidence.keys() - operation_case_tools
    )
    assert not missing_tested, (
        f"Emulate-backed capabilities lack executable evidence: {missing_tested}"
    )
    assert operation_case_tools <= EMULATE_SUPPORTED_TOOLS
