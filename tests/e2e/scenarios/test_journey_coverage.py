"""Completeness gate for typed whole-path journey evidence."""

import ast
import json
import re
from pathlib import Path

import pytest
import tomllib
from journey_cases import (
    ALL_JOURNEY_CASES,
    PROVIDER_JOURNEY_CASES,
    PROVIDER_JOURNEY_RUN_IDS,
    PROVIDER_JOURNEY_RUNS,
    required_delivery_targets,
    required_ingresses,
    uncovered_surfaces,
)
from journey_types import (
    CargoEvidence,
    JourneyCase,
    ProviderJourneyCase,
    ProviderWorld,
    PytestEvidence,
)
from provider_capability_inventory import EMULATE_SUPPORTED_TOOLS

ROOT = Path(__file__).resolve().parents[3]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"
_DISABLING_PYTEST_MARKS = {"skip", "skipif", "xfail"}


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


def _cargo_test_config(manifest_path: Path) -> tuple[dict[str, dict], bool]:
    with manifest_path.open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
    targets = {target["name"]: target for target in manifest.get("test", [])}
    package = manifest.get("package", {})
    if "autotests" in package:
        autotests_enabled = package["autotests"]
    else:
        edition = package.get("edition", "2015")
        has_manual_target = any(
            target_kind in manifest
            for target_kind in ("lib", "bin", "test", "bench", "example")
        )
        autotests_enabled = edition != "2015" or not has_manual_target
    return targets, autotests_enabled


def _disabling_pytest_marks(
    node: ast.AST,
    aliases: dict[str, set[str]],
) -> set[str]:
    marks = {
        candidate.attr
        for candidate in ast.walk(node)
        if isinstance(candidate, ast.Attribute)
        and candidate.attr in _DISABLING_PYTEST_MARKS
    }
    for candidate in ast.walk(node):
        if isinstance(candidate, ast.Name):
            marks.update(aliases.get(candidate.id, set()))
    return marks


def _pytest_mark_aliases(tree: ast.Module) -> dict[str, set[str]]:
    aliases: dict[str, set[str]] = {}
    for statement in tree.body:
        if isinstance(statement, (ast.Assign, ast.AnnAssign)):
            targets = (
                statement.targets
                if isinstance(statement, ast.Assign)
                else [statement.target]
            )
            if statement.value is None:
                continue
            if isinstance(statement.value, ast.Name):
                marks = aliases.setdefault(statement.value.id, set())
            else:
                marks = _disabling_pytest_marks(statement.value, aliases)
            for target in targets:
                if isinstance(target, ast.Name):
                    aliases[target.id] = marks
        elif isinstance(statement, ast.AugAssign) and isinstance(
            statement.target, ast.Name
        ):
            aliases.setdefault(statement.target.id, set()).update(
                _disabling_pytest_marks(statement.value, aliases)
            )
        elif (
            isinstance(statement, ast.Expr)
            and isinstance(statement.value, ast.Call)
            and isinstance(statement.value.func, ast.Attribute)
            and statement.value.func.attr in {"append", "extend", "insert"}
            and isinstance(statement.value.func.value, ast.Name)
        ):
            collection = aliases.setdefault(statement.value.func.value.id, set())
            for argument in statement.value.args:
                collection.update(_disabling_pytest_marks(argument, aliases))
    return aliases


def _assert_python_test_declaration(
    source: str,
    test_name: str,
    source_label: str,
) -> None:
    tree = ast.parse(source)
    aliases = _pytest_mark_aliases(tree)
    module_disabling_marks = aliases.get("pytestmark", set())
    assert not module_disabling_marks, (
        f"pytest evidence {test_name!r} is disabled by module-level marks "
        f"{sorted(module_disabling_marks)} in {source_label}"
    )

    tests = {
        node.name: node
        for node in tree.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name.startswith("test_")
    }
    assert test_name in tests, (
        f"pytest evidence {test_name!r} is missing from {source_label}"
    )
    disabling_marks = {
        mark
        for decorator in tests[test_name].decorator_list
        for mark in _disabling_pytest_marks(decorator, aliases)
    }
    assert not disabling_marks, (
        f"pytest evidence {test_name!r} is disabled by test-level marks "
        f"{sorted(disabling_marks)} in {source_label}"
    )


