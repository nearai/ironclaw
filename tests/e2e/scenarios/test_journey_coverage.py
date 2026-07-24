"""Completeness gate for typed whole-path journey evidence."""

import ast
import json
import re
import tomllib
from pathlib import Path

from journey_cases import (
    ALL_JOURNEY_CASES,
    PROVIDER_JOURNEY_CASES,
    required_delivery_targets,
    required_ingresses,
    uncovered_surfaces,
)
from journey_types import EvidenceRunner, JourneyCase, ProviderWorld
from provider_capability_inventory import EMULATE_SUPPORTED_TOOLS

ROOT = Path(__file__).resolve().parents[3]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"


def _manifest_provider_journeys() -> set[str]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    excluded = set(manifest["no_model_cases"])
    excluded.update(manifest.get("quarantined_model_cases", []))
    cases = set()
    for case_id in manifest["selected_cases"]:
        if case_id in excluded:
            continue
        trace = json.loads((TRACE_DIR / f"{case_id}.json").read_text(encoding="utf-8"))
        if any(
            call["name"] in EMULATE_SUPPORTED_TOOLS
            for step in trace["steps"]
            for call in step["response"].get("tool_calls", [])
        ):
            cases.add(case_id)
    return cases


def _cargo_targets(manifest_path: Path) -> dict[str, str]:
    with manifest_path.open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
    return {
        target["name"]: target["path"]
        for target in manifest.get("test", [])
        if "path" in target
    }


def _assert_python_evidence(case: JourneyCase) -> None:
    evidence = case.evidence
    source_path = ROOT / evidence.source
    assert source_path.is_file(), f"{case.case_id}: missing {evidence.source}"
    tree = ast.parse(source_path.read_text(encoding="utf-8"))
    tests = {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name.startswith("test_")
    }
    assert evidence.test in tests, (
        f"{case.case_id}: pytest evidence {evidence.test!r} is missing from "
        f"{evidence.source}"
    )


def _assert_rust_evidence(case: JourneyCase) -> None:
    evidence = case.evidence
    source_path = ROOT / evidence.source
    assert source_path.is_file(), f"{case.case_id}: missing {evidence.source}"
    source = source_path.read_text(encoding="utf-8")
    declaration = re.compile(
        rf"(?P<attributes>(?:^[ \t]*#\s*\[[^\n]+\][ \t]*\n)+)"
        rf"^[ \t]*(?:pub\s+)?(?:async\s+)?fn\s+{re.escape(evidence.test)}\s*\(",
        re.MULTILINE,
    ).search(source)
    assert declaration, (
        f"{case.case_id}: Rust evidence {evidence.test!r} is missing from "
        f"{evidence.source}"
    )
    attributes = set(
        re.findall(
            r"#\s*\[\s*([A-Za-z_][A-Za-z0-9_:]*)",
            declaration.group("attributes"),
        )
    )
    assert attributes & {"test", "tokio::test"}, (
        f"{case.case_id}: Rust evidence {evidence.test!r} is not executable"
    )
    assert not attributes & {"cfg", "cfg_attr", "ignore"}, (
        f"{case.case_id}: Rust evidence {evidence.test!r} is disabled"
    )

    manifest_path = (
        ROOT / evidence.manifest
        if evidence.manifest is not None
        else ROOT / "Cargo.toml"
    )
    targets = _cargo_targets(manifest_path)
    if evidence.target in targets:
        expected_source = (manifest_path.parent / targets[evidence.target]).resolve()
        assert expected_source == source_path.resolve(), (
            f"{case.case_id}: Cargo target {evidence.target!r} points to "
            f"{expected_source}, not {source_path}"
        )
        return

    auto_target = manifest_path.parent / "tests" / f"{evidence.target}.rs"
    assert auto_target.resolve() == source_path.resolve(), (
        f"{case.case_id}: unknown Cargo target {evidence.target!r} in {manifest_path}"
    )


def test_provider_journey_registry_matches_every_harvested_emulate_journey():
    """Manifest additions cannot bypass the typed whole-path runner."""
    registered = {case.case_id for case in PROVIDER_JOURNEY_CASES}
    assert registered == _manifest_provider_journeys()


def test_every_journey_has_complete_typed_executable_evidence():
    """A coverage claim must name a real trace/world/path/assertion and test."""
    case_ids = [case.case_id for case in ALL_JOURNEY_CASES]
    duplicates = sorted(
        case_id for case_id in set(case_ids) if case_ids.count(case_id) > 1
    )
    assert not duplicates, f"duplicate journey ids: {duplicates}"

    for case in ALL_JOURNEY_CASES:
        assert case.provider_worlds, f"{case.case_id}: provider_worlds is empty"
        assert case.assertions, f"{case.case_id}: assertions is empty"
        if case.trace is not None:
            trace_path = ROOT / case.trace
            assert trace_path.is_file(), f"{case.case_id}: missing trace {case.trace}"
            assert ProviderWorld.NONE not in case.provider_worlds, (
                f"{case.case_id}: provider trace has no classified provider world"
            )
        if case.evidence.runner is EvidenceRunner.PYTEST:
            _assert_python_evidence(case)
        else:
            _assert_rust_evidence(case)


def test_every_supported_ingress_and_delivery_target_has_journey_evidence():
    """Production channel manifests and built-in surfaces stay a closed set."""
    missing_ingress = uncovered_surfaces(
        required_ingresses(), ALL_JOURNEY_CASES, lambda case: case.ingress
    )
    missing_delivery = uncovered_surfaces(
        required_delivery_targets(),
        ALL_JOURNEY_CASES,
        lambda case: case.delivery_target,
    )
    assert not missing_ingress, f"ingresses lack journey evidence: {missing_ingress}"
    assert not missing_delivery, (
        f"delivery targets lack journey evidence: {missing_delivery}"
    )


def test_surface_gate_reports_a_new_uncovered_surface():
    """The completeness gate must fail loudly when production gains a surface."""
    assert uncovered_surfaces(
        {"webui", "future-ingress"},
        ALL_JOURNEY_CASES,
        lambda case: case.ingress,
    ) == {"future-ingress"}
