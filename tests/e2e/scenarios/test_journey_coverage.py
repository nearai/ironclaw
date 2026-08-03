"""Completeness gate for typed whole-path journey evidence."""

import ast
import contextlib
import importlib.util
import json
import os
import re
import subprocess
from pathlib import Path

import pytest
import tomllib
from journey_cases import (
    _HISTORICAL_MUTATING_PROVIDER_TOOLS,
    _MUTATING_PROVIDER_TOOLS,
    _PROVIDER_REPLAY_FACTS,
    _TOOL_WORLD_PREFIXES,
    ALL_JOURNEY_CASES,
    JOURNEY_ORDER_ENV,
    PROVIDER_JOURNEY_CASES,
    _production_channel_capabilities,
    _provider_journey_cases,
    journey_order_is_reversed,
    provider_journey_runs,
    required_delivery_targets,
    required_ingresses,
    shared_world_provider_journey_runs,
    uncovered_surfaces,
    unreset_mutating_tools,
)
from journey_types import (
    CargoEvidence,
    DeliveryAddressEvidence,
    JourneyCase,
    ObservableAssertion,
    ProductJourneyCase,
    ProviderJourneyCase,
    ProviderJourneyReplayFacts,
    ProviderWorld,
    PytestEvidence,
)
from provider_capability_inventory import EMULATE_SUPPORTED_TOOLS
from provider_journey_google import require_single_google_account
from provider_journey_slack import (
    EMULATE_SLACK_CHANNEL_BEARER_ENV,
    emulate_slack_channel_bearer,
)
from provider_journey_trace import (
    MISSING_SLACK_CHANNEL_ID,
    compile_provider_journey_trace,
    load_recorded_trace,
    recorded_provider_calls,
)

ROOT = Path(__file__).resolve().parents[3]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"
_DISABLING_PYTEST_MARKS = {"skip", "skipif", "xfail"}
_JOURNEY_RUNNER_SOURCES = (
    ROOT / "tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
    ROOT / "tests/e2e/scenarios/test_reborn_qa_trace_replay.py",
    *sorted((ROOT / "tests/e2e").glob("provider_journey_*.py")),
)
_SEEDED_SLACK_STATE = {
    "channel_id": "C_SEEDED",
    "reviewer_id": "U_REVIEWER",
    "thread_ts": "1234.5",
    "channel_name": "reborn-alerts",
}
_EXPECTED_COMPILED_PROVIDER_CALLS = {
    "qa_2d_calendar_prep_live_chat": (
        "google-calendar__list_events",
        "google-calendar__list_events",
        "google-drive__list_files",
        "google-drive__download_file",
    ),
    "qa_2f_calendar_prep_email_delivery": (
        "google-calendar__list_events",
        "gmail__send_message",
    ),
    "qa_4e_github_release_email_delivery": ("gmail__send_message",),
    "qa_5d_slack_strategy_doc_answer": (
        "google-drive__upload_file",
        "google-drive__download_file",
        "google-drive__download_file",
    ),
    "qa_6c_gmail_to_sheet_live_chat": (
        "gmail__list_messages",
        "google-drive__list_files",
        "gmail__get_message",
        "google-sheets__get_spreadsheet",
        "google-sheets__read_values",
    ),
    "qa_6e_gmail_to_sheet_delivery": (
        "gmail__list_messages",
        "google-sheets__create_spreadsheet",
        "gmail__get_message",
        "google-sheets__append_values",
    ),
    "qa_7c_slack_bug_logger_routine": (
        "google-sheets__get_spreadsheet",
        "google-sheets__read_values",
    ),
    "qa_7e_slack_bug_sheet_delivery": (
        "google-sheets__create_spreadsheet",
        "google-sheets__rename_sheet",
        "google-sheets__write_values",
        "google-sheets__get_spreadsheet",
        "google-sheets__read_values",
        "google-sheets__append_values",
    ),
    "qa_10a_slack_self_attribution": (
        "slack__whoami",
        "slack__get_conversation_history",
    ),
    "qa_10b_slack_ooo_status": (
        "slack__whoami",
        "slack__get_user_info",
    ),
    "qa_10c_slack_thread_replies": (
        "slack__get_conversation_info",
        "slack__get_conversation_history",
        "slack__get_thread_replies",
    ),
    "qa_10d_slack_channel_membership": ("slack__list_conversations",),
    "qa_10e_slack_error_honesty": ("slack__get_conversation_history",),
    "qa_10f_slack_mention_encoding": (
        "slack__get_conversation_info",
        "slack__send_message",
    ),
    "qa_10g_slack_last_message_sent": ("slack__get_conversation_history",),
    "qa_10g_slack_last_message_sent_global": (
        "slack__whoami",
        "slack__search_messages",
    ),
    "qa_10h_slack_email_hallucination_guard": (
        "slack__list_conversations",
        "slack__get_user_info",
    ),
    "qa_10i_slack_raw_entity_hygiene": (
        "slack__get_conversation_info",
        "slack__search_messages",
        "slack__get_conversation_history",
    ),
}


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


