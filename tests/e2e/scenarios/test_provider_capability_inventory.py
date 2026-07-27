"""Completeness gate for shipped first-party provider capabilities."""

import ast
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


def _python_function(
    source: str, symbol: str, source_label: str, role: str
) -> ast.FunctionDef | ast.AsyncFunctionDef:
    """Locate a module-level function definition, or fail with `role` context.

    Parsed rather than pattern-matched: the checks built on this must reason
    about executable code, and text matching cannot tell a real definition or
    call from one inside a comment or a string literal.
    """
    tree = ast.parse(source)
    matches = [
        node
        for node in tree.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == symbol
    ]
    if not matches:
        raise AssertionError(f"{role} {symbol!r} is missing from {source_label}")
    # Python binds the last definition, so inspecting the first would let a
    # stale earlier copy vouch for a redefinition that never calls the readback.
    # A duplicate module-level name is a defect in its own right, so reject it
    # rather than silently picking one.
    if len(matches) > 1:
        raise AssertionError(
            f"{role} {symbol!r} is defined {len(matches)} times at module level "
            f"in {source_label}; only the last definition runs, so the evidence "
            "is ambiguous"
        )
    return matches[0]


def _scope_nodes(node: ast.AST, *, top: bool = False):
    """Nodes in this function's own execution scope.

    Deliberately does not descend into nested `def`/`lambda`/`class` bodies: a
    call sitting inside a nested function that nobody invokes never runs, and
    must not count as evidence that the readback executed.
    """
    children = node.body if top else ast.iter_child_nodes(node)
    for child in children:
        if isinstance(
            child,
            (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef),
        ):
            continue
        yield child
        yield from _scope_nodes(child)