def _assert_python_evidence(case: JourneyCase, evidence: PytestEvidence) -> None:
    source_path = ROOT / evidence.source
    assert source_path.is_file(), f"{case.case_id}: missing {evidence.source}"
    _assert_python_test_declaration(
        source_path.read_text(encoding="utf-8"),
        evidence.test,
        evidence.source,
    )


def _rust_code_without_comments_or_strings(source: str) -> str:
    """Mask Rust comments and strings while preserving line positions."""
    result = list(source)
    index = 0
    block_depth = 0
    while index < len(source):
        if block_depth:
            if source.startswith("/*", index):
                result[index : index + 2] = "  "
                block_depth += 1
                index += 2
            elif source.startswith("*/", index):
                result[index : index + 2] = "  "
                block_depth -= 1
                index += 2
            else:
                if source[index] != "\n":
                    result[index] = " "
                index += 1
            continue
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end == -1 else end
            result[index:end] = " " * (end - index)
            index = end
            continue
        if source.startswith("/*", index):
            result[index : index + 2] = "  "
            block_depth = 1
            index += 2
            continue
        raw_match = re.match(r'(?:b)?r(#{0,255})"', source[index:])
        if raw_match:
            hashes = raw_match.group(1)
            delimiter = f'"{hashes}'
            end = source.find(delimiter, index + raw_match.end())
            end = len(source) if end == -1 else end + len(delimiter)
            for position in range(index, end):
                if source[position] != "\n":
                    result[position] = " "
            index = end
            continue
        if source[index] == '"':
            end = index + 1
            while end < len(source):
                if source[end] == "\\":
                    end += 2
                    continue
                end += 1
                if source[end - 1] == '"':
                    break
            for position in range(index, min(end, len(source))):
                if source[position] != "\n":
                    result[position] = " "
            index = end
            continue
        index += 1
    return "".join(result)


def _assert_rust_test_declaration(
    source: str,
    test_name: str,
    source_label: str,
) -> None:
    source = _rust_code_without_comments_or_strings(source)
    declaration = re.compile(
        rf"(?P<attributes>(?:^[ \t]*#\s*\[[^\n]+\][ \t]*\n)+)"
        rf"^[ \t]*(?:pub\s+)?(?:async\s+)?fn\s+{re.escape(test_name)}\s*\(",
        re.MULTILINE,
    ).search(source)
    assert declaration, f"Rust evidence {test_name!r} is missing from {source_label}"
    attributes = set(
        re.findall(
            r"#\s*\[\s*([A-Za-z_][A-Za-z0-9_:]*)",
            declaration.group("attributes"),
        )
    )
    assert attributes & {"test", "tokio::test"}, (
        f"Rust evidence {test_name!r} is not executable"
    )
    assert not attributes & {"cfg", "cfg_attr", "ignore"}, (
        f"Rust evidence {test_name!r} is disabled"
    )


def _assert_cargo_target(
    case_id: str,
    evidence: CargoEvidence,
    source_path: Path,
    root: Path = ROOT,
) -> None:
    manifest_path = (
        root / evidence.manifest
        if evidence.manifest is not None
        else root / "Cargo.toml"
    )
    targets, autotests_enabled = _cargo_test_config(manifest_path)
    if evidence.target in targets:
        target = targets[evidence.target]
        assert target.get("test", True) is not False, (
            f"{case_id}: Cargo target {evidence.target!r} disables test execution"
        )
        assert target.get("harness", True) is not False, (
            f"{case_id}: Cargo target {evidence.target!r} disables the test harness"
        )
        required_features = target.get("required-features", [])
        assert not required_features, (
            f"{case_id}: Cargo target {evidence.target!r} requires features "
            f"{required_features} that journey evidence does not enable"
        )
        target_path = target.get("path", f"tests/{evidence.target}.rs")
        expected_source = (manifest_path.parent / target_path).resolve()
        assert expected_source == source_path.resolve(), (
            f"{case_id}: Cargo target {evidence.target!r} points to "
            f"{expected_source}, not {source_path}"
        )
        return

    assert autotests_enabled, (
        f"{case_id}: Cargo manifest disables automatic test discovery"
    )
    auto_target = manifest_path.parent / "tests" / f"{evidence.target}.rs"
    assert auto_target.resolve() == source_path.resolve(), (
        f"{case_id}: unknown Cargo target {evidence.target!r} in {manifest_path}"
    )


