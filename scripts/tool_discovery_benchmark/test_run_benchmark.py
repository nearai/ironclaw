import importlib.util
import sys
from pathlib import Path


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
    assert sum(map(len, first)) == 1000
    assert {len(bucket) for bucket in first} == {50}


def test_score_requires_complete_workflow_and_no_match_silence():
    workflow = next(task for task in BENCH.TASKS if task["id"] == "cross-namespace-workflow")
    no_match = next(task for task in BENCH.TASKS if task["id"] == "no-match")

    partial = BENCH.score_task(workflow, [{"name": "gmail__search_messages"}])
    complete = BENCH.score_task(
        workflow,
        [
            {"name": "gmail__search_messages"},
            {"name": "google_calendar__create_event"},
        ],
    )

    assert not partial["completed"]
    assert complete["completed"]
    assert BENCH.score_task(no_match, [])["completed"]
    assert not BENCH.score_task(no_match, [{"name": "unrelated"}])["completed"]


def test_upload_task_is_self_contained_and_does_not_require_a_workspace_fixture():
    upload = next(task for task in BENCH.TASKS if task["id"] == "nested-argument-vocabulary")

    assert "report.csv" in upload["prompt"]
    assert "benchmark-report" in upload["prompt"]
    assert "mime_type" in upload["prompt"]


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