def _case_name_branches(source_path: Path) -> list[int]:
    """Find runner control flow or dispatch keyed by journey identity."""
    tree = ast.parse(source_path.read_text(encoding="utf-8"))
    offenders = set()

    def _is_case_identity_node(node: ast.AST) -> bool:
        return (
            (
                isinstance(node, ast.Attribute)
                and node.attr in {"case_id", "stem", "trace"}
                and isinstance(node.value, ast.Name)
                and (node.value.id == "case" or node.value.id.endswith("_case"))
            )
            or (isinstance(node, ast.Name) and node.id in {"case_id", "case_name"})
            or (
                isinstance(node, ast.Compare)
                and isinstance(node.left, ast.Name)
                and node.left.id == "journey_case"
            )
        )

    def _reads_case_identity(node: ast.AST) -> bool:
        return any(_is_case_identity_node(child) for child in ast.walk(node))

    for node in ast.walk(tree):
        if isinstance(node, (ast.If, ast.IfExp)):
            selectors = (node.test,)
        elif isinstance(node, ast.Match):
            selectors = (
                node.subject,
                *(case.guard for case in node.cases if case.guard is not None),
            )
        else:
            continue
        for selector in selectors:
            embeds_case_name = any(
                isinstance(child, ast.Constant)
                and isinstance(child.value, str)
                and child.value.startswith("qa_")
                for child in ast.walk(selector)
            )
            if embeds_case_name or _reads_case_identity(selector):
                offenders.add(selector.lineno)
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            if (
                isinstance(node.func, ast.Attribute)
                and node.func.attr == "parametrize"
                and isinstance(node.func.value, ast.Attribute)
                and node.func.value.attr == "mark"
                and isinstance(node.func.value.value, ast.Name)
                and node.func.value.value.id == "pytest"
            ):
                continue
            arguments = (
                *node.args,
                *(keyword.value for keyword in node.keywords),
            )
        elif isinstance(node, ast.Subscript):
            arguments = (node.slice,)
        else:
            continue
        if any(_reads_case_identity(argument) for argument in arguments):
            offenders.add(node.lineno)
    return sorted(offenders)


def test_journey_runners_do_not_branch_on_case_names():
    """Journey-specific execution facts belong in typed declarations."""
    offenders = {
        str(source.relative_to(ROOT)): lines
        for source in _JOURNEY_RUNNER_SOURCES
        if (lines := _case_name_branches(source))
    }
    assert not offenders, f"journey-name branches found in runners: {offenders}"


def test_provider_replay_facts_must_name_collected_case(monkeypatch):
    monkeypatch.setitem(
        _PROVIDER_REPLAY_FACTS,
        "qa_unknown_provider_journey",
        ProviderJourneyReplayFacts(),
    )

    with pytest.raises(
        AssertionError,
        match="replay facts declared for unknown provider journey cases",
    ):
        _provider_journey_cases()


