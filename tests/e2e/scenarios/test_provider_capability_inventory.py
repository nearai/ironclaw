"""Completeness gate for shipped first-party provider capabilities."""

import json
from pathlib import Path
import re
import tomllib

import pytest

from provider_capability_inventory import (
    ALL_CLASSIFIED_CAPABILITY_IDS,
    COVERAGE_BACKLOG,
    EMULATE_SUPPORTED_TOOLS,
    INTEGRATION_EVIDENCE,
    INTEGRATION_EVIDENCE_CAPABILITY_IDS,
    INVENTORY,
    JOURNEY_EVIDENCE,
    JOURNEY_EVIDENCE_CAPABILITY_IDS,
    READ_CAPABILITY_IDS,
    REQUIRED_READ_OUTCOME_CLASSES,
    TESTED_CAPABILITY_IDS,
    WRITE_CAPABILITY_IDS,
    backlogged_capabilities,
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


def _assert_python_symbol_exists(
    source: str, symbol: str, source_label: str, role: str
) -> None:
    declaration = re.compile(
        rf"^[ \t]*(?:async[ \t]+)?def[ \t]+{re.escape(symbol)}[ \t]*\(",
        re.MULTILINE,
    ).search(source)
    assert declaration, f"{role} {symbol!r} is missing from {source_label}"


def _assert_journey_evidence_is_executable(evidence: dict) -> None:
    required = {"capability", "source", "test", "assertion"}
    assert set(evidence) == required, (
        f"journey evidence fields must be exactly {sorted(required)}: {evidence}"
    )
    source_path = ROOT / evidence["source"]
    assert source_path.is_file(), (
        f"journey evidence source is missing: {evidence['source']}"
    )
    source = source_path.read_text()
    _assert_python_symbol_exists(
        source, evidence["test"], evidence["source"], "journey evidence test"
    )
    # The assertion helper is the part that actually reads provider state back.
    # Without it the named test still runs but proves nothing about the write.
    _assert_python_symbol_exists(
        source,
        evidence["assertion"],
        evidence["source"],
        "journey evidence assertion helper",
    )


def test_journey_evidence_names_an_executable_assertion():
    """A write may cite a journey only if its readback helper still exists."""
    capabilities = [entry["capability"] for entry in JOURNEY_EVIDENCE]
    duplicates = sorted(
        capability
        for capability in set(capabilities)
        if capabilities.count(capability) > 1
    )
    assert not duplicates, f"duplicate journey evidence: {duplicates}"
    assert JOURNEY_EVIDENCE_CAPABILITY_IDS <= TESTED_CAPABILITY_IDS, (
        "journey evidence for untested capabilities: "
        f"{sorted(JOURNEY_EVIDENCE_CAPABILITY_IDS - TESTED_CAPABILITY_IDS)}"
    )
    for evidence in JOURNEY_EVIDENCE:
        _assert_journey_evidence_is_executable(evidence)


@pytest.mark.parametrize(
    ("missing_symbol", "remaining_source"),
    [
        ("test_journey", "async def _assert_readback(url):\n    ...\n"),
        ("_assert_readback", "async def test_journey(world):\n    ...\n"),
    ],
)
def test_journey_evidence_rejects_a_vanished_symbol(
    missing_symbol: str, remaining_source: str
):
    """Deleting the readback helper must turn the gate red, not stay green."""
    with pytest.raises(AssertionError, match=rf"{missing_symbol}.* is missing"):
        _assert_python_symbol_exists(
            remaining_source, missing_symbol, "synthetic.py", "journey evidence"
        )


def _covered_capability_outcomes() -> dict[str, set[str]]:
    """Capability -> outcome classes proven by a typed operation case."""
    covered: dict[str, set[str]] = {}
    for case in PROVIDER_OPERATION_CASES:
        covered.setdefault(case.capability_id, set()).add(case.outcome_class)
    return covered


def test_write_capabilities_are_not_evidenced_by_a_recorded_tool_name():
    """A recorded tool-call name proves model choice, never a provider mutation.

    `_recorded_tool_evidence` only observes that some harvested trace emitted a
    call with this name. For an `external_write` capability that says nothing
    about whether the provider committed the effect, so it cannot stand in for
    a typed case with provider-side readback.
    """
    covered = set(_covered_capability_outcomes())
    unproven = sorted(
        WRITE_CAPABILITY_IDS
        - covered
        - INTEGRATION_EVIDENCE_CAPABILITY_IDS
        - JOURNEY_EVIDENCE_CAPABILITY_IDS
        - backlogged_capabilities("write_requires_operation_case")
    )
    assert not unproven, (
        "write capabilities whose only evidence is a recorded tool-call name; "
        "add a ProviderOperationCase with provider readback or an owned "
        f"coverage_backlog entry: {unproven}"
    )


def test_read_capabilities_cover_every_required_outcome_class():
    """Epic #6524 workstream 5: seeded success *and* empty-result per read."""
    covered = _covered_capability_outcomes()
    backlogged = backlogged_capabilities("read_requires_outcome_classes")
    missing = sorted(
        f"{capability_id}:{outcome_class}"
        for capability_id in READ_CAPABILITY_IDS
        - INTEGRATION_EVIDENCE_CAPABILITY_IDS
        - backlogged
        for outcome_class in REQUIRED_READ_OUTCOME_CLASSES
        - covered.get(capability_id, set())
    )
    assert not missing, (
        "read capabilities missing a required outcome class; add a "
        "ProviderOperationCase with that outcome_class or an owned "
        f"coverage_backlog entry: {missing}"
    )


def test_coverage_backlog_entries_are_owned_and_not_stale():
    """The backlog is a ratchet: it must shrink, and never hide live coverage."""
    covered = _covered_capability_outcomes()
    known_rules = {
        "write_requires_operation_case",
        "read_requires_outcome_classes",
    }
    seen: list[str] = []
    for entry in COVERAGE_BACKLOG:
        for field in ("rule", "owner", "reason", "issue", "review_condition"):
            assert entry.get(field), f"backlog entry is missing {field}: {entry}"
        assert entry["rule"] in known_rules, (
            f"unknown backlog rule {entry['rule']!r}: {sorted(known_rules)}"
        )
        assert entry["capabilities"], f"backlog entry has no capabilities: {entry}"
        seen.extend(entry["capabilities"])

    duplicates = sorted({c for c in seen if seen.count(c) > 1})
    assert not duplicates, f"capabilities backlogged more than once: {duplicates}"

    unknown = sorted(set(seen) - TESTED_CAPABILITY_IDS)
    assert not unknown, f"backlog names capabilities that do not ship: {unknown}"

    for entry in COVERAGE_BACKLOG:
        if entry["rule"] != "write_requires_operation_case":
            continue
        already_covered = sorted(set(entry["capabilities"]) & set(covered))
        assert not already_covered, (
            "backlog entry is stale — these write capabilities now have a "
            f"typed operation case and must be removed: {already_covered}"
        )