def _assert_rust_evidence(case: JourneyCase, evidence: CargoEvidence) -> None:
    source_path = ROOT / evidence.source
    assert source_path.is_file(), f"{case.case_id}: missing {evidence.source}"
    _assert_rust_test_declaration(
        source_path.read_text(encoding="utf-8"),
        evidence.test,
        evidence.source,
    )
    _assert_cargo_target(case.case_id, evidence, source_path)


def test_provider_journey_registry_matches_every_harvested_emulate_journey():
    """Manifest additions cannot bypass the typed whole-path runner."""
    registered = {case.case_id for case in PROVIDER_JOURNEY_CASES}
    assert registered == _manifest_provider_journeys()


def test_provider_journey_runs_preserve_isolated_repeat_cases():
    """The two isolation probes remain doubled while ordinary cases run once."""
    expected_repeat_cases = {
        "qa_5d_slack_strategy_doc_answer",
        "qa_10f_slack_mention_encoding",
    }
    actual_repeat_cases = {
        case.case_id for case in PROVIDER_JOURNEY_CASES if case.repeat_after_reset
    }
    assert actual_repeat_cases == expected_repeat_cases

    expected_ids = []
    for case in PROVIDER_JOURNEY_CASES:
        expected_ids.append(case.case_id)
        if case.case_id in expected_repeat_cases:
            expected_ids.append(f"{case.case_id}-isolated-repeat")
    assert list(PROVIDER_JOURNEY_RUN_IDS) == expected_ids
    assert [case.case_id for case in PROVIDER_JOURNEY_RUNS] == [
        case_id.removesuffix("-isolated-repeat") for case_id in expected_ids
    ]


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
        if isinstance(case, ProviderJourneyCase):
            trace_path = ROOT / case.trace
            assert trace_path.is_file(), f"{case.case_id}: missing trace {case.trace}"
            assert ProviderWorld.NONE not in case.provider_worlds, (
                f"{case.case_id}: provider trace has no classified provider world"
            )
        if isinstance(case.evidence, PytestEvidence):
            _assert_python_evidence(case, case.evidence)
        else:
            _assert_rust_evidence(case, case.evidence)


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