@pytest.mark.parametrize(
    "bad_runner",
    [
        (
            "def run(case):\n"
            "    if case.case_id == 'qa_new_special_case':\n"
            "        return 'special'\n"
        ),
        (
            "SPECIAL = '10e_slack_error_honesty.json'\n"
            "def run(journey_case):\n"
            "    if journey_case.trace.endswith(SPECIAL):\n"
            "        return 'special'\n"
        ),
        (
            "TIMEOUTS = {'qa_new_special_case': 180}\n"
            "def run(journey_case):\n"
            "    return TIMEOUTS.get(journey_case.case_id, 120)\n"
        ),
        (
            "SPECIAL = object()\n"
            "def run(journey_case):\n"
            "    if journey_case == SPECIAL:\n"
            "        return 'special'\n"
        ),
        (
            "SPECIAL = object()\n"
            "def run(journey_case):\n"
            "    match object():\n"
            "        case _ if journey_case == SPECIAL:\n"
            "            return 'special'\n"
        ),
        ("def run(case_id):\n    return timeout_for(case_id)\n"),
        ("def run(case_id):\n    return timeout_for(case_id=case_id)\n"),
        (
            "SPECIAL = object()\n"
            "def run(journey_case):\n"
            "    return TIMEOUTS[journey_case == SPECIAL]\n"
        ),
    ],
)
def test_case_name_branch_detector_fails_loudly(tmp_path, bad_runner):
    source = tmp_path / "bad_runner.py"
    source.write_text(bad_runner, encoding="utf-8")
    assert _case_name_branches(source)


def test_google_account_seed_rejects_an_empty_account_list():
    with pytest.raises(AssertionError, match="no selectable Google account"):
        require_single_google_account([], "no selectable Google account")


def test_google_account_seed_allows_an_explicit_existing_account():
    account = require_single_google_account(
        [],
        "no selectable Google account",
        allow_existing_account=True,
    )

    assert account is None


def test_slack_channel_bearer_requires_the_harness_environment(monkeypatch):
    monkeypatch.delenv(EMULATE_SLACK_CHANNEL_BEARER_ENV, raising=False)
    with pytest.raises(KeyError, match=EMULATE_SLACK_CHANNEL_BEARER_ENV):
        emulate_slack_channel_bearer()

    monkeypatch.setenv(EMULATE_SLACK_CHANNEL_BEARER_ENV, "test-channel-token")
    assert emulate_slack_channel_bearer() == "test-channel-token"


@pytest.mark.parametrize(
    "case",
    PROVIDER_JOURNEY_CASES,
    ids=lambda case: case.case_id,
)
def test_provider_trace_compilation_keeps_recording_immutable(case):
    trace_path = ROOT / case.trace
    fixture_before = trace_path.read_bytes()
    recorded = load_recorded_trace(trace_path)
    before = json.dumps(recorded, sort_keys=True)
    compiled = compile_provider_journey_trace(
        recorded,
        source=trace_path.name,
        facts=case.replay,
        provider_tools=EMULATE_SUPPORTED_TOOLS,
        slack_state=_SEEDED_SLACK_STATE,
    )

    assert json.dumps(recorded, sort_keys=True) == before
    assert trace_path.read_bytes() == fixture_before
    assert compiled.trace is not recorded


def test_provider_trace_compilation_declares_expected_failure():
    case = next(
        case
        for case in PROVIDER_JOURNEY_CASES
        if case.replay.expected_capability_failure is not None
    )
    trace_path = ROOT / case.trace
    compiled = compile_provider_journey_trace(
        load_recorded_trace(trace_path),
        source=trace_path.name,
        facts=case.replay,
        provider_tools=EMULATE_SUPPORTED_TOOLS,
        slack_state=_SEEDED_SLACK_STATE,
    )

    assert MISSING_SLACK_CHANNEL_ID in json.dumps(compiled.trace)
    assert compiled.trace["steps"][-1]["request_hint"] == {
        "expected_failed_tool_result_contains": (
            case.replay.expected_capability_failure
        )
    }