def _assert_python_symbol_called(
    function: ast.FunctionDef | ast.AsyncFunctionDef,
    helper: ast.FunctionDef | ast.AsyncFunctionDef,
    caller: str,
    source_label: str,
) -> None:
    symbol = helper.name
    nodes = list(_scope_nodes(function, top=True))
    calls = [
        node
        for node in nodes
        if isinstance(node, ast.Call)
        and (
            (isinstance(node.func, ast.Name) and node.func.id == symbol)
            or (isinstance(node.func, ast.Attribute) and node.func.attr == symbol)
        )
    ]
    assert calls, (
        f"journey evidence assertion helper {symbol!r} is never called by "
        f"{caller!r} in {source_label}; declaring it is not evidence that the "
        "readback runs"
    )
    if isinstance(helper, ast.AsyncFunctionDef):
        # An un-awaited coroutine call builds a coroutine and discards it. The
        # assertions inside it never execute, so it is not evidence either.
        awaited = {
            id(node.value) for node in nodes if isinstance(node, ast.Await)
        }
        assert any(id(call) in awaited for call in calls), (
            f"journey evidence assertion helper {symbol!r} is called by "
            f"{caller!r} in {source_label} but never awaited; the readback "
            "never runs"
        )


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
    test = _python_function(
        source, evidence["test"], evidence["source"], "journey evidence test"
    )
    # The assertion helper is the part that actually reads provider state back.
    # Without it the named test still runs but proves nothing about the write.
    helper = _python_function(
        source,
        evidence["assertion"],
        evidence["source"],
        "journey evidence assertion helper",
    )
    # Declaring the helper is not enough: deleting the call from the test would
    # leave both symbols and the recorded tool names intact, and the write
    # would still be credited. Bind the evidence to an actual invocation.
    _assert_python_symbol_called(
        test,
        helper,
        evidence["test"],
        evidence["source"],
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
        _python_function(
            remaining_source, missing_symbol, "synthetic.py", "journey evidence"
        )


@pytest.mark.parametrize(
    "test_body",
    [
        pytest.param("    await _something_else(world)\n", id="uncalled"),
        # A text search for "_assert_readback(" is satisfied by both of these
        # while the helper never runs, which would re-admit exactly the
        # declaration-only evidence this gate exists to reject.
        pytest.param(
            "    await _something_else(world)\n"
            "    # await _assert_readback(url)\n",
            id="commented out",
        ),
        pytest.param(
            '    note = "_assert_readback(url)"\n', id="inside a string"
        ),
    ],
)
def test_journey_evidence_rejects_a_helper_that_never_runs(test_body: str):
    """Both symbols present is not evidence; the test must invoke the readback."""
    source = (
        f"async def test_journey(world):\n{test_body}"
        "\n"
        "async def _assert_readback(url):\n"
        "    ...\n"
    )
    # The declaration check passes — this is exactly the hole it cannot see.
    test = _python_function(
        source, "test_journey", "synthetic.py", "journey evidence test"
    )
    helper = _python_function(
        source, "_assert_readback", "synthetic.py", "journey evidence helper"
    )
    with pytest.raises(AssertionError, match="is never called by 'test_journey'"):
        _assert_python_symbol_called(
            test, helper, "test_journey", "synthetic.py"
        )


def test_journey_evidence_rejects_an_unawaited_coroutine_call():
    """Calling an async readback without awaiting it never runs its assertions."""
    source = (
        "async def test_journey(world):\n"
        "    _assert_readback(world)\n"
        "\n"
        "async def _assert_readback(url):\n"
        "    ...\n"
    )
    test = _python_function(source, "test_journey", "synthetic.py", "test")
    helper = _python_function(source, "_assert_readback", "synthetic.py", "helper")
    with pytest.raises(AssertionError, match="never awaited"):
        _assert_python_symbol_called(
            test, helper, "test_journey", "synthetic.py"
        )


def test_journey_evidence_rejects_a_call_inside_an_uninvoked_nested_function():
    """A call in a nested def nobody invokes never executes."""
    source = (
        "async def test_journey(world):\n"
        "    async def _dead_branch():\n"
        "        await _assert_readback(world)\n"
        "    return None\n"
        "\n"
        "async def _assert_readback(url):\n"
        "    ...\n"
    )
    test = _python_function(source, "test_journey", "synthetic.py", "test")
    helper = _python_function(source, "_assert_readback", "synthetic.py", "helper")
    with pytest.raises(AssertionError, match="is never called by 'test_journey'"):
        _assert_python_symbol_called(
            test, helper, "test_journey", "synthetic.py"
        )


def test_journey_evidence_rejects_a_duplicated_definition():
    """Python binds the last def; a stale earlier copy must not vouch for it."""
    source = (
        "async def test_journey(world):\n"
        "    await _assert_readback(world)\n"
        "\n"
        "async def test_journey(world):\n"
        "    return None\n"
        "\n"
        "async def _assert_readback(url):\n"
        "    ...\n"
    )
    with pytest.raises(AssertionError, match="defined 2 times at module level"):
        _python_function(source, "test_journey", "synthetic.py", "test")


@pytest.mark.parametrize(
    "call",
    [
        pytest.param("    await _assert_readback(world)\n", id="direct"),
        # Matched via ast.Attribute, which is receiver-agnostic on purpose:
        # a helper reached through a module or holder object still counts.
        pytest.param(
            "    await helpers._assert_readback(world)\n",
            id="through a receiver",
        ),
    ],
)
def test_journey_evidence_accepts_a_genuinely_invoked_helper(call: str):
    """The check must pass on real calls, direct or through a receiver.

    Guards the other direction: a detector that rejects everything would make
    the gate unfalsifiable rather than strict.
    """
    source = (
        f"async def test_journey(world):\n{call}"
        "\n"
        "async def _assert_readback(url):\n"
        "    ...\n"
    )
    _assert_python_symbol_called(
        _python_function(source, "test_journey", "synthetic.py", "test"),
        _python_function(source, "_assert_readback", "synthetic.py", "helper"),
        "test_journey",
        "synthetic.py",
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
    # Integration evidence is not subtracted here. It names one executable test
    # per capability with no notion of outcome class, so letting it exempt a
    # read would be a silent exemption of exactly the kind this gate exists to
    # remove. Those capabilities are carried in the backlog with a reason.
    missing = sorted(
        f"{capability_id}:{outcome_class}"
        for capability_id in READ_CAPABILITY_IDS - backlogged
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

    # The ratchet applies to both rules. A read entry whose capability has since
    # gained every required outcome class must be deleted too, or the backlog
    # stops shrinking and the gate quietly stops meaning anything.
    for entry in COVERAGE_BACKLOG:
        if entry["rule"] == "write_requires_operation_case":
            already_covered = sorted(set(entry["capabilities"]) & set(covered))
            assert not already_covered, (
                "backlog entry is stale — these write capabilities now have a "
                f"typed operation case and must be removed: {already_covered}"
            )
        else:
            already_covered = sorted(
                capability
                for capability in entry["capabilities"]
                if REQUIRED_READ_OUTCOME_CLASSES <= covered.get(capability, set())
            )
            assert not already_covered, (
                "backlog entry is stale — these read capabilities now cover "
                f"every required outcome class and must be removed: "
                f"{already_covered}"
            )