@pytest.mark.parametrize(
    "source",
    [
        (
            "@pytest.mark.skip(reason='disabled')\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
        (
            "pytestmark = pytest.mark.skip(reason='disabled')\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
        (
            "@pytest.mark.xfail(run=False, reason='disabled')\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
        (
            "skip = pytest.mark.skip\n"
            "@skip(reason='disabled')\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
        (
            "pytestmark = []\n"
            "pytestmark += [pytest.mark.skip]\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
        (
            "pytestmark = []\n"
            "pytestmark.append(pytest.mark.skip)\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
        (
            "marks = []\n"
            "marks += [pytest.mark.skip]\n"
            "pytestmark = marks\n"
            "def test_required_journey():\n"
            "    pass\n"
        ),
    ],
)
def test_python_evidence_rejects_disabled_tests(source: str):
    """A named Python test cannot satisfy the gate while disabled."""
    with pytest.raises(AssertionError, match=r"disabled by .* marks"):
        _assert_python_test_declaration(
            source,
            "test_required_journey",
            "synthetic.py",
        )


@pytest.mark.parametrize(
    "source",
    [
        '#[cfg(feature = "disabled")]\n#[test]\nfn required_journey() {}\n',
        "#[cfg_attr(test, ignore)]\n#[test]\nfn required_journey() {}\n",
        "#[ignore]\n#[test]\nfn required_journey() {}\n",
        "/*\n#[tokio::test]\nasync fn required_journey() {}\n*/\n",
    ],
)
def test_rust_evidence_rejects_disabled_or_commented_tests(source: str):
    """Disabled or commented Rust declarations cannot satisfy the gate."""
    with pytest.raises(AssertionError, match=r"(disabled|missing)"):
        _assert_rust_test_declaration(source, "required_journey", "synthetic.rs")


def test_cargo_evidence_rejects_a_misdirected_target(tmp_path: Path):
    """A Cargo target must resolve to the source named by the evidence."""
    (tmp_path / "tests").mkdir()
    manifest_path = tmp_path / "Cargo.toml"
    manifest_path.write_text(
        '[[test]]\nname = "journey"\npath = "tests/actual.rs"\n',
        encoding="utf-8",
    )
    expected_source = tmp_path / "tests/expected.rs"
    expected_source.touch()
    evidence = CargoEvidence(
        source="tests/expected.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="points to"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            expected_source,
            root=tmp_path,
        )


@pytest.mark.parametrize(
    "path_entry",
    ['path = "tests/journey.rs"\n', ""],
    ids=["explicit-path", "implicit-path"],
)
def test_cargo_evidence_rejects_a_feature_gated_target(
    tmp_path: Path,
    path_entry: str,
):
    """A target that CI cannot run cannot satisfy executable evidence."""
    (tmp_path / "tests").mkdir()
    manifest_path = tmp_path / "Cargo.toml"
    manifest_path.write_text(
        "[[test]]\n"
        'name = "journey"\n'
        f"{path_entry}"
        'required-features = ["test-support"]\n',
        encoding="utf-8",
    )
    source_path = tmp_path / "tests/journey.rs"
    source_path.touch()
    evidence = CargoEvidence(
        source="tests/journey.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="requires features"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            source_path,
            root=tmp_path,
        )


def test_cargo_evidence_rejects_a_harnessless_target(tmp_path: Path):
    """A binary without Cargo's test harness cannot prove a named test runs."""
    (tmp_path / "tests").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[[test]]\nname = "journey"\npath = "tests/journey.rs"\nharness = false\n',
        encoding="utf-8",
    )
    source_path = tmp_path / "tests/journey.rs"
    source_path.touch()
    evidence = CargoEvidence(
        source="tests/journey.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="disables the test harness"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            source_path,
            root=tmp_path,
        )


def test_cargo_evidence_rejects_a_target_with_test_disabled(tmp_path: Path):
    """A target excluded from cargo test cannot satisfy executable evidence."""
    (tmp_path / "tests").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[[test]]\nname = "journey"\npath = "tests/journey.rs"\ntest = false\n',
        encoding="utf-8",
    )
    source_path = tmp_path / "tests/journey.rs"
    source_path.touch()
    evidence = CargoEvidence(
        source="tests/journey.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="disables test execution"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            source_path,
            root=tmp_path,
        )


def test_cargo_evidence_rejects_disabled_implicit_discovery(tmp_path: Path):
    """An inferred tests-directory target requires Cargo autotests."""
    (tmp_path / "tests").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "synthetic"\nversion = "0.1.0"\nautotests = false\n',
        encoding="utf-8",
    )
    source_path = tmp_path / "tests/journey.rs"
    source_path.touch()
    evidence = CargoEvidence(
        source="tests/journey.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="automatic test discovery"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            source_path,
            root=tmp_path,
        )


def test_cargo_evidence_rejects_legacy_manual_target_implicit_discovery(
    tmp_path: Path,
):
    """Edition 2015 disables default discovery when a target is configured."""
    (tmp_path / "src").mkdir()
    (tmp_path / "tests").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "synthetic"\nversion = "0.1.0"\n'
        '[[bin]]\nname = "tool"\npath = "src/main.rs"\n',
        encoding="utf-8",
    )
    (tmp_path / "src/main.rs").touch()
    source_path = tmp_path / "tests/journey.rs"
    source_path.touch()
    evidence = CargoEvidence(
        source="tests/journey.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="automatic test discovery"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            source_path,
            root=tmp_path,
        )


def test_cargo_evidence_counts_an_empty_lib_table_as_a_manual_target(
    tmp_path: Path,
):
    """Even an empty target table disables edition-2015 auto-discovery."""
    (tmp_path / "src").mkdir()
    (tmp_path / "tests").mkdir()
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "synthetic"\nversion = "0.1.0"\n[lib]\n',
        encoding="utf-8",
    )
    (tmp_path / "src/lib.rs").touch()
    source_path = tmp_path / "tests/journey.rs"
    source_path.touch()
    evidence = CargoEvidence(
        source="tests/journey.rs",
        test="required_journey",
        target="journey",
    )
    with pytest.raises(AssertionError, match="automatic test discovery"):
        _assert_cargo_target(
            "synthetic",
            evidence,
            source_path,
            root=tmp_path,
        )