def test_provider_trace_compilation_preserves_provider_call_inventory():
    actual = {}
    for case in PROVIDER_JOURNEY_CASES:
        trace_path = ROOT / case.trace
        compiled = compile_provider_journey_trace(
            load_recorded_trace(trace_path),
            source=trace_path.name,
            facts=case.replay,
            provider_tools=EMULATE_SUPPORTED_TOOLS,
            slack_state=_SEEDED_SLACK_STATE,
        )
        actual[case.case_id] = tuple(
            call["name"]
            for call in recorded_provider_calls(compiled.trace, EMULATE_SUPPORTED_TOOLS)
        )

    assert actual == _EXPECTED_COMPILED_PROVIDER_CALLS


def test_provider_trace_compilation_uses_declared_google_seed():
    case = next(
        case
        for case in PROVIDER_JOURNEY_CASES
        if case.case_id == "qa_7c_slack_bug_logger_routine"
    )
    trace_path = ROOT / case.trace
    compiled = compile_provider_journey_trace(
        load_recorded_trace(trace_path),
        source=trace_path.name,
        facts=case.replay,
        provider_tools=EMULATE_SUPPORTED_TOOLS,
        slack_state=_SEEDED_SLACK_STATE,
    )
    sheet_calls = [
        call
        for call in recorded_provider_calls(compiled.trace, EMULATE_SUPPORTED_TOOLS)
        if call["name"].startswith("google-sheets__")
    ]

    assert sheet_calls
    assert {
        call["arguments"]["spreadsheet_id"]
        for call in sheet_calls
        if "spreadsheet_id" in call["arguments"]
    } == {case.replay.google_spreadsheet_id}


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


def _rust_code_without_comments_or_strings(
    source: str,
    *,
    preserve_strings: bool = False,
) -> str:
    """Mask Rust comments and optionally strings, preserving positions."""
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
            if not preserve_strings:
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
            if not preserve_strings:
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
        rf"^[ \t]*(?:pub\s+)?(?P<async>async\s+)?"
        rf"fn\s+{re.escape(test_name)}\s*\(",
        re.MULTILINE,
    ).search(source)
    assert declaration, f"Rust evidence {test_name!r} is missing from {source_label}"
    attributes = set(
        re.findall(
            r"#\s*\[\s*([A-Za-z_][A-Za-z0-9_:]*)",
            declaration.group("attributes"),
        )
    )
    if declaration.group("async"):
        assert "tokio::test" in attributes, (
            f"Rust evidence {test_name!r} lacks an async-compatible test attribute"
        )
    else:
        assert "test" in attributes, f"Rust evidence {test_name!r} is not executable"
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


def _rust_function_body(
    source: str,
    function_name: str,
    *,
    preserve_literals: bool = False,
) -> str:
    """Return one Rust body, optionally preserving strings and comments."""
    masked = _rust_code_without_comments_or_strings(source)
    declarations = list(
        re.finditer(
            rf"\bfn\s+{re.escape(function_name)}\s*\([^)]*\)[^{{;]*\{{",
            masked,
            re.MULTILINE,
        )
    )
    assert len(declarations) == 1, (
        f"expected one Rust function {function_name!r}, found {len(declarations)}"
    )
    body_start = masked.find("{", declarations[0].start())
    depth = 0
    for index in range(body_start, len(masked)):
        if masked[index] == "{":
            depth += 1
        elif masked[index] == "}":
            depth -= 1
            if depth == 0:
                body_source = (
                    _rust_code_without_comments_or_strings(
                        source,
                        preserve_strings=True,
                    )
                    if preserve_literals
                    else masked
                )
                return body_source[body_start + 1 : index]
    raise AssertionError(f"Rust function {function_name!r} has no closing brace")


def _rust_value_pattern(value: str) -> str:
    if value.lstrip("-").isdigit():
        return rf"(?:{re.escape(json.dumps(value))}|{re.escape(value)})"
    return re.escape(json.dumps(value))


