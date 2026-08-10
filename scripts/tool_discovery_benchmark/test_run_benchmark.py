import importlib.util
import sys
from pathlib import Path

import pytest


SCRIPT = Path(__file__).with_name("run_benchmark.py")
SPEC = importlib.util.spec_from_file_location("tool_discovery_benchmark", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
BENCH = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = BENCH
SPEC.loader.exec_module(BENCH)


def test_catalog_is_deterministic_bounded_and_fair():
    first = BENCH.generate_catalog(1000)
    second = BENCH.generate_catalog(1000)

    assert first == second
    assert len(first) == 20
    assert sum(len(bucket["tools"]) for bucket in first) == 1000
    assert max(len(bucket["tools"]) for bucket in first) - min(
        len(bucket["tools"]) for bucket in first
    ) <= 1
    namespace_by_tool = {
        tool["name"]: bucket["namespace"]
        for bucket in first
        for tool in bucket["tools"]
    }
    assert namespace_by_tool["github__get_pull_request"] == "github"
    assert namespace_by_tool["gmail__search_messages"] == "gmail"
    assert namespace_by_tool["google_calendar__create_event"] == "google-calendar"


def test_catalog_rejects_tool_count_below_fixed_corpus():
    corpus_size = len(BENCH.json.loads(BENCH.CORPUS_PATH.read_text())["tools"])

    with pytest.raises(ValueError, match=f"at least {corpus_size}"):
        BENCH.generate_catalog(corpus_size - 1)


def test_score_requires_complete_workflow_and_no_match_silence():
    workflow = next(task for task in BENCH.TASKS if task["id"] == "cross-namespace-workflow")
    no_match = next(task for task in BENCH.TASKS if task["id"] == "no-match")

    partial = BENCH.score_task(
        workflow,
        [{"name": "gmail__search_messages", "arguments": {"query": "Project Aurora"}}],
        [],
    )
    complete = BENCH.score_task(
        workflow,
        [
            {"name": "gmail__search_messages", "arguments": {"query": "Project Aurora"}},
            {
                "name": "google_calendar__create_event",
                "arguments": {
                    "schedule": {
                        "start_at": "2026-08-12T10:00:00Z",
                        "end_at": "2026-08-12T10:30:00Z",
                    }
                },
            },
        ],
        [],
    )
    reversed_calls = BENCH.score_task(
        workflow,
        list(reversed([
            {"name": "gmail__search_messages", "arguments": {"query": "Project Aurora"}},
            {
                "name": "google_calendar__create_event",
                "arguments": {
                    "schedule": {
                        "start_at": "2026-08-12T10:00:00Z",
                        "end_at": "2026-08-12T10:30:00Z",
                    }
                },
            },
        ])),
        [],
    )

    assert not partial["completed"]
    assert complete["completed"]
    assert not reversed_calls["completed"]
    assert BENCH.score_task(no_match, [], [{"name": "tool_search", "arguments": {}}])[
        "completed"
    ]
    assert not BENCH.score_task(
        no_match, [], [{"name": "builtin__write_file", "arguments": {}}]
    )["completed"]


def test_score_checks_required_arguments_and_unauthorized_attempts():
    upload = next(task for task in BENCH.TASKS if task["id"] == "nested-argument-vocabulary")
    denied = next(task for task in BENCH.TASKS if task["id"] == "denied-capability")
    wrong_upload = BENCH.score_task(
        upload,
        [{
            "name": "google_drive__upload_file",
            "arguments": {"name": "report.csv", "content": "wrong", "mime_type": "text/csv"},
        }],
        [],
    )
    denied_attempt = BENCH.score_task(
        denied,
        [],
        [{"name": "builtin__spawn_subagent", "arguments": {}}],
    )

    assert not wrong_upload["completed"]
    assert not denied_attempt["completed"]
    assert denied_attempt["unauthorized_tool_leaks"] == 1


def test_first_correct_tool_latency_skips_unrelated_calls():
    calls = [
        {"name": "unrelated", "monotonic_ns": 1_100_000_000},
        {"name": "expected", "monotonic_ns": 1_400_000_000},
    ]

    assert BENCH.first_correct_tool_call_latency_ms(("expected",), calls, 1.0) == 400
    assert BENCH.first_correct_tool_call_latency_ms((), calls, 1.0) is None


def test_upload_task_is_self_contained_and_does_not_require_a_workspace_fixture():
    upload = next(task for task in BENCH.TASKS if task["id"] == "nested-argument-vocabulary")

    assert "report.csv" in upload["prompt"]
    assert "benchmark-report" in upload["prompt"]
    assert "mime_type" in upload["prompt"]


def test_observations_are_durable_and_resume_without_duplicates(tmp_path):
    path = tmp_path / "observations.jsonl"
    observation = {
        "observation_id": "namespaces:100:no-match:0",
        "arm": "namespaces",
        "catalog": {"tool_count": 100},
        "run": {"repetition": 0},
        "task": {"id": "no-match"},
    }

    BENCH.append_observation(path, observation)
    BENCH.append_observation(path, observation)

    loaded = BENCH.load_observations(path)
    assert loaded == [observation]


def test_aggregate_keeps_completion_and_latency_by_arm_and_size():
    observations = [
        {
            "arm": "bridged",
            "catalog": {"tool_count": 100},
            "task": {"completed": True, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 10},
        },
        {
            "arm": "bridged",
            "catalog": {"tool_count": 100},
            "task": {"completed": False, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 30},
        },
    ]

    assert BENCH.aggregate_observations(observations) == [
        {
            "arm": "bridged",
            "tool_count": 100,
            "observations": 2,
            "completion_rate": 0.5,
            "latency_ms_median": 20.0,
            "latency_ms_worst": 30,
            "unauthorized_tool_leaks": 0,
        }
    ]