def _assert_rust_assignment(
    case_id: str,
    assertion: str,
    variable: str,
    value: str,
    *,
    optional: bool = False,
) -> None:
    value_pattern = _rust_value_pattern(value)
    if optional:
        value_pattern = rf"Some\s*\(\s*{value_pattern}\s*\)"
    assert re.search(
        rf"\blet\s+{re.escape(variable)}(?:\s*:[^=;]+)?\s*=\s*"
        rf"{value_pattern}\s*;",
        assertion,
    ), (
        f"{case_id}: declared delivery value {value!r} is not bound to "
        f"{variable} by the cited helper"
    )


def _assert_delivery_address_is_citable(
    case: ProductJourneyCase,
    address: DeliveryAddressEvidence,
) -> None:
    assert isinstance(case.evidence, CargoEvidence), (
        f"{case.case_id}: external delivery evidence must cite a Cargo caller seam"
    )
    assert address.conversation_id.strip(), (
        f"{case.case_id}: delivery conversation id is blank"
    )
    assert address.thread_anchor is None or address.thread_anchor.strip(), (
        f"{case.case_id}: delivery thread anchor is blank"
    )
    assert address.exact_count == 1, (
        f"{case.case_id}: representative delivery must assert exactly once"
    )
    assert ObservableAssertion.EXACT_DESTINATION in case.assertions
    assert ObservableAssertion.EXACT_MUTATION_COUNT in case.assertions

    source = (ROOT / case.evidence.source).read_text(encoding="utf-8")
    assertion_body = _rust_function_body(
        source,
        address.assertion,
        preserve_literals=True,
    )
    _assert_rust_assignment(
        case.case_id,
        assertion_body,
        "expected_conversation_id",
        address.conversation_id,
    )
    assert re.search(r"==\s*expected_conversation_id\b", assertion_body), (
        f"{case.case_id}: expected_conversation_id does not gate provider evidence"
    )
    if address.thread_anchor is None:
        assert re.search(
            r"\blet\s+expected_thread_anchor(?:\s*:[^=;]+)?\s*=\s*None\s*;",
            assertion_body,
        ), (
            f"{case.case_id}: declared unthreaded delivery is not asserted "
            "by the cited helper"
        )
    else:
        _assert_rust_assignment(
            case.case_id,
            assertion_body,
            "expected_thread_anchor",
            address.thread_anchor,
            optional=True,
        )
    assert re.search(
        r"==\s*expected_thread_anchor\b",
        assertion_body,
    ), (
        f"{case.case_id}: expected_thread_anchor does not gate provider evidence"
    )
    _assert_rust_assignment(
        case.case_id,
        assertion_body,
        "expected_count",
        str(address.exact_count),
    )
    assert re.search(
        r"\bmatching\s*\.\s*count\s*\(\s*\)\s*,\s*expected_count\b",
        assertion_body,
    ), (
        f"{case.case_id}: expected_count does not gate the provider mutation count"
    )

    test_body = _rust_function_body(source, case.evidence.test)
    reachable_bodies = [test_body]
    for delegate in set(re.findall(r"\b([a-z][A-Za-z0-9_]*_impl)\s*\(", test_body)):
        if delegate == address.assertion:
            continue
        with contextlib.suppress(AssertionError):
            reachable_bodies.append(_rust_function_body(source, delegate))
    assert any(
        re.search(rf"\b{re.escape(address.assertion)}\s*\(", body)
        for body in reachable_bodies
    ), (
        f"{case.case_id}: cited assertion {address.assertion!r} is not called "
        f"by {case.evidence.test!r} or its direct delegate"
    )


def test_literal_preserving_rust_extraction_masks_comments():
    """Commented-out evidence cannot satisfy the mechanical inventory."""
    source = """
fn evidence() {
    // let comment_only = "C-FAKE";
    /* let block_comment_only = "C-ALSO-FAKE"; */
    let executable = "C777";
}
"""
    body = _rust_function_body(source, "evidence", preserve_literals=True)
    assert "comment_only" not in body
    assert "block_comment_only" not in body
    assert '"C777"' in body


def test_provider_journey_registry_matches_every_harvested_emulate_journey():
    """Manifest additions cannot bypass the typed whole-path runner."""
    registered = {case.case_id for case in PROVIDER_JOURNEY_CASES}
    assert registered == _manifest_provider_journeys()


def _expected_forward_ids() -> list[str]:
    expected_repeat_cases = {
        "qa_5d_slack_strategy_doc_answer",
        "qa_10f_slack_mention_encoding",
    }
    expected_ids = []
    for case in PROVIDER_JOURNEY_CASES:
        expected_ids.append(case.case_id)
        if case.case_id in expected_repeat_cases:
            expected_ids.append(f"{case.case_id}-isolated-repeat")
    return expected_ids


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

    expected_ids = _expected_forward_ids()
    forward_runs, forward_ids = provider_journey_runs(reverse=False)
    assert list(forward_ids) == expected_ids
    assert [case.case_id for case in forward_runs] == [
        case_id.removesuffix("-isolated-repeat") for case_id in expected_ids
    ]


def test_reversed_journey_order_runs_every_case_back_to_front():
    """The reversed lane must actually reverse, and drop nothing.

    A reversed lane that quietly ran forward would pass exactly like the
    ordinary lane and retire the proof it exists to provide — the same
    silent-no-op shape as a guard that never fires. Asserting the order flipped
    AND that the multiset is unchanged catches both halves: a lane that does
    not reverse, and a "reversal" that loses or duplicates a case.
    """
    forward_runs, forward_ids = provider_journey_runs(reverse=False)
    reversed_runs, reversed_ids = provider_journey_runs(reverse=True)

    assert list(reversed_ids) == list(reversed(forward_ids))
    assert sorted(reversed_ids) == sorted(forward_ids)
    assert [case.case_id for case in reversed_runs] == [
        case.case_id for case in reversed(forward_runs)
    ]
    # Guards the degenerate case: with fewer than two runs, "reversed" and
    # "forward" are the same list and every assertion above passes vacuously.
    assert len(forward_ids) > 1, forward_ids
    assert list(reversed_ids) != list(forward_ids)


def test_shared_world_replay_reverses_each_mutating_journey_once():
    """The shared lane excludes read-only cases and self-colliding repeats."""
    forward_runs, forward_ids = shared_world_provider_journey_runs(reverse=False)
    reversed_runs, reversed_ids = shared_world_provider_journey_runs(reverse=True)

    assert forward_runs
    assert all(case.mutable_provider_worlds for case in forward_runs)
    assert len(forward_ids) == len(set(forward_ids))
    assert list(reversed_ids) == list(reversed(forward_ids))
    assert [case.case_id for case in reversed_runs] == [
        case.case_id for case in reversed(forward_runs)
    ]


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        ("reverse", True),
        ("REVERSE", True),
        ("  reverse  ", True),
        ("forward", False),
        ("", False),
        (None, False),
    ],
)
def test_journey_order_env_selects_the_reversed_lane(monkeypatch, value, expected):
    """The lane switch is the part CI sets, so parse it deliberately.

    A typo or a stray space silently selecting the forward lane is the failure
    that makes a green reversed run meaningless.
    """
    if value is None:
        monkeypatch.delenv(JOURNEY_ORDER_ENV, raising=False)
    else:
        monkeypatch.setenv(JOURNEY_ORDER_ENV, value)
    assert journey_order_is_reversed() is expected


def test_alone_lane_lists_every_mutating_journey():
    """The alone-lane loop is only as good as the list it iterates.

    An empty or drifted list would make the nightly step iterate fewer times,
    exit 0, and retire the proof with nothing failing — so pin the list
    against the case inventory rather than trusting the script's own output.
    """
    script = ROOT / "scripts/ci/list_mutating_journeys.py"
    assert script.is_file(), script
    spec = importlib.util.spec_from_file_location("list_mutating_journeys", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    expected = [
        case.case_id for case in PROVIDER_JOURNEY_CASES if case.mutable_provider_worlds
    ]
    assert expected, "no mutating journeys: the alone lane would test nothing"
    assert module.mutating_journey_ids() == expected


@pytest.mark.parametrize(
    ("stub_body", "expected_returncode"),
    [
        ("exit 23\n", 23),
        ("exit 0\n", 1),
    ],
)
def test_alone_lane_rejects_failed_or_empty_inventory(
    tmp_path,
    stub_body,
    expected_returncode,
):
    """The workflow must fail closed before replaying an invalid inventory."""
    workflow = (ROOT / ".github/workflows/reborn-e2e.yml").read_text(encoding="utf-8")
    step = re.search(
        r"      - name: Replay each mutating journey alone\n"
        r"(?:        .*\n)*?"
        r"        run: \|\n"
        r"(?P<script>(?:          .*\n)+)",
        workflow,
    )
    assert step, "missing isolated journey replay workflow step"
    script = "\n".join(
        line.removeprefix("          ") for line in step.group("script").splitlines()
    )

    python_stub = tmp_path / "python"
    python_stub.write_text(
        f"#!/usr/bin/env bash\n{stub_body}",
        encoding="utf-8",
    )
    python_stub.chmod(0o755)
    result = subprocess.run(
        ["bash", "-c", script],
        cwd=ROOT,
        env={**os.environ, "PATH": f"{tmp_path}:{os.environ['PATH']}"},
        check=False,
    )

    assert result.returncode == expected_returncode


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
        if isinstance(case, ProductJourneyCase) and case.browser_evidence is not None:
            _assert_python_evidence(case, case.browser_evidence)


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


def _assert_production_delivery_variants(
    by_surface: dict[str, list[DeliveryAddressEvidence]],
    production_capabilities: dict[str, dict],
) -> None:
    for surface, capabilities in production_capabilities.items():
        addresses = by_surface.get(surface, [])
        assert any(address.thread_anchor is None for address in addresses), (
            f"{surface}: no unthreaded delivery address evidence"
        )
        supports_threads = capabilities.get("supports_threads")
        assert isinstance(supports_threads, bool), (
            f"{surface}: outbound manifest must declare supports_threads as a boolean"
        )
        if supports_threads:
            assert any(address.thread_anchor is not None for address in addresses), (
                f"{surface}: threaded delivery is implemented but lacks exact evidence"
            )


def test_external_delivery_variants_name_exact_caller_evidence():
    """Opaque destinations and optional anchors stay mechanically citable."""
    product_cases = [
        case for case in ALL_JOURNEY_CASES if isinstance(case, ProductJourneyCase)
    ]
    by_surface: dict[str, list[DeliveryAddressEvidence]] = {}
    for case in product_cases:
        for address in case.delivery_addresses:
            _assert_delivery_address_is_citable(case, address)
            by_surface.setdefault(str(case.delivery_target), []).append(address)

    _assert_production_delivery_variants(
        by_surface, _production_channel_capabilities("outbound")
    )

    slack_destinations = {
        address.conversation_id for address in by_surface.get("slack", [])
    }
    assert {"D-TRIGGER-DEFAULT", "C-TRIGGER-OVERRIDE"} <= slack_destinations, (
        "Slack's existing DM and shared-channel caller proofs must remain "
        "independently citable"
    )


@pytest.mark.parametrize(
    "capabilities",
    ({}, {"supports_thread": True}, {"supports_threads": "true"}),
)
def test_delivery_variant_gate_requires_explicit_boolean_threading_declaration(
    capabilities,
):
    """Missing, misspelled, or mistyped declarations cannot disable evidence."""
    unthreaded = DeliveryAddressEvidence(
        conversation_id="C-EXACT",
        thread_anchor=None,
        exact_count=1,
        assertion="assert_exact_delivery",
    )
    with pytest.raises(
        AssertionError,
        match="outbound manifest must declare supports_threads as a boolean",
    ):
        _assert_production_delivery_variants(
            {"slack": [unthreaded]},
            {"slack": capabilities},
        )


def test_channel_capability_inventory_requires_a_manifest_id(tmp_path: Path):
    """Malformed production manifests fail with a path-specific diagnostic."""
    manifest_dir = tmp_path / "missing-id"
    manifest_dir.mkdir()
    (manifest_dir / "manifest.toml").write_text(
        "[channel]\noutbound = true\n\n"
        "[channel.presentation]\nsupports_threads = false\n"
    )

    with pytest.raises(
        AssertionError,
        match=r"missing-id/manifest\.toml: channel manifest declares no non-empty id",
    ):
        _production_channel_capabilities("outbound", asset_root=tmp_path)


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


def test_rust_evidence_rejects_plain_test_attribute_on_async_function():
    """Cargo's plain test harness cannot execute an async test function."""
    source = "#[test]\nasync fn required_journey() {}\n"
    with pytest.raises(AssertionError, match="async-compatible"):
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


# ---------------------------------------------------------------------------
# Provider-world baseline gate (#6524 workstream 3)
#
# A journey that writes to a provider world must declare that world, because
# the declaration is what triggers the reset afterwards. Without it, whatever
# the journey created survives into the next test and the leak guards this
# workstream added never run.
#
# Which tools write is not a judgement call: production says so, as the
# `external_write` effect on each manifest tool. The harness used to restate
# that as a hand-kept list of five names while production declared seventy.
# Nothing detected the drift, because a journey using an undeclared write
# simply resets nothing and passes.
# ---------------------------------------------------------------------------


def test_every_production_provider_write_maps_to_a_resettable_world():
    """No production write can run without a world that gets reset."""
    unreset = unreset_mutating_tools()
    assert not unreset, (
        "these production tools declare `external_write` but belong to no "
        f"provider world the harness can reset: {sorted(unreset)}. A journey "
        "using one would mutate a world that nothing restores, and the next "
        "test would inherit the result. Add the world to _TOOL_WORLD_PREFIXES "
        "with a reset path, or give the tool a world that has one."
    )


def test_provider_write_derivation_still_finds_the_tools_it_replaced():
    """The derivation cannot quietly collapse to nothing.

    This is the gate on the gate. Deriving the set from manifests removes the
    drift risk but adds a worse one: a manifest key rename would empty the
    mapping, every journey would declare no mutable world, every reset would
    be skipped, and the whole suite would still pass. The five names below are
    the ones the hand-kept list carried, so they are a floor the derivation
    must always clear.
    """
    missing = sorted(
        _HISTORICAL_MUTATING_PROVIDER_TOOLS - set(_MUTATING_PROVIDER_TOOLS)
    )
    assert not missing, (
        f"the derivation stopped recognising known provider writes: {missing}. "
        "It reads the `external_write` effect from "
        "crates/extensions/packages/*/manifest.toml -- check "
        "whether that key or the tool ids were renamed. Until this is fixed no "
        "journey resets its provider world."
    )
    # A count alone is a weak check: discovery could break for one provider
    # and still clear any global floor on the strength of the others. Assert
    # per world instead, so a single provider's manifests going unread fails
    # here and names that provider.
    derived_by_world: dict[str, list[str]] = {}
    for tool_name, world in _MUTATING_PROVIDER_TOOLS.items():
        derived_by_world.setdefault(str(world), []).append(tool_name)
    resettable_worlds = {str(world) for world in _TOOL_WORLD_PREFIXES.values()}
    empty_worlds = sorted(resettable_worlds - set(derived_by_world))
    assert not empty_worlds, (
        f"no provider writes were derived for {empty_worlds}, but every world "
        "the harness can reset ships write tools. Journeys touching those "
        "providers would declare no mutable world and skip their reset. Check "
        "whether those manifests moved or their tool ids were renamed."
    )

    # Production currently declares 70 provider writes. Hold the floor close
    # to that rather than at a token value: a partial discovery failure that
    # still finds most tools is exactly what a low floor would wave through.
    # Deliberately removing write tools should require moving this number, and
    # noticing that you are.
    assert len(_MUTATING_PROVIDER_TOOLS) >= 60, (
        f"only {len(_MUTATING_PROVIDER_TOOLS)} provider writes were derived from "
        "the shipped manifests; production declares about 70. Either the "
        "derivation is reading the wrong manifests or key, or write tools were "
        "removed -- if the removal is intentional, lower this floor in the same "
        "change so the drop is reviewed rather than absorbed."
    )
