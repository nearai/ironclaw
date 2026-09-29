import asyncio
import importlib.util
import json
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


def test_catalog_rejects_empty_namespace_packages():
    corpus_size = len(BENCH.json.loads(BENCH.CORPUS_PATH.read_text())["tools"])

    with pytest.raises(ValueError, match="empty benchmark namespace"):
        BENCH.generate_catalog(corpus_size)


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


def test_discovery_turns_count_model_steps_not_calls():
    calls = [
        {"name": "tool_search", "model_turn": 1},
        {"name": "tool_describe", "model_turn": 1},
        {"name": "tool_search", "model_turn": 3},
        {"name": "github__get_repo", "model_turn": 4},
    ]

    assert BENCH.discovery_turn_count(calls) == 2


def test_result_read_count_includes_bridged_targets_in_either_spelling():
    calls = [
        {"name": "builtin__result_read", "arguments": {"result_id": "a"}},
        {"name": "tool_call", "arguments": {"name": "builtin.result_read"}},
        {"name": "tool_call", "arguments": {"name": "builtin__result_read"}},
        {"name": "tool_call", "arguments": {"name": "github__get_repo"}},
        {"name": "tool_search", "arguments": {"query": "result_read"}},
        {"name": "builtin__result_reader", "arguments": {}},
    ]

    assert BENCH.result_read_call_count(calls) == 3
    assert BENCH.result_read_call_count([]) == 0


def test_bridged_tool_call_count_counts_only_the_bridge():
    calls = [
        {"name": "tool_call", "arguments": {"name": "github__get_repo"}},
        {"name": "tool_call", "arguments": {}},
        {"name": "github__get_repo", "arguments": {}},
        {"name": "tool_search", "arguments": {"query": "tool_call"}},
    ]

    assert BENCH.bridged_tool_call_count(calls) == 2
    assert BENCH.bridged_tool_call_count([]) == 0


def test_tool_definition_signature_changes_counts_array_changes_only():
    tools_a = [{"type": "function", "function": {"name": "a", "parameters": {}}}]
    tools_b = [{"type": "function", "function": {"name": "b", "parameters": {}}}]
    sig_a = BENCH.tool_definitions_signature({"messages": [], "tools": tools_a})
    sig_b = BENCH.tool_definitions_signature({"messages": [], "tools": tools_b})
    reordered = BENCH.tool_definitions_signature(
        {"messages": [], "tools": tools_b + tools_a}
    )

    assert sig_a == BENCH.tool_definitions_signature({"tools": list(tools_a)})
    assert sig_a != sig_b
    assert reordered != BENCH.tool_definitions_signature({"tools": tools_a + tools_b})
    assert BENCH.tool_definitions_signature({"messages": []}) is None
    assert BENCH.tool_definitions_signature({"tools": []}) is None

    stable = [{"tools_signature": sig_a}] * 3
    changed = [
        {"tools_signature": sig_a},
        {"tools_signature": None},
        {"tools_signature": sig_b},
        {"tools_signature": sig_b},
        {"tools_signature": sig_a},
    ]
    assert BENCH.tool_definition_signature_changes(stable) == 0
    assert BENCH.tool_definition_signature_changes(changed) == 2
    assert BENCH.tool_definition_signature_changes([{"tools_signature": None}]) is None
    assert BENCH.tool_definition_signature_changes([]) is None


def test_request_conversation_names_the_repetition_from_its_user_messages():
    marker = "BENCHMARK_DONE_prefetch_lexical_100_topic_drift_2"
    opening = {"role": "user", "content": f"List issues.\n\nEnd with exactly: {marker}_T1"}
    follow_up = {
        "role": "user",
        "content": [{"type": "text", "text": f"Now book it. End with: {marker}_T2"}],
    }
    system = {"role": "system", "content": "BENCHMARK_DONE_not_a_user_message"}

    assert BENCH.request_conversation({"messages": [system, opening, follow_up]}) == marker
    assert BENCH.request_conversation({"messages": [system, follow_up]}) == marker
    assert BENCH.request_conversation({"messages": [system]}) is None
    assert BENCH.request_conversation({"tools": []}) is None
    assert BENCH.conversation_key(f"{marker}_T1") == marker
    assert BENCH.conversation_key(marker) == marker

    mine = {"conversation": marker, "tools_signature": "A"}
    unknown = {"conversation": None, "tools_signature": "A"}
    other = {"conversation": f"{marker[:-1]}1", "tools_signature": "B"}
    assert BENCH.own_requests([mine, other, unknown, other], marker) == [mine, unknown]


def test_request_recorder_relays_unchanged_and_records_tools_hash():
    received = []

    class Upstream(BENCH.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            return

        def do_POST(self):
            length = int(self.headers.get("content-length", "0"))
            received.append({
                "path": self.path,
                "authorization": self.headers.get("authorization"),
                "body": self.rfile.read(length),
            })
            payload = b'data: {"ok": true}\n\ndata: [DONE]\n\n'
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("content-length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    upstream = BENCH.ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    thread = BENCH.threading.Thread(target=upstream.serve_forever, daemon=True)
    thread.start()
    recorder = BENCH.LlmRequestRecorder(
        f"http://127.0.0.1:{upstream.server_address[1]}/api/v1/"
    )
    recorder.start()
    try:
        tools = [{"type": "function", "function": {"name": "a", "parameters": {}}}]
        body = BENCH.json.dumps({"model": "m", "messages": [], "tools": tools}).encode()
        request = BENCH.urllib.request.Request(
            f"{recorder.base_url}/chat/completions",
            data=body,
            headers={"authorization": "Bearer secret", "content-type": "application/json"},
            method="POST",
        )
        with BENCH.urllib.request.urlopen(request, timeout=10) as response:
            relayed = response.read()
    finally:
        recorder.stop()
        upstream.shutdown()
        upstream.server_close()

    assert relayed == b'data: {"ok": true}\n\ndata: [DONE]\n\n'
    assert received == [{
        "path": "/api/v1/chat/completions",
        "authorization": "Bearer secret",
        "body": body,
    }]
    assert len(recorder.requests) == 1
    assert recorder.requests[0]["tools_signature"] == BENCH.tool_definitions_signature(
        {"tools": tools}
    )
    assert recorder.requests[0]["tool_names"] == ["a"]
    assert recorder.requests[0]["tool_count"] == 1
    assert recorder.requests[0]["tool_schema_tokens"] == (
        BENCH.advertised_tools({"tools": tools})["schema_tokens"]
    )
    assert "secret" not in BENCH.json.dumps(recorder.requests)


def test_model_base_url_resolution_mirrors_live_qa_config():
    assert BENCH.resolve_model_base_url({}) is None
    assert BENCH.resolve_model_base_url({
        "REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID": "openai_compatible",
        "LIVE_OPENAI_COMPATIBLE_BASE_URL": "https://api.example/v1",
    }) == "https://api.example/v1"
    assert BENCH.resolve_model_base_url({
        "REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL": "https://override/v1",
    }) == "https://override/v1"


def test_git_head_is_nonempty_checked_provenance():
    head = asyncio.run(BENCH.git_head())

    assert len(head) == 40
    assert all(character in "0123456789abcdef" for character in head)


def test_upload_task_is_self_contained_and_does_not_require_a_workspace_fixture():
    upload = next(task for task in BENCH.TASKS if task["id"] == "nested-argument-vocabulary")

    assert "report.csv" in upload["prompt"]
    assert "benchmark-report" in upload["prompt"]
    assert "mime_type" in upload["prompt"]
    assert "text/csv" in upload["prompt"]


def test_observations_are_durable_and_resume_without_duplicates(tmp_path):
    path = tmp_path / "observations.jsonl"
    observation = {
        "schema_version": BENCH.OBSERVATION_SCHEMA_VERSION,
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


def test_observation_resume_rejects_schema_6_output(tmp_path):
    # Schema 7 made the Jev arm's provider configurable (TypeSafe by
    # default) and records its endpoint in `config.jev`; a schema-6
    # directory, run against another provider, must not be resumed into a
    # schema-7 summary.
    assert BENCH.OBSERVATION_SCHEMA_VERSION == 7
    assert BENCH.SUMMARY_SCHEMA_VERSION == 7
    path = tmp_path / "observations.jsonl"
    path.write_text(
        BENCH.json.dumps({"schema_version": 6, "observation_id": "off:100:no-match:0"})
        + "\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="schema_version 6; expected 7"):
        BENCH.load_observations(path)


def test_observation_resume_rejects_stale_schema_before_deduplication(tmp_path):
    path = tmp_path / "observations.jsonl"
    path.write_text(
        BENCH.json.dumps({
            "schema_version": BENCH.OBSERVATION_SCHEMA_VERSION - 1,
            "observation_id": "namespaces:100:no-match:0",
        }) + "\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="schema_version"):
        BENCH.load_observations(path)


def test_aggregate_keeps_completion_and_latency_by_arm_and_size():
    observations = [
        {
            "arm": "bridged",
            "catalog": {"tool_count": 100},
            "task": {"completed": True, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 10},
            "failure": None,
        },
        {
            "arm": "bridged",
            "catalog": {"tool_count": 100},
            "task": {"completed": False, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 30},
            "failure": "task_incomplete",
        },
    ]
    assert BENCH.aggregate_observations(observations) == [
        {
            "arm": "bridged",
            "tool_count": 100,
            "max_tools": None,
            "observations": 2,
            "completion_rate": 0.5,
            "latency_ms_median": 20.0,
            "latency_ms_worst": 30,
            "latency_ms_spread": 20,
            "unauthorized_tool_leaks": 0,
            "failure_categories": {"task_incomplete": 1},
            "model_turns_mean": None,
            "model_turns_median": None,
            "model_turns_max": None,
            "tool_search_calls_total": None,
            "tool_search_calls_mean": None,
            "tool_describe_calls_total": None,
            "tool_describe_calls_mean": None,
            "result_read_calls_total": None,
            "discovery_result_read_calls_total": None,
            "bridged_tool_calls_total": None,
            "token_usage_observations": 0,
            "input_tokens_mean": None,
            "input_tokens_median": None,
            "cached_input_tokens_mean": None,
            "cached_input_tokens_median": None,
            "first_correct_tool_observations": 0,
            "time_to_first_correct_tool_ms_median": None,
            "time_to_first_correct_tool_ms_worst": None,
            "tool_definition_signature_changes_total": None,
            "observations_with_tool_definition_changes": None,
            "config": None,
            "config_consistent": True,
            "advertised_requests": 0,
            "advertised_tool_count_p20_median_p80": None,
            "advertised_tool_count_max": None,
            "advertised_schema_tokens_p20_median_p80": None,
            "advertised_schema_tokens_max": None,
            "turn0_tool_count_p20_median_p80": None,
            "selection_hit_rate_observations": 0,
            "selection_hit_rate_mean": None,
            "selection_full_hit_observations": None,
            "selection_misses_total": None,
            "observations_with_selection_misses": None,
            "selection_log_observations": 0,
            "turn0_selection_latency_ms_p20_median_p80": None,
            "turn0_selection_latency_ms_max": None,
            "core_set_fallback_observations": None,
            "core_set_fallback_rate": None,
            "core_set_fallback_reasons": None,
            "index_observations": 0,
            "index_embedded_at_selection_total": None,
            "index_missing_at_selection_total": None,
            "index_dense_fallback_rate": None,
            "index_dense_fallback_reasons": None,
            "jev_observations": 0,
            "jev_models": None,
            "jev_slices_per_conversation_p20_median_p80": None,
            "jev_slices_per_conversation_max": None,
            "jev_input_tokens_total": None,
            "jev_input_tokens_mean": None,
            "jev_cost_usd_total": None,
            "jev_cost_usd_per_conversation_mean": None,
        }
    ]


def _observation(arm, tool_count, *, counts, tokens, first_correct, signature_changes):
    return {
        "arm": arm,
        "catalog": {"tool_count": tool_count},
        "task": {"completed": True, "unauthorized_tool_leaks": 0},
        "latency_ms": {"end_to_end": 100, "time_to_first_correct_tool_call": first_correct},
        "counts": counts,
        "tokens": tokens,
        "cache": {"tool_definition_signature_changes": signature_changes},
        "failure": None,
    }


def test_aggregate_reports_turns_discovery_tokens_and_first_correct_tool():
    counts = {
        "model_turns": 2, "tool_search_calls": 1, "tool_describe_calls": 0,
        "result_read_calls": 0, "bridged_tool_calls": 1,
    }
    observations = [
        _observation(
            "namespaces", 500,
            counts=counts,
            tokens={"input": 1000, "cached_input": 800},
            first_correct=300, signature_changes=0,
        ),
        _observation(
            "namespaces", 500,
            counts={**counts, "model_turns": 4, "tool_describe_calls": 2},
            tokens={"input": 3000, "cached_input": 0},
            first_correct=None, signature_changes=2,
        ),
        _observation(
            "namespaces", 500,
            counts={**counts, "model_turns": 3, "result_read_calls": 1},
            # Provider reported no usage: must not be averaged in as zero.
            tokens={"input": 0, "cached_input": None},
            first_correct=500, signature_changes=None,
        ),
    ]

    [aggregate] = BENCH.aggregate_observations(observations)

    assert aggregate["model_turns_mean"] == 3.0
    assert aggregate["model_turns_median"] == 3
    assert aggregate["model_turns_max"] == 4
    assert aggregate["tool_search_calls_total"] == 3
    assert aggregate["tool_search_calls_mean"] == 1.0
    assert aggregate["tool_describe_calls_total"] == 2
    assert aggregate["tool_describe_calls_mean"] == 0.67
    assert aggregate["result_read_calls_total"] == 1
    assert aggregate["bridged_tool_calls_total"] == 3
    assert aggregate["token_usage_observations"] == 2
    assert aggregate["input_tokens_mean"] == 2000.0
    assert aggregate["input_tokens_median"] == 2000.0
    assert aggregate["cached_input_tokens_mean"] == 400.0
    assert aggregate["first_correct_tool_observations"] == 2
    assert aggregate["time_to_first_correct_tool_ms_median"] == 400.0
    assert aggregate["time_to_first_correct_tool_ms_worst"] == 500
    assert aggregate["tool_definition_signature_changes_total"] == 2
    assert aggregate["observations_with_tool_definition_changes"] == 1


def test_run_cache_metadata_marks_first_resumed_execution_cold():
    assert BENCH.run_cache_metadata([2, 3], 0, 2) == {
        "thermal_class": "cold",
        "repetition": 2,
        "resumed_group": True,
    }
    assert BENCH.run_cache_metadata([2, 3], 1, 3) == {
        "thermal_class": "warm",
        "repetition": 3,
        "resumed_group": True,
    }


TEI_ENV = {
    "EMBEDDING_PROVIDER": "openai_compatible",
    "EMBEDDING_BASE_URL": "http://127.0.0.1:8080",
    "EMBEDDING_MODEL": "BAAI/bge-small-en-v1.5",
    "EMBEDDING_DIMENSION": "384",
}


def test_arms_include_the_turn_start_selection_arms():
    assert BENCH.ARMS[:5] == ("off", "compact", "signatures", "namespaces", "bridged")
    assert {"prefetch-lexical", "prefetch-lexical-floor", "prefetch-semantic"} <= set(
        BENCH.ARMS
    )


def test_baseline_arms_set_only_their_disclosure_mode():
    catalogs = BENCH.generate_catalog(100)
    for arm in ("off", "compact", "signatures", "namespaces"):
        env, config = BENCH.arm_env(arm, catalogs, dict(TEI_ENV))
        assert env == {"REBORN_TOOL_DISCLOSURE": arm}
        assert config["prefetch"] == "off"
        assert config["retrieval"] == "native"
        assert config["embeddings"] is None
        assert config["prefetch_tuning"] is None

    bridged, _ = BENCH.arm_env("bridged", catalogs, {})
    pins = BENCH.json.loads(bridged["REBORN_TOOL_DISCLOSURE_PROFILE_PINS"])
    assert "mcp-benchmark-github.github__get_pull_request" in pins["interactive_tools"]


def test_prefetch_lexical_arms_env_and_recorded_config():
    catalogs = BENCH.generate_catalog(100)
    tuning = {"REBORN_TOOL_PREFETCH_MAX_TOOLS": "40"}

    env, config = BENCH.arm_env("prefetch-lexical", catalogs, tuning)
    assert env == {
        "REBORN_TOOL_DISCLOSURE": "namespaces",
        "REBORN_TOOL_PREFETCH": "lexical",
        "REBORN_TOOL_RETRIEVAL": "native",
    }
    assert config["prefetch"] == "lexical"
    assert config["prefetch_always"] == []
    assert config["prefetch_tuning"]["REBORN_TOOL_PREFETCH_MAX_TOOLS"] == "40"
    assert config["prefetch_tuning"]["REBORN_TOOL_PREFETCH_TOKEN_BUDGET"] is None
    assert config["embeddings"] is None

    floor_env, floor_config = BENCH.arm_env("prefetch-lexical-floor", catalogs, {})
    always = floor_env["REBORN_TOOL_PREFETCH_ALWAYS"].split(",")
    assert floor_env["REBORN_TOOL_PREFETCH"] == "lexical"
    assert "outbound_deliver" in always and "memory_search" in always
    assert "tool_search" not in always  # the mandatory floor is implicit
    assert floor_config["prefetch_always"] == always


def test_prefetch_semantic_arm_uses_hybrid_and_records_the_endpoint():
    catalogs = BENCH.generate_catalog(100)
    env, config = BENCH.arm_env(
        "prefetch-semantic", catalogs,
        {**TEI_ENV, "EMBEDDING_API_KEY": "sk-secret"},
    )

    assert env == {
        "REBORN_TOOL_DISCLOSURE": "namespaces",
        "REBORN_TOOL_PREFETCH": "semantic",
        "REBORN_TOOL_RETRIEVAL": "hybrid",
    }
    assert config["retrieval"] == "hybrid"
    assert config["embeddings"] == {
        "provider": "openai_compatible",
        "model": "BAAI/bge-small-en-v1.5",
        "base_url": "http://127.0.0.1:8080",
        "dimension": 384,
        "api_key_env": "EMBEDDING_API_KEY",
    }
    assert "sk-secret" not in BENCH.json.dumps(config)


def test_embeddings_config_redacts_url_credentials_and_defaults_openai_url():
    recorded = BENCH.embeddings_config({
        **TEI_ENV, "EMBEDDING_BASE_URL": "http://user:pw@10.0.0.5:8080/v1",
    })
    assert recorded["base_url"] == "http://10.0.0.5:8080/v1"

    openai = BENCH.embeddings_config({
        "EMBEDDING_PROVIDER": "openai",
        "EMBEDDING_MODEL": "text-embedding-3-small",
        "EMBEDDING_API_KEY_ENV": "MY_KEY",
        "MY_KEY": "sk-x",
    })
    assert openai["base_url"] == "https://api.openai.com/v1"
    assert openai["api_key_env"] == "MY_KEY"


@pytest.mark.parametrize(
    ("env", "reason"),
    [
        ({}, "EMBEDDING_PROVIDER"),
        ({**TEI_ENV, "EMBEDDING_PROVIDER": "ollama"}, "EMBEDDING_PROVIDER"),
        ({k: v for k, v in TEI_ENV.items() if k != "EMBEDDING_MODEL"}, "EMBEDDING_MODEL"),
        (
            {k: v for k, v in TEI_ENV.items() if k != "EMBEDDING_BASE_URL"},
            "EMBEDDING_BASE_URL",
        ),
        (
            {"EMBEDDING_PROVIDER": "openai", "EMBEDDING_MODEL": "m"},
            "EMBEDDING_API_KEY",
        ),
    ],
)
def test_semantic_arm_refuses_without_embeddings_env(env, reason):
    with pytest.raises(ValueError, match=reason):
        BENCH.arm_env("prefetch-semantic", BENCH.generate_catalog(100), env)


class _NoServerLiveQa:
    """Live-QA stand-in that fails the test if a server would be started."""

    def __init__(self):
        self.started = []

    def create_generated_reborn_home(self, path):
        self.started.append(("home", path))
        return path

    def case_llm_trace_env(self, *_args):
        self.started.append(("trace",))
        return {}

    async def start_reborn_server(self, *_args):
        self.started.append(("server",))
        raise AssertionError("server must not start")


def test_semantic_task_group_fails_before_starting_the_server(tmp_path, monkeypatch):
    for name in TEI_ENV:
        monkeypatch.delenv(name, raising=False)
    live_qa = _NoServerLiveQa()
    task = next(task for task in BENCH.TASKS if task["id"] == "no-match")

    with pytest.raises(ValueError, match="needs an embeddings endpoint"):
        asyncio.run(BENCH.run_task_group(
            live_qa, tmp_path / "ironclaw", tmp_path, "prefetch-semantic", 100,
            task, [0], tmp_path / "observations.jsonl",
        ))

    assert live_qa.started == []
    assert not (tmp_path / "cases").exists()


def test_run_refuses_a_semantic_arm_up_front_and_strips_inherited_arm_env(
    tmp_path, monkeypatch,
):
    for name in TEI_ENV:
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NEARAI_API_KEY", "test-key")
    monkeypatch.setenv("REBORN_TOOL_PREFETCH", "semantic")
    monkeypatch.setenv("REBORN_TOOL_RETRIEVAL", "dense")
    binary = tmp_path / "ironclaw"
    binary.write_text("")
    live_qa = _NoServerLiveQa()
    monkeypatch.setattr(BENCH, "_load_live_qa", lambda: live_qa)
    args = BENCH.argparse.Namespace(
        output_dir=tmp_path / "out", binary=binary,
        arm=["prefetch-lexical", "prefetch-semantic"], tool_count=[100],
        task=None, repetitions=1, no_request_recorder=True,
    )

    with pytest.raises(RuntimeError, match="arm prefetch-semantic: .*EMBEDDING_PROVIDER"):
        asyncio.run(BENCH.async_main(args))

    assert live_qa.started == []
    assert "REBORN_TOOL_PREFETCH" not in BENCH.os.environ
    assert "REBORN_TOOL_RETRIEVAL" not in BENCH.os.environ


def _tool(name):
    return {
        "type": "function",
        "function": {"name": name, "description": "d", "parameters": {"type": "object"}},
    }


def test_advertised_tools_counts_names_and_estimates_schema_tokens():
    tools = [_tool("tool_search"), {"name": "anthropic_style", "input_schema": {}}]
    advertised = BENCH.advertised_tools({"messages": [], "tools": tools})
    encoded = BENCH.json.dumps(tools, separators=(",", ":"))

    assert advertised["names"] == ["tool_search", "anthropic_style"]
    assert advertised["count"] == 2
    assert advertised["schema_tokens"] == -(-len(encoded) // 4)
    assert BENCH.advertised_tools({"messages": []}) is None
    assert BENCH.advertised_tools({"tools": []}) is None


def test_advertised_metrics_lists_tool_bearing_requests_only():
    requests = [
        {"tool_count": 5, "tool_schema_tokens": 800},
        {"tool_count": None, "tool_schema_tokens": None},
        {"tool_count": 5, "tool_schema_tokens": 800},
    ]

    metrics = BENCH.advertised_metrics(requests)
    assert metrics["tool_count_per_request"] == [5, 5]
    assert metrics["schema_tokens_per_request"] == [800, 800]
    assert "/ 4" in metrics["schema_token_estimator"]


def test_selection_metrics_hit_rate_and_misses():
    turn0 = [
        "tool_search", "tool_describe", "tool_call", "builtin__result_read",
        "mcp-benchmark-gmail__gmail__search_messages",
    ]
    requests = [
        {"tool_names": None},  # a tools-free side call is not turn 0
        {"tool_names": turn0},
        {"tool_names": turn0},
    ]
    calls = [
        {"name": "mcp-benchmark-gmail__gmail__search_messages", "arguments": {}},
        {"name": "tool_search", "arguments": {"query": "calendar"}},
        {
            "name": "tool_call",
            "arguments": {"name": "mcp-benchmark-google-calendar.google_calendar__create_event"},
        },
        {"name": "tool_call", "arguments": {"name": "google_calendar__create_event"}},
        {"name": "tool_call", "arguments": {"name": "builtin.result_read"}},
        {"name": "tool_call", "arguments": {}},
    ]

    metrics = BENCH.selection_metrics(requests, calls)

    assert metrics["turn0_tool_count"] == 5
    assert metrics["used_tools"] == [
        "gmail__search_messages", "google_calendar__create_event",
    ]
    assert metrics["hit_rate"] == 0.5
    assert metrics["misses"] == 2
    assert metrics["missed_tools"] == ["google_calendar__create_event"]


def test_selection_metrics_unknown_without_requests_or_used_tools():
    no_relay = BENCH.selection_metrics([], [{"name": "github__get_repo"}])
    assert no_relay["hit_rate"] is None
    assert no_relay["misses"] is None
    assert no_relay["used_tools"] == ["github__get_repo"]

    only_discovery = BENCH.selection_metrics(
        [{"tool_names": ["tool_search"]}], [{"name": "tool_search", "arguments": {}}],
    )
    assert only_discovery["hit_rate"] is None
    assert only_discovery["misses"] == 0


def test_tool_key_folds_capability_id_provider_name_and_bare_name():
    assert BENCH.tool_key("mcp-benchmark-google-calendar.google_calendar__list_events") == (
        "google_calendar__list_events"
    )
    assert BENCH.tool_key("mcp-benchmark-google-calendar__google_calendar__list_events") == (
        "google_calendar__list_events"
    )
    assert BENCH.tool_key("google_calendar__list_events") == "google_calendar__list_events"
    assert BENCH.tool_key("builtin.result_read") == "builtin__result_read"


def test_aggregate_reports_advertised_size_and_selection_quality():
    config = {"disclosure": "namespaces", "prefetch": "lexical"}

    def observation(tool_counts, tokens, hit_rate, misses):
        return {
            "arm": "prefetch-lexical",
            "config": config,
            "catalog": {"tool_count": 500},
            "task": {"completed": True, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 100},
            "failure": None,
            "advertised": {
                "tool_count_per_request": tool_counts,
                "schema_tokens_per_request": tokens,
            },
            "selection": {
                "turn0_tool_count": tool_counts[0] if tool_counts else None,
                "hit_rate": hit_rate,
                "misses": misses,
            },
        }

    [aggregate] = BENCH.aggregate_observations([
        observation([10, 10], [1500, 1500], 1.0, 0),
        observation([20, 20, 20], [3000, 3000, 3000], 0.5, 2),
        observation([], [], None, None),
    ])

    assert aggregate["config"] == config
    assert aggregate["config_consistent"] is True
    assert aggregate["advertised_requests"] == 5
    assert aggregate["advertised_tool_count_p20_median_p80"] == [10.0, 20.0, 20.0]
    assert aggregate["advertised_tool_count_max"] == 20
    assert aggregate["advertised_schema_tokens_p20_median_p80"] == [1500.0, 3000.0, 3000.0]
    assert aggregate["turn0_tool_count_p20_median_p80"] == [12.0, 15.0, 18.0]
    assert aggregate["selection_hit_rate_observations"] == 2
    assert aggregate["selection_hit_rate_mean"] == 0.75
    assert aggregate["selection_full_hit_observations"] == 1
    assert aggregate["selection_misses_total"] == 2
    assert aggregate["observations_with_selection_misses"] == 1


# --- The Jev selection arm ---------------------------------------------

JEV_KEY = "jev-test-key-never-sent"


def test_prefetch_jev_arm_env_and_recorded_config():
    catalogs = BENCH.generate_catalog(100)
    env, config = BENCH.arm_env(
        "prefetch-jev", catalogs,
        {"TYPESAFE_API_KEY": JEV_KEY, "REBORN_TOOL_PREFETCH_MAX_TOOLS": "30"},
    )

    assert env == {
        "REBORN_TOOL_DISCLOSURE": "namespaces",
        "REBORN_TOOL_PREFETCH": "lexical",
        "REBORN_TOOL_PREFETCH_CLASSIFIER": "jev",
        "REBORN_TOOL_PREFETCH_JEV_MODEL": "jev-latest",
        "REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV": "TYPESAFE_API_KEY",
    }
    assert config["classifier"] == "jev"
    assert config["jev"] == {
        "endpoint": "https://api.typesafe.ai/v1/systemone",
        "model": "jev-latest",
        "api_key_env": "TYPESAFE_API_KEY",
        "timeout_ms": 500,
        "usd_per_million_input_tokens": 0.042,
    }
    assert config["retrieval"] == "native"
    assert config["embeddings"] is None
    assert config["prefetch_tuning"]["REBORN_TOOL_PREFETCH_MAX_TOOLS"] == "30"
    assert JEV_KEY not in BENCH.json.dumps(config)
    assert "REBORN_TOOL_PREFETCH_CLASSIFIER" in BENCH.ARM_CONTROLLED_ENV


def test_prefetch_jev_arm_passes_another_provider_through_and_records_it():
    catalogs = BENCH.generate_catalog(100)
    endpoint = "https://decisions.example.test/api/v1/decisions"
    env, config = BENCH.arm_env("prefetch-jev", catalogs, {
        "REBORN_TOOL_PREFETCH_JEV_ENDPOINT": f" {endpoint} ",
        "REBORN_TOOL_PREFETCH_JEV_MODEL": "jev-1.13.0",
        "REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV": "OTHER_JEV_KEY",
        "OTHER_JEV_KEY": JEV_KEY,
        "TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS": "0.5",
    })

    assert env["REBORN_TOOL_PREFETCH_JEV_ENDPOINT"] == endpoint
    assert env["REBORN_TOOL_PREFETCH_JEV_MODEL"] == "jev-1.13.0"
    assert env["REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV"] == "OTHER_JEV_KEY"
    assert config["jev"] == {
        "endpoint": endpoint,
        "model": "jev-1.13.0",
        "api_key_env": "OTHER_JEV_KEY",
        "timeout_ms": 500,
        "usd_per_million_input_tokens": 0.5,
    }
    assert JEV_KEY not in BENCH.json.dumps(config)
    assert JEV_KEY not in BENCH.json.dumps(env)

    # Credentials in the endpoint reach the server, which refuses them, but
    # never the record.
    _, config = BENCH.arm_env("prefetch-jev", catalogs, {
        "REBORN_TOOL_PREFETCH_JEV_ENDPOINT": (
            "https://user:hunter2@decisions.example.test/v1?token=hunter2"
        ),
        "TYPESAFE_API_KEY": JEV_KEY,
    })
    assert config["jev"]["endpoint"] == "https://decisions.example.test/v1"


@pytest.mark.parametrize("price", ["free", "-1", "nan"])
def test_jev_arm_refuses_a_price_that_is_not_a_price(price):
    with pytest.raises(ValueError, match="TOOL_BENCHMARK_JEV_USD_PER_MILLION"):
        BENCH.arm_env("prefetch-jev", BENCH.generate_catalog(100), {
            "TYPESAFE_API_KEY": JEV_KEY,
            "TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS": price,
        })


def test_prefetch_jev_arm_is_opt_in_and_local_arms_record_no_classifier():
    assert "prefetch-jev" in BENCH.ARMS
    assert "prefetch-jev" not in BENCH.DEFAULT_ARMS
    assert set(BENCH.DEFAULT_ARMS) == set(BENCH.ARMS) - {"prefetch-jev"}

    catalogs = BENCH.generate_catalog(100)
    for arm in ("prefetch-lexical", "prefetch-lexical-floor"):
        env, config = BENCH.arm_env(arm, catalogs, {"TYPESAFE_API_KEY": JEV_KEY})
        assert "REBORN_TOOL_PREFETCH_CLASSIFIER" not in env
        assert "REBORN_TOOL_PREFETCH_JEV_MODEL" not in env
        assert "classifier" not in config and "jev" not in config


@pytest.mark.parametrize("env", [
    {},
    {"TYPESAFE_API_KEY": "  "},
    # The key sits in the default variable, but the override names another.
    {"TYPESAFE_API_KEY": JEV_KEY, "REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV": "OTHER_JEV_KEY"},
])
def test_jev_arm_refuses_without_its_key(env):
    name = env.get("REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV", "TYPESAFE_API_KEY")
    with pytest.raises(ValueError, match=f"{name} is not set.*never falls back"):
        BENCH.arm_env("prefetch-jev", BENCH.generate_catalog(100), env)


def test_jev_task_group_fails_before_starting_the_server(tmp_path, monkeypatch):
    monkeypatch.delenv("TYPESAFE_API_KEY", raising=False)
    live_qa = _NoServerLiveQa()
    task = next(task for task in BENCH.TASKS if task["id"] == "no-match")

    with pytest.raises(ValueError, match="TYPESAFE_API_KEY"):
        asyncio.run(BENCH.run_task_group(
            live_qa, tmp_path / "ironclaw", tmp_path, "prefetch-jev", 100,
            task, [0], tmp_path / "observations.jsonl",
        ))

    assert live_qa.started == []
    assert not (tmp_path / "cases").exists()


def test_run_refuses_the_jev_arm_up_front_and_strips_an_exported_classifier(
    tmp_path, monkeypatch,
):
    monkeypatch.delenv("TYPESAFE_API_KEY", raising=False)
    monkeypatch.setenv("NEARAI_API_KEY", "test-key")
    monkeypatch.setenv("REBORN_TOOL_PREFETCH_CLASSIFIER", "jev")
    binary = tmp_path / "ironclaw"
    binary.write_text("")
    live_qa = _NoServerLiveQa()
    monkeypatch.setattr(BENCH, "_load_live_qa", lambda: live_qa)
    args = BENCH.argparse.Namespace(
        output_dir=tmp_path / "out", binary=binary,
        arm=["prefetch-lexical", "prefetch-jev"], tool_count=[100],
        task=None, repetitions=1, no_request_recorder=True,
    )

    with pytest.raises(RuntimeError, match="arm prefetch-jev: .*TYPESAFE_API_KEY"):
        asyncio.run(BENCH.async_main(args))

    assert live_qa.started == []
    assert "REBORN_TOOL_PREFETCH_CLASSIFIER" not in BENCH.os.environ


def test_selection_log_env_adds_the_selection_target_at_debug():
    target = "ironclaw::reborn::tool_prefetch"
    assert BENCH.selection_log_env({}) == {
        "IRONCLAW_REBORN_LOG": f"info,{target}=debug",
    }
    assert BENCH.selection_log_env({"IRONCLAW_REBORN_LOG": " "}) == {
        "IRONCLAW_REBORN_LOG": f"info,{target}=debug",
    }
    assert BENCH.selection_log_env(
        {"IRONCLAW_REBORN_LOG": f"warn,ironclaw_webui=info,{target}=error"}
    ) == {"IRONCLAW_REBORN_LOG": f"warn,ironclaw_webui=info,{target}=debug"}


def _ansi(text):
    """Colour a line the way `tracing`'s fmt layer does on a terminal."""
    return text.replace(
        "ironclaw::reborn::tool_prefetch: ",
        "\x1b[2mironclaw::reborn::tool_prefetch\x1b[0m\x1b[2m:\x1b[0m ",
    ).replace("slices=", "\x1b[3mslices\x1b[0m\x1b[2m=\x1b[0m")


def _selected_line(latency_ms, *, classifier="jev", fallback=""):
    scorer = "" if fallback else "jev:jev-latest"
    chosen = "[]" if fallback else '[("google_calendar__list_events", 0.91)]'
    return (
        "2026-09-28T10:00:01.000000Z DEBUG run{turn=1}: "
        "ironclaw::reborn::tool_prefetch: selected the conversation's tools "
        f'from its opening request classifier="{classifier}" scorer="{scorer}" '
        f'fallback="{fallback}" advertised_tool_count=9 est_schema_tokens=1800 '
        f'floor=["tool_search", "tool_describe", "tool_call", "result_read"] '
        f"chosen={chosen} latency_ms={latency_ms}"
    )


def _jev_scored_line(slices, latency_ms, input_tokens):
    return (
        "2026-09-28T10:00:00.900000Z DEBUG run{turn=1}: "
        "ironclaw::reborn::tool_prefetch: Jev scored the candidate tools "
        'classifier="jev" model=jev-latest served_model=jev-1.13.0 '
        'chosen=[("google_calendar__list_events", 0.91), ("gmail__send", 0.4)] '
        f"first_left_out=Some(0.12) slices={slices} latency_ms={latency_ms} "
        f"input_tokens={input_tokens}"
    )


def _jev_failed_line(error_kind, slices, latency_ms):
    return (
        "2026-09-28T10:00:00.900000Z DEBUG ironclaw::reborn::tool_prefetch: "
        f'Jev tool classification failed classifier="jev" model=jev-latest '
        f'error_kind="{error_kind}" slices={slices} latency_ms={latency_ms}'
    )


def test_selection_log_metrics_read_jev_latency_slices_tokens_and_cost():
    text = "\n".join([
        "2026-09-28T10:00:00.1Z  INFO ironclaw_webui: request handled latency_ms=3",
        _ansi(_jev_scored_line(3, 412, 52_000)),
        _ansi(_selected_line(430)),
        "2026-09-28T10:00:02.0Z DEBUG ironclaw::reborn::tool_prefetch: "
        "serving the conversation's recorded tool selection reason=\"initial\"",
    ])

    metrics = BENCH.selection_log_metrics(text)

    assert metrics == {
        "selections": 1,
        "classifier": "jev",
        "turn0_latency_ms": 430,
        "core_set_fallbacks": 0,
        "fallback_reasons": [],
        "jev": {
            "model": "jev-latest",
            "served_models": ["jev-1.13.0"],
            "classifications": 1,
            "failures": [],
            "slices": 3,
            "input_tokens": 52_000,
            "cost_usd": 0.002184,
            "latency_ms": [412],
        },
        "index": None,
    }


def test_selection_log_metrics_count_the_core_set_fallback():
    text = "\n".join([
        _jev_failed_line("timeout", 2, 501),
        _selected_line(503, fallback="timeout"),
    ])

    metrics = BENCH.selection_log_metrics(text)

    assert metrics["selections"] == 1
    assert metrics["turn0_latency_ms"] == 503
    assert metrics["core_set_fallbacks"] == 1
    assert metrics["fallback_reasons"] == ["timeout"]
    assert metrics["jev"]["failures"] == ["timeout"]
    assert metrics["jev"]["slices"] == 2
    # A failure line carries no token count, so neither tokens nor cost is known.
    assert metrics["jev"]["input_tokens"] is None
    assert metrics["jev"]["cost_usd"] is None


def test_selection_log_metrics_for_a_local_arm_and_an_empty_log():
    local = BENCH.selection_log_metrics(_selected_line(4, classifier="local"))
    assert local["classifier"] == "local"
    assert local["turn0_latency_ms"] == 4
    assert local["jev"] is None

    assert BENCH.selection_log_metrics("") == {
        "selections": 0, "classifier": None, "turn0_latency_ms": None,
        "core_set_fallbacks": 0, "fallback_reasons": [], "jev": None,
        "index": None,
    }


def _index_line(stored, loaded, embedded, missing, dense_fallback="none"):
    return (
        "2026-09-28T10:00:00.800000Z DEBUG run{turn=1}: "
        "ironclaw::reborn::tool_prefetch: tool index vectors at selection time "
        'ranker_version="hybrid-rrf-v1(bounded-bm25f-v1,dense-cosine-v1)" '
        f"documents={stored + embedded + missing} stored={stored} loaded={loaded} "
        f'embedded={embedded} missing={missing} dense_fallback="{dense_fallback}"'
    )


def test_selection_log_metrics_read_the_turn0_index_vectors():
    text = "\n".join([
        _ansi(_index_line(120, 120, 0, 0)),
        _ansi(_selected_line(40, classifier="local")),
        # A later turn's fit is not the turn-0 figure.
        _index_line(118, 0, 2, 0),
    ])

    metrics = BENCH.selection_log_metrics(text)

    assert metrics["index"] == {
        "documents": 120, "stored": 120, "loaded": 120, "embedded": 0,
        "missing": 0, "dense_fallback": None,
    }
    timed_out = BENCH.selection_log_metrics(_index_line(0, 0, 0, 120, "timeout"))
    assert timed_out["index"]["missing"] == 120
    assert timed_out["index"]["dense_fallback"] == "timeout"


def test_aggregate_reports_index_vectors_at_selection():
    config = {"disclosure": "namespaces", "prefetch": "semantic", "classifier": "local"}

    def observation(selection_log):
        return {
            "arm": "prefetch-semantic",
            "config": config,
            "catalog": {"tool_count": 120},
            "task": {"completed": True, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 100},
            "failure": None,
            "selection_log": selection_log,
        }

    [summary] = BENCH.aggregate_observations([
        observation(BENCH.selection_log_metrics(_index_line(120, 120, 0, 0))),
        observation(BENCH.selection_log_metrics(_index_line(100, 0, 20, 0))),
        observation(BENCH.selection_log_metrics(_index_line(0, 0, 0, 120, "timeout"))),
        observation(BENCH.selection_log_metrics("")),
    ])

    assert summary["index_observations"] == 3
    assert summary["index_embedded_at_selection_total"] == 20
    assert summary["index_missing_at_selection_total"] == 120
    assert summary["index_dense_fallback_rate"] == 0.3333
    assert summary["index_dense_fallback_reasons"] == {"timeout": 1}


def test_read_log_since_returns_only_the_appended_text(tmp_path):
    path = tmp_path / "server.log"
    assert BENCH.log_size(path) == 0
    assert BENCH.read_log_since(path, 0) == ""
    path.write_text("before\n")
    offset = BENCH.log_size(path)
    with path.open("a") as handle:
        handle.write("after\n")
    assert BENCH.read_log_since(path, offset) == "after\n"


def test_aggregate_reports_selection_latency_fallbacks_and_jev_usage():
    config = {"disclosure": "namespaces", "prefetch": "lexical", "classifier": "jev"}

    def observation(arm, selection_log):
        return {
            "arm": arm,
            "config": config,
            "catalog": {"tool_count": 500},
            "task": {"completed": True, "unauthorized_tool_leaks": 0},
            "latency_ms": {"end_to_end": 100},
            "failure": None,
            "selection_log": selection_log,
        }

    scored = BENCH.selection_log_metrics(
        "\n".join([_jev_scored_line(2, 300, 40_000), _selected_line(310)])
    )
    scored_more = BENCH.selection_log_metrics(
        "\n".join([_jev_scored_line(4, 600, 80_000), _selected_line(620)])
    )
    fell_back = BENCH.selection_log_metrics(
        "\n".join([
            _jev_failed_line("rate_limited", 4, 480),
            _selected_line(490, fallback="rate_limited"),
        ])
    )
    # Aggregates come sorted by arm name.
    off, jev = BENCH.aggregate_observations([
        observation("prefetch-jev", scored),
        observation("prefetch-jev", scored_more),
        observation("prefetch-jev", fell_back),
        observation("prefetch-jev", BENCH.selection_log_metrics("")),
        observation("namespaces", None),
    ])

    assert jev["arm"] == "prefetch-jev"
    assert jev["selection_log_observations"] == 3
    assert jev["turn0_selection_latency_ms_p20_median_p80"] == [382.0, 490.0, 568.0]
    assert jev["turn0_selection_latency_ms_max"] == 620
    assert jev["core_set_fallback_observations"] == 1
    assert jev["core_set_fallback_rate"] == 0.3333
    assert jev["core_set_fallback_reasons"] == {"rate_limited": 1}
    assert jev["jev_observations"] == 3
    assert jev["jev_models"] == ["jev-1.13.0"]
    assert jev["jev_slices_per_conversation_p20_median_p80"] == [2.8, 4.0, 4.0]
    assert jev["jev_slices_per_conversation_max"] == 4
    assert jev["jev_input_tokens_total"] == 120_000
    assert jev["jev_input_tokens_mean"] == 60_000
    assert jev["jev_cost_usd_total"] == 0.00504
    assert jev["jev_cost_usd_per_conversation_mean"] == 0.00252

    assert off["arm"] == "namespaces"
    assert off["selection_log_observations"] == 0
    assert off["turn0_selection_latency_ms_p20_median_p80"] is None
    assert off["core_set_fallback_rate"] is None
    assert off["jev_observations"] == 0
    assert off["jev_input_tokens_total"] is None
    assert off["jev_cost_usd_total"] is None


class _ScriptedJevLiveQa:
    """Live-QA stand-in whose "server" writes Jev selection lines to its log.

    Nothing leaves the process: no server, browser or Jev provider call. Each
    chat case appends the lines the real server would for one conversation.
    """

    def __init__(self, per_case_lines):
        self.per_case_lines = list(per_case_lines)
        self.server_env = None
        self.log_path = None

    def create_generated_reborn_home(self, path):
        return path

    def case_llm_trace_env(self, *_args):
        return {}

    async def start_reborn_server(self, _binary, _home, case_dir, extra_env):
        self.server_env = dict(extra_env)
        case_dir.mkdir(parents=True, exist_ok=True)
        self.log_path = case_dir / BENCH.SERVER_STDERR_LOG
        with self.log_path.open("a") as handle:
            handle.write("2026-09-28T10:00:00Z  INFO ironclaw: serving\n")
        return object(), "http://127.0.0.1:9"

    class LiveQaContext:
        def __init__(self, **kwargs):
            self.__dict__.update(kwargs)

    async def _live_chat_case(self, _ctx, **_kwargs):
        with self.log_path.open("a") as handle:
            handle.write("\n".join(self.per_case_lines.pop(0)) + "\n")
        return type("Result", (), {"success": True})()

    def stop_process(self, _proc):
        return None


def test_jev_task_group_records_each_repetitions_own_selection_log(
    tmp_path, monkeypatch,
):
    monkeypatch.setenv("TYPESAFE_API_KEY", JEV_KEY)
    monkeypatch.setenv("TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS", "1.0")
    for name in ("REBORN_TOOL_PREFETCH_JEV_ENDPOINT", "REBORN_TOOL_PREFETCH_JEV_MODEL",
                 "REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.delenv("IRONCLAW_REBORN_LOG", raising=False)
    monkeypatch.setattr(BENCH, "install_catalog", lambda *_args: ["p"] * 20)
    live_qa = _ScriptedJevLiveQa([
        [_jev_scored_line(1, 200, 10_000), _selected_line(210)],
        [
            _jev_failed_line("unauthorized", 1, 90),
            _selected_line(95, fallback="unauthorized"),
        ],
    ])
    task = next(task for task in BENCH.TASKS if task["id"] == "natural-language-alias")
    observations_path = tmp_path / "observations.jsonl"

    first, second = asyncio.run(BENCH.run_task_group(
        live_qa, tmp_path / "ironclaw", tmp_path, "prefetch-jev", 100,
        task, [0, 1], observations_path,
    ))

    assert live_qa.server_env["REBORN_TOOL_PREFETCH_CLASSIFIER"] == "jev"
    assert live_qa.server_env["IRONCLAW_REBORN_LOG"] == (
        "info,ironclaw::reborn::tool_prefetch=debug"
    )
    assert first["config"]["jev"]["model"] == "jev-latest"
    assert first["selection_log"]["jev"]["served_models"] == ["jev-1.13.0"]
    assert second["selection_log"]["jev"]["served_models"] == []
    assert first["selection_log"]["turn0_latency_ms"] == 210
    assert first["selection_log"]["jev"]["input_tokens"] == 10_000
    # Priced at the recorded (overridden) price, not the default.
    assert first["config"]["jev"]["usd_per_million_input_tokens"] == 1.0
    assert first["selection_log"]["jev"]["cost_usd"] == 0.01
    assert first["selection_log"]["core_set_fallbacks"] == 0
    assert second["selection_log"]["turn0_latency_ms"] == 95
    assert second["selection_log"]["fallback_reasons"] == ["unauthorized"]
    assert second["selection_log"]["jev"]["input_tokens"] is None
    assert JEV_KEY not in observations_path.read_text()


def test_jev_defaults_and_parsed_log_messages_match_the_rust_sources():
    config_src = (
        BENCH.ROOT / "crates/app/ironclaw_config/src/tool_prefetch.rs"
    ).read_text(encoding="utf-8")
    assert f'DEFAULT_JEV_ENDPOINT: &str = "{BENCH.JEV_ENDPOINT}"' in config_src
    assert f'DEFAULT_JEV_MODEL: &str = "{BENCH.JEV_MODEL}"' in config_src
    for name in (BENCH.JEV_ENDPOINT_OVERRIDE_ENV, BENCH.JEV_MODEL_OVERRIDE_ENV,
                 BENCH.JEV_API_KEY_ENV_OVERRIDE_ENV):
        assert BENCH.re.search(rf'_ENV: &str =\s*"{name}";', config_src), name
    assert f'DEFAULT_JEV_API_KEY_ENV: &str = "{BENCH.JEV_API_KEY_ENV}"' in config_src
    assert f"DEFAULT_JEV_TIMEOUT_MS: u64 = {BENCH.JEV_TIMEOUT_MS};" in config_src

    loop_host = (
        BENCH.ROOT / "crates/loop/ironclaw_loop_host/src/tool_prefetch.rs"
    ).read_text(encoding="utf-8")
    jev = (
        BENCH.ROOT / "crates/extensions/packages/tool-selection-jev/src/classifier.rs"
    ).read_text(encoding="utf-8")
    assert f'"{BENCH.SELECTION_LOG_TARGET}"' in loop_host
    assert f'"{BENCH.SELECTION_LOG_TARGET}"' in jev
    assert f'"{BENCH.SELECTION_LOG_SELECTED}"' in loop_host
    for field in ("classifier =", "fallback =", "latency_ms,"):
        assert field in loop_host
    disclosure = (
        BENCH.ROOT / "crates/loop/ironclaw_loop_host/src/tool_disclosure_port.rs"
    ).read_text(encoding="utf-8")
    assert f'"{BENCH.SELECTION_LOG_INDEX_VECTORS}"' in disclosure
    for field in ("stored = report.stored", "loaded = report.loaded",
                  "embedded = report.embedded", "missing = report.missing",
                  'dense_fallback = report.dense_fallback.unwrap_or("none")'):
        assert field in disclosure
    assert f'"{BENCH.SELECTION_LOG_JEV_SCORED}"' in jev
    assert f'"{BENCH.SELECTION_LOG_JEV_FAILED}"' in jev
    for field in ("model = %self.model", "served_model = %", "slices = slices.len()",
                  "input_tokens,",
                  "error_kind = error.kind_label()"):
        assert field in jev

    cli = (
        BENCH.ROOT / "crates/app/ironclaw_cli/src/runtime/mod.rs"
    ).read_text(encoding="utf-8")
    # The server's stderr filter, which the selection arms extend.
    assert f'"{BENCH.SERVER_LOG_FILTER_ENV}",' in cli
    live_qa = BENCH.LIVE_QA_PATH.read_text(encoding="utf-8")
    assert f'"{BENCH.SERVER_STDERR_LOG}"' in live_qa


# --- Two-turn topic-drift task -------------------------------------------


def _task(task_id):
    return next(task for task in BENCH.TASKS if task["id"] == task_id)


GITHUB_CALL = {
    "name": "github__list_issues",
    "arguments": {"owner": "nearai", "repo": "ironclaw", "state": "open"},
}
CALENDAR_CALL = {
    "name": "google_calendar__create_event",
    "arguments": {
        "schedule": {
            "start_at": "2026-08-14T15:00:00Z", "end_at": "2026-08-14T15:30:00Z",
        },
    },
}
TURN0_TOOLS = [
    "tool_search", "tool_describe", "tool_call", "builtin__result_read",
    "mcp-benchmark-github__github__list_issues",
]
RESELECTED_TOOLS = [
    "tool_search", "tool_describe", "tool_call", "builtin__result_read",
    "mcp-benchmark-google-calendar__google_calendar__create_event",
]


def _request(signature, names):
    return {
        "tools_signature": signature, "tool_names": names, "tool_count": len(names),
        "tool_schema_tokens": 100 * len(names),
    }


def _mark(calls, requests, trace_calls, monotonic):
    return {
        "calls": calls, "requests": requests, "trace_calls": trace_calls,
        "monotonic": monotonic,
    }


def _drift_observation_parts(turn1_signature="A", turn1_tools=TURN0_TOOLS):
    """Synthetic fixture calls, relay requests and trace calls for both turns.

    Turn 0 calls the GitHub tool directly 300 ms after it starts. Turn 1
    starts at t=20 s, finds Calendar with `tool_search` and calls it through
    the `tool_call` bridge 1.5 s later.
    """
    calls = [
        {**GITHUB_CALL, "monotonic_ns": 10_300_000_000},
        {**CALENDAR_CALL, "monotonic_ns": 21_500_000_000},
    ]
    requests = [
        _request("A", TURN0_TOOLS), _request("A", TURN0_TOOLS),
        _request(turn1_signature, turn1_tools),
        {
            "tools_signature": None, "tool_names": None, "tool_count": None,
            "tool_schema_tokens": None,
        },
        _request(turn1_signature, turn1_tools),
        _request(turn1_signature, turn1_tools),
    ]
    trace_calls = [
        {"name": "mcp-benchmark-github__github__list_issues", "arguments": {}},
        {"name": "tool_search", "arguments": {"query": "calendar"}},
        {
            "name": "tool_call",
            "arguments": {
                "name": "mcp-benchmark-google-calendar.google_calendar__create_event",
            },
        },
    ]
    marks = [
        {"start": _mark(0, 0, 0, 10.0), "end": _mark(1, 2, 1, 11.0)},
        {"start": _mark(1, 2, 1, 20.0), "end": _mark(2, 6, 3, 23.0)},
    ]
    return marks, calls, requests, trace_calls


def test_topic_drift_task_moves_to_a_namespace_turn_one_never_mentions():
    drift = _task("topic-drift")
    first, second = drift["turns"]

    assert first["expected"] == ("github__list_issues",)
    assert second["expected"] == ("google_calendar__create_event",)
    assert drift["expected"] == first["expected"] + second["expected"]
    assert "calendar" not in first["prompt"].lower()
    assert "github" not in second["prompt"].lower()
    assert "topic-drift" in BENCH.DEFAULT_TASK_IDS
    assert "server_env" not in drift and "idle_gap_seconds" not in drift


def test_topic_drift_idle_variant_is_opt_in_and_waits_seconds_not_minutes():
    idle = _task("topic-drift-idle")
    env = idle["server_env"]
    lifetime = int(env["REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS"])

    assert "topic-drift-idle" not in BENCH.DEFAULT_TASK_IDS
    assert idle["turns"] == _task("topic-drift")["turns"]
    assert env["REBORN_TOOL_PREFETCH_RESELECT"] == "on"
    assert int(env["REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS"]) < lifetime
    assert lifetime < idle["idle_gap_seconds"] <= 30
    assert set(env) <= set(BENCH.TASK_CONTROLLED_ENV)


def test_task_turns_marks_each_turn_and_keeps_single_turn_prompts():
    catalogs = BENCH.generate_catalog(100)
    exact = BENCH.task_turns(_task("exact-canonical-id"), catalogs, "off-100-x-0")
    drift = BENCH.task_turns(_task("topic-drift"), catalogs, "off-100-topic-drift-0")

    assert exact == [(
        _task("exact-canonical-id")["prompt"].format(
            canonical="mcp-benchmark-github.github__get_pull_request"
        ),
        "BENCHMARK_DONE_off_100_x_0",
    )]
    assert [marker for _, marker in drift] == [
        "BENCHMARK_DONE_off_100_topic_drift_0_T1",
        "BENCHMARK_DONE_off_100_topic_drift_0_T2",
    ]
    assert [prompt for prompt, _ in drift] == [
        turn["prompt"] for turn in _task("topic-drift")["turns"]
    ]


def test_topic_drift_scoring_checks_order_and_each_turns_arguments():
    drift = _task("topic-drift")

    assert BENCH.score_task(drift, [GITHUB_CALL, CALENDAR_CALL], [])["completed"]
    assert not BENCH.score_task(drift, [CALENDAR_CALL, GITHUB_CALL], [])["completed"]
    wrong_time = {
        **CALENDAR_CALL,
        "arguments": {"schedule": {
            "start_at": "2026-08-14T16:00:00Z", "end_at": "2026-08-14T16:30:00Z",
        }},
    }
    assert not BENCH.score_task(drift, [GITHUB_CALL, wrong_time], [])["completed"]
    assert BENCH.score_turn(drift, 0, [GITHUB_CALL])["completed"]
    assert not BENCH.score_turn(drift, 1, [wrong_time])["completed"]
    assert BENCH.score_turn(drift, 1, [wrong_time])["correct_tool_recalled"]
    wrong_repo = {**GITHUB_CALL, "arguments": {"owner": "nearai", "repo": "other"}}
    assert not BENCH.score_turn(drift, 0, [wrong_repo])["completed"]


def test_turn_metrics_splits_calls_misses_and_latency_by_turn():
    marks, calls, requests, trace_calls = _drift_observation_parts()

    first, second = BENCH.turn_metrics(
        _task("topic-drift"), marks, calls, requests, trace_calls,
    )

    assert first["turn"] == 0 and first["started"] and first["replied"]
    assert first["completed"]
    assert first["tool_calls"] == 1
    assert first["synthetic_tool_calls"] == 1
    assert first["tool_search_calls"] == 0
    assert first["bridged_tool_calls"] == 0
    assert first["selection"]["misses"] == 0
    assert first["selection"]["hit_rate"] == 1.0
    assert first["selection"]["turn_start_tool_count"] == len(TURN0_TOOLS)
    assert first["time_to_first_correct_tool_call_ms"] == 300
    assert first["tool_bearing_model_requests"] == 2
    assert first["tool_definition_signature_changes"] == 0
    assert first["tools_changed_at_turn_start"] is None

    # The drift: Calendar was not in the turn-0 selection, so turn 1 reaches
    # it through tool_search and the tool_call bridge, and that is a miss.
    assert second["completed"]
    assert second["tool_calls"] == 2
    assert second["synthetic_tool_calls"] == 1
    assert second["tool_search_calls"] == 1
    assert second["bridged_tool_calls"] == 1
    assert second["selection"]["misses"] == 1
    assert second["selection"]["missed_tools"] == ["google_calendar__create_event"]
    assert second["selection"]["hit_rate"] == 0.0
    # Measured from the start of turn 1, not from the start of the conversation.
    assert second["time_to_first_correct_tool_call_ms"] == 1500
    assert second["tool_bearing_model_requests"] == 3
    assert second["tool_definition_signature_changes"] == 0
    assert second["tools_changed_at_turn_start"] is False
    assert BENCH.tool_definition_signature_changes(requests) == 0


def test_turn_metrics_sees_a_reselection_at_the_turn_boundary():
    marks, calls, requests, trace_calls = _drift_observation_parts(
        turn1_signature="B", turn1_tools=RESELECTED_TOOLS,
    )

    first, second = BENCH.turn_metrics(
        _task("topic-drift-idle"), marks, calls, requests, trace_calls,
    )

    assert first["tool_definition_signature_changes"] == 0
    assert second["tool_definition_signature_changes"] == 0
    assert second["tools_changed_at_turn_start"] is True
    assert second["selection"]["misses"] == 0
    assert second["selection"]["hit_rate"] == 1.0
    # The conversation-wide count sees the one change, at the boundary.
    assert BENCH.tool_definition_signature_changes(requests) == 1


def test_turn_metrics_reports_an_unreplied_turn_and_one_never_sent():
    calls = [{**GITHUB_CALL, "monotonic_ns": 10_300_000_000}]
    trace_calls = [{"name": "github__list_issues", "arguments": {}}]
    marks = [{"start": _mark(0, 0, 0, 10.0), "end": None}]

    first, second = BENCH.turn_metrics(
        _task("topic-drift"), marks, calls, [_request("A", TURN0_TOOLS)], trace_calls,
    )

    assert first["started"] and not first["replied"]
    assert first["tool_calls"] == 1
    assert first["completed"]
    assert second["started"] is False and second["replied"] is False
    assert not second["completed"]
    assert second["tool_calls"] is None
    assert second["selection"] is None
    assert second["time_to_first_correct_tool_call_ms"] is None
    assert second["expected_tools"] == ["google_calendar__create_event"]


def test_turn_metrics_unknown_without_relay_or_readable_trace_boundary():
    marks, calls, _requests, trace_calls = _drift_observation_parts()
    marks[0]["end"]["trace_calls"] = None
    marks[1]["start"]["trace_calls"] = None

    first, second = BENCH.turn_metrics(
        _task("topic-drift"), marks, calls, None, trace_calls,
    )

    for turn in (first, second):
        assert turn["tool_calls"] is None
        assert turn["tool_search_calls"] is None
        assert turn["selection"] is None
        assert turn["tool_definition_signature_changes"] is None
        assert turn["tool_bearing_model_requests"] is None
        assert turn["tools_changed_at_turn_start"] is None
    # Fixture calls and their timing do not depend on the relay or trace.
    assert first["synthetic_tool_calls"] == 1
    assert second["time_to_first_correct_tool_call_ms"] == 1500


def test_aggregate_turns_reports_per_turn_metrics_for_multi_turn_tasks_only():
    marks, calls, requests, trace_calls = _drift_observation_parts()
    turns = BENCH.turn_metrics(_task("topic-drift"), marks, calls, requests, trace_calls)
    unreplied = BENCH.turn_metrics(
        _task("topic-drift"),
        [{"start": _mark(0, 0, 0, 10.0), "end": None}],
        calls[:1], requests[:2], trace_calls[:1],
    )

    def observation(turns, signature_changes):
        return {
            "arm": "prefetch-lexical",
            "catalog": {"tool_count": 500},
            "task": {"id": "topic-drift"},
            "cache": {"tool_definition_signature_changes": signature_changes},
            "turns": turns,
            "idle_gap": None,
        }

    single_turn = {
        "arm": "prefetch-lexical", "catalog": {"tool_count": 500},
        "task": {"id": "no-match"}, "turns": None,
    }
    [aggregate] = BENCH.aggregate_turns([
        observation(turns, 0), observation(turns, 0), observation(unreplied, 0),
        single_turn,
    ])

    assert aggregate["task"] == "topic-drift"
    assert aggregate["observations"] == 3
    assert aggregate["idle_gap_seconds"] is None
    assert aggregate["tool_definition_signature_changes_total"] == 0
    assert aggregate["observations_with_tool_definition_changes"] == 0
    first, second = aggregate["turns"]
    assert first["turn"] == 0 and first["started"] == 3 and first["replied"] == 2
    assert first["completion_rate"] == 1.0
    assert first["selection_misses_total"] == 0
    assert first["time_to_first_correct_tool_ms_median"] == 300
    assert second["started"] == 2 and second["replied"] == 2
    assert second["completion_rate"] == pytest.approx(2 / 3)
    assert second["tool_calls_total"] == 4
    assert second["tool_search_calls_total"] == 2
    assert second["bridged_tool_calls_total"] == 2
    assert second["selection_misses_total"] == 2
    assert second["observations_with_selection_misses"] == 2
    assert second["first_correct_tool_observations"] == 2
    assert second["time_to_first_correct_tool_ms_median"] == 1500
    assert second["tool_definition_signature_changes_total"] == 0
    assert second["tools_changed_at_turn_start_observations"] == 0


# --- Driving `run_task_group` through a scripted live-QA stand-in -----------


class _FakeScriptedFollowUp:
    def __init__(self, prompt, marker, required_text=()):
        self.prompt = prompt
        self.marker = marker
        self.required_text = required_text


class _ScriptedLiveQa:
    """Live-QA stand-in that plays each turn's side effects synthetically.

    For every user message it records the fixture call, the relay requests
    and the trace step that a real model turn would produce, then awaits
    the turn-boundary hook exactly as `_live_chat_case` does.
    """

    ScriptedFollowUp = _FakeScriptedFollowUp

    def __init__(self, recorder, turn_effects):
        self.recorder = recorder
        self.turn_effects = turn_effects
        self.fixture = None
        self.chat_kwargs = []
        self.server_env = None
        self.trace_path = None

    def LiveQaContext(self, **kwargs):  # noqa: N802 - mirrors the live-QA class
        return BENCH.argparse.Namespace(**kwargs)

    def create_generated_reborn_home(self, path):
        return path

    def case_llm_trace_env(self, output_dir, group_name):
        self.trace_path = output_dir / "llm-traces" / f"{group_name}.json"
        self.trace_path.parent.mkdir(parents=True, exist_ok=True)
        return {}

    async def start_reborn_server(self, _binary, _home, _case_dir, extra_env):
        self.server_env = dict(extra_env)
        return object(), "http://127.0.0.1:9"

    def stop_process(self, _proc):
        return None

    def parse_case_llm_trace_metrics(self, trace_path):
        steps = BENCH.json.loads(trace_path.read_text())["steps"]
        return {
            "model_call_count": len(steps),
            "tool_call_count": sum(len(step["response"]["tool_calls"]) for step in steps),
            "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": None,
            "uncached_input_tokens": None,
        }

    def _play(self, turn):
        effects = self.turn_effects[turn]
        for call in effects["fixture_calls"]:
            self.fixture.calls.append({**call, "monotonic_ns": BENCH.time.monotonic_ns()})
        self.recorder.requests.extend(effects["requests"])
        steps = (
            BENCH.json.loads(self.trace_path.read_text())["steps"]
            if self.trace_path.exists() else []
        )
        steps.append({"response": {"type": "tool_calls", "tool_calls": effects["trace_calls"]}})
        self.trace_path.write_text(BENCH.json.dumps({"steps": steps}))

    async def _live_chat_case(self, _ctx, **kwargs):
        self.chat_kwargs.append(kwargs)
        follow_ups = kwargs.get("scripted_follow_ups") or []
        on_turn_complete = kwargs.get("on_turn_complete")
        for turn in range(1 + len(follow_ups)):
            self._play(turn)
            if on_turn_complete is not None:
                await on_turn_complete(turn)
        return BENCH.argparse.Namespace(success=True)


DRIFT_TURN_EFFECTS = [
    {
        "fixture_calls": [GITHUB_CALL],
        "requests": [_request("A", TURN0_TOOLS), _request("A", TURN0_TOOLS)],
        "trace_calls": [{"name": "mcp-benchmark-github__github__list_issues", "arguments": {}}],
    },
    {
        "fixture_calls": [CALENDAR_CALL],
        "requests": [_request("A", TURN0_TOOLS)] * 3,
        "trace_calls": [
            {"name": "tool_search", "arguments": {"query": "calendar"}},
            {
                "name": "tool_call",
                "arguments": {"name": "google_calendar__create_event"},
            },
        ],
    },
]


def _run_scripted_group(tmp_path, monkeypatch, task, turn_effects):
    recorder = BENCH.argparse.Namespace(requests=[])
    live_qa = _ScriptedLiveQa(recorder, turn_effects)

    class CapturingFixture(BENCH.McpFixture):
        def __init__(self, catalogs):
            super().__init__(catalogs)
            live_qa.fixture = self

    monkeypatch.setattr(BENCH, "McpFixture", CapturingFixture)
    monkeypatch.setattr(BENCH, "install_catalog", lambda _url, catalogs: ["p"] * len(catalogs))
    observations = asyncio.run(BENCH.run_task_group(
        live_qa, tmp_path / "ironclaw", tmp_path, "prefetch-lexical", 100,
        task, [0], tmp_path / "observations.jsonl", recorder,
    ))
    return live_qa, observations


def test_run_task_group_sends_the_drift_turns_and_records_per_turn_metrics(
    tmp_path, monkeypatch,
):
    live_qa, [observation] = _run_scripted_group(
        tmp_path, monkeypatch, _task("topic-drift"), DRIFT_TURN_EFFECTS,
    )

    [kwargs] = live_qa.chat_kwargs
    first_marker = "BENCHMARK_DONE_prefetch_lexical_100_topic_drift_0_T1"
    second_marker = "BENCHMARK_DONE_prefetch_lexical_100_topic_drift_0_T2"
    assert kwargs["marker"] == first_marker
    assert kwargs["prompt"].startswith(_task("topic-drift")["turns"][0]["prompt"])
    assert kwargs["prompt"].endswith(first_marker)
    [follow_up] = kwargs["scripted_follow_ups"]
    assert follow_up.marker == second_marker
    assert follow_up.required_text == (second_marker,)
    assert follow_up.prompt.startswith(_task("topic-drift")["turns"][1]["prompt"])
    assert "REBORN_TOOL_PREFETCH_RESELECT" not in live_qa.server_env

    assert observation["schema_version"] == 7
    assert observation["task"]["completed"]
    assert observation["idle_gap"] is None
    assert observation["cache"]["tool_definition_signature_changes"] == 0
    first, second = observation["turns"]
    assert first["replied"] and second["replied"]
    assert first["completed"] and second["completed"]
    assert (first["tool_calls"], second["tool_calls"]) == (1, 2)
    assert (first["tool_search_calls"], second["tool_search_calls"]) == (0, 1)
    assert (first["bridged_tool_calls"], second["bridged_tool_calls"]) == (0, 1)
    assert first["selection"]["misses"] == 0
    assert second["selection"]["missed_tools"] == ["google_calendar__create_event"]
    assert first["tool_bearing_model_requests"] == 2
    assert second["tool_bearing_model_requests"] == 3
    assert second["tools_changed_at_turn_start"] is False
    assert isinstance(second["time_to_first_correct_tool_call_ms"], int)
    assert BENCH.load_observations(tmp_path / "observations.jsonl") == [observation]

    [aggregate] = BENCH.aggregate_turns([observation])
    assert [turn["selection_misses_total"] for turn in aggregate["turns"]] == [0, 1]


def test_run_task_group_ignores_model_requests_of_an_earlier_repetition(
    tmp_path, monkeypatch,
):
    # An earlier repetition whose turn timed out keeps calling the model, with
    # its own (different) tools array, while this repetition runs. Its
    # requests must not count as changes of this conversation's array.
    other = {
        **_request("B", TURN0_TOOLS[:1]),
        "conversation": "BENCHMARK_DONE_prefetch_lexical_100_topic_drift_7",
    }
    mine = {
        **_request("A", TURN0_TOOLS),
        "conversation": "BENCHMARK_DONE_prefetch_lexical_100_topic_drift_0",
    }
    effects = [
        {**DRIFT_TURN_EFFECTS[0], "requests": [mine, other, mine]},
        {**DRIFT_TURN_EFFECTS[1], "requests": [other, mine, other, mine, mine]},
    ]

    _live_qa, [observation] = _run_scripted_group(
        tmp_path, monkeypatch, _task("topic-drift"), effects,
    )

    assert observation["cache"]["tool_definition_signature_changes"] == 0
    assert observation["cache"]["tool_bearing_model_requests"] == 5
    assert observation["advertised"]["tool_count_per_request"] == [len(TURN0_TOOLS)] * 5
    first, second = observation["turns"]
    assert first["tool_bearing_model_requests"] == 2
    assert second["tool_bearing_model_requests"] == 3
    assert first["tool_definition_signature_changes"] == 0
    assert second["tool_definition_signature_changes"] == 0
    assert second["tools_changed_at_turn_start"] is False


def test_run_task_group_idle_variant_waits_and_sets_the_short_cache_lifetime(
    tmp_path, monkeypatch,
):
    # The real gap is a few seconds; shrink it so the test does not sleep.
    idle = {**_task("topic-drift-idle"), "idle_gap_seconds": 0.05}

    live_qa, [observation] = _run_scripted_group(
        tmp_path, monkeypatch, idle, DRIFT_TURN_EFFECTS,
    )

    for name, value in idle["server_env"].items():
        assert live_qa.server_env[name] == value
    assert observation["idle_gap"] == {
        "seconds": 0.05, "server_env": idle["server_env"],
    }
    assert [turn["replied"] for turn in observation["turns"]] == [True, True]


def test_run_task_group_single_turn_tasks_send_no_follow_up(tmp_path, monkeypatch):
    effects = [{
        "fixture_calls": [],
        "requests": [_request("A", TURN0_TOOLS)],
        "trace_calls": [],
    }]

    live_qa, [observation] = _run_scripted_group(
        tmp_path, monkeypatch, _task("no-match"), effects,
    )

    [kwargs] = live_qa.chat_kwargs
    assert "scripted_follow_ups" not in kwargs
    assert "on_turn_complete" not in kwargs
    assert kwargs["marker"] == "BENCHMARK_DONE_prefetch_lexical_100_no_match_0"
    assert observation["turns"] is None
    assert BENCH.aggregate_turns([observation]) == []


def test_run_strips_inherited_task_env_and_skips_opt_in_tasks_by_default(
    tmp_path, monkeypatch,
):
    monkeypatch.setenv("NEARAI_API_KEY", "test-key")
    monkeypatch.setenv("REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS", "1")
    binary = tmp_path / "ironclaw"
    binary.write_text("")
    monkeypatch.setattr(BENCH, "_load_live_qa", lambda: object())
    ran = []

    async def fake_matrix(_live_qa, _args, _arms, _counts, tasks, *_rest):
        ran.extend(task["id"] for task in tasks)

    async def fake_head():
        return "0" * 40

    monkeypatch.setattr(BENCH, "_run_matrix", fake_matrix)
    monkeypatch.setattr(BENCH, "git_head", fake_head)
    args = BENCH.argparse.Namespace(
        output_dir=tmp_path / "out", binary=binary, arm=["off"], tool_count=[100],
        task=None, repetitions=1, no_request_recorder=True,
    )

    asyncio.run(BENCH.async_main(args))

    assert "topic-drift" in ran
    assert "topic-drift-idle" not in ran
    assert "REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS" not in BENCH.os.environ
    summary = BENCH.json.loads((tmp_path / "out" / "summary.json").read_text())
    assert summary["schema_version"] == 7
    assert summary["turn_aggregates"] == []


# --- result_read of a discovery result ---------------------------------------


def _ref_result(call_id, name, ref):
    """A recorded tool result that hands the model a result reference."""
    return {
        "tool_call_id": call_id,
        "name": name,
        "content": json.dumps({
            "detail": {"kind": "result_reference", "result_ref": ref},
            "status": "success",
        }),
    }


def _read(ref, *, bridged=False):
    arguments = {"result_ref": ref, "json_pointer": "/results", "offset": 0}
    if bridged:
        return {
            "id": f"read-{ref}",
            "name": "tool_call",
            "arguments": {
                "name": "builtin.result_read", "arguments": json.dumps(arguments),
            },
        }
    return {"id": f"read-{ref}", "name": "builtin__result_read", "arguments": arguments}


def _trace(*steps):
    return {"steps": [{"response": {"type": "user_input", "content": "x"}}, *steps]}


def _tool_step(calls, results=()):
    step = {"response": {"type": "tool_calls", "tool_calls": list(calls)}}
    if results:
        step["expected_tool_results"] = list(results)
    return step


def _text_step(text, results=()):
    step = {"response": {"type": "text", "content": text}}
    if results:
        step["expected_tool_results"] = list(results)
    return step


def test_trace_tool_calls_resolve_each_result_read_to_the_result_it_reads():
    trace = _trace(
        _tool_step([
            {"id": "s1", "name": "tool_search", "arguments": {"query": "cactus"}},
            {"id": "g1", "name": "mcp-benchmark-github__github__list_issues",
             "arguments": {}},
        ]),
        _tool_step(
            [_read("r-search"), _read("r-search", bridged=True), _read("r-github"),
             _read("r-unknown")],
            [
                _ref_result("s1", "tool_search", "r-search"),
                _ref_result("g1", "mcp-benchmark-github__github__list_issues", "r-github"),
            ],
        ),
        # The read's own result carries the same reference; it must not
        # replace the producer.
        _text_step("done", [
            _ref_result("read-r-search", "builtin__result_read", "r-search"),
        ]),
    )

    calls = BENCH.trace_tool_calls(trace)

    assert [call["name"] for call in calls] == [
        "tool_search", "mcp-benchmark-github__github__list_issues",
        "builtin__result_read", "tool_call", "builtin__result_read",
        "builtin__result_read",
    ]
    assert "reads_result_of" not in calls[0]
    assert [call.get("reads_result_of") for call in calls[2:]] == [
        "tool_search", "tool_search",
        "mcp-benchmark-github__github__list_issues", None,
    ]
    assert calls[2]["model_turn"] == 2


def test_trace_tool_calls_resolve_a_read_recorded_before_its_producer():
    # A timed-out conversation keeps running and its steps land between the
    # next repetition's, so a read can precede the step that records its
    # source.
    trace = _trace(
        _tool_step([_read("r1")]),
        _tool_step([{"id": "d1", "name": "tool_describe", "arguments": {}}]),
        _text_step("done", [_ref_result("d1", "tool_describe", "r1")]),
    )

    assert BENCH.trace_tool_calls(trace)[0]["reads_result_of"] == "tool_describe"


def test_trace_tool_calls_name_a_bridged_producer_by_its_target():
    trace = _trace(
        _tool_step([{
            "id": "b1", "name": "tool_call",
            "arguments": {"name": "mcp-benchmark-hubspot__hubspot__search_contacts",
                          "arguments": "{}"},
        }]),
        _tool_step([_read("r1")], [_ref_result("b1", "tool_call", "r1")]),
    )

    assert BENCH.trace_tool_calls(trace)[1]["reads_result_of"] == (
        "mcp-benchmark-hubspot__hubspot__search_contacts"
    )
    assert BENCH.trace_tool_calls({"steps": "nope"}) == []


def test_no_tool_tasks_score_a_search_then_result_read_as_completed():
    search = {"name": "tool_search", "arguments": {"query": "cactus"}}
    search_read = {
        "name": "builtin__result_read", "arguments": {"result_ref": "r1"},
        "reads_result_of": "tool_search",
    }
    bridged_describe_read = {
        "name": "tool_call",
        "arguments": {"name": "builtin.result_read", "arguments": "{}"},
        "reads_result_of": "tool_describe",
    }
    unresolved_read = {"name": "builtin__result_read", "arguments": {}, "reads_result_of": None}
    real_read = {
        "name": "builtin__result_read", "arguments": {},
        "reads_result_of": "builtin__shell",
    }

    for task in (_task("no-match"), _task("denied-capability")):
        assert BENCH.score_task(task, [], [search, search_read])["completed"]
        assert BENCH.score_task(task, [], [search, bridged_describe_read])["completed"]
        assert BENCH.score_task(task, [], [search, unresolved_read])["completed"]
        # A read of a real tool's result is still a real call.
        assert not BENCH.score_task(task, [], [search, real_read])["completed"]
    # A read never excuses a call that reached the fixture.
    assert not BENCH.score_task(
        _task("no-match"), [{"name": "github__list_issues", "arguments": {}}],
        [search_read],
    )["completed"]


def test_no_tool_scoring_resolves_reads_from_a_real_trace_shape():
    # The shape the prefetch-semantic arm produced: search, read the
    # referenced result, answer.
    trace = _trace(
        _tool_step([{"id": "s1", "name": "tool_search", "arguments": {"limit": 5}}]),
        _tool_step([_read("r1")], [_ref_result("s1", "tool_search", "r1")]),
        _text_step("No such tool exists. BENCHMARK_DONE_x"),
    )

    scored = BENCH.score_task(_task("no-match"), [], BENCH.trace_tool_calls(trace))

    assert scored["completed"]
    assert scored["unauthorized_tool_leaks"] == 0


def test_discovery_result_reads_are_counted_apart_from_all_result_reads():
    calls = [
        {"name": "tool_search", "arguments": {}},
        {"name": "builtin__result_read", "arguments": {}, "reads_result_of": "tool_search"},
        {"name": "builtin__result_read", "arguments": {}, "reads_result_of": None},
        {"name": "builtin__result_read", "arguments": {}, "reads_result_of": "builtin__shell"},
    ]

    assert BENCH.result_read_call_count(calls) == 3
    assert BENCH.discovery_result_read_call_count(calls) == 2
    aggregate = BENCH._efficiency_aggregates([
        {"counts": {"result_read_calls": 3, "discovery_result_read_calls": 2}},
        {"counts": {"result_read_calls": 1}},
    ])
    assert aggregate["result_read_calls_total"] == 4
    assert aggregate["discovery_result_read_calls_total"] == 2


# --- topic-drift failure causes ----------------------------------------------


_JSON_TYPES = {
    "object": dict, "string": str, "array": list, "boolean": bool,
    "integer": int, "number": (int, float),
}


def _matches(schema, value):
    """A validator for the JSON Schema subset the corpus uses.

    Covers `type`, `properties`, `required`, `items` and `oneOf` (exactly
    one branch must match), which is all the drift task's tools need.
    """
    expected = schema.get("type")
    if expected is not None:
        if not isinstance(value, _JSON_TYPES[expected]):
            return False
        if expected in {"integer", "number"} and isinstance(value, bool):
            return False
    if "oneOf" in schema:
        if sum(_matches(branch, value) for branch in schema["oneOf"]) != 1:
            return False
    if isinstance(value, dict):
        if any(name not in value for name in schema.get("required", ())):
            return False
        for name, child in (schema.get("properties") or {}).items():
            if name in value and not _matches(child, value[name]):
                return False
    if isinstance(value, list) and "items" in schema:
        return all(_matches(schema["items"], item) for item in value)
    return True


def _catalog_tool(catalogs, name):
    return next(
        tool for catalog in catalogs for tool in catalog["tools"] if tool["name"] == name
    )


def _corpus_tool(name):
    corpus = json.loads(BENCH.CORPUS_PATH.read_text(encoding="utf-8"))
    return next(tool for tool in corpus["tools"] if tool["name"] == name)


def test_catalog_create_event_accepts_a_correct_schedule():
    correct = CALENDAR_CALL["arguments"]
    recurring = {"schedule": {"recurrence": "RRULE:FREQ=WEEKLY"}}
    raw = _corpus_tool("google_calendar__create_event")["parameters"]
    schema = _catalog_tool(
        BENCH.generate_catalog(100), "google_calendar__create_event"
    )["inputSchema"]

    # The corpus schema as written: both branches match any object, so the
    # correct call fails `oneOf`. This is what the run's validator reported.
    assert not _matches(raw, correct)
    assert not _matches(raw, recurring)
    assert _matches(schema, correct)
    assert _matches(schema, recurring)
    assert not _matches(schema, {"schedule": "2026-08-14T15:00:00Z"})
    # The benchmark repairs its own copy, never the shared corpus file.
    assert "required" not in raw["properties"]["schedule"]["oneOf"][0]


def test_exclusive_one_of_leaves_other_schemas_alone():
    schema = {
        "type": "object",
        "properties": {
            "choice": {"oneOf": [
                {"type": "object", "properties": {"a": {"type": "string"}}},
                {"type": "object", "properties": {"b": {"type": "string"}},
                 "required": []},
                {"type": "string"},
            ]},
            "plain": {"type": "object", "properties": {"c": {"type": "string"}}},
        },
    }

    repaired = BENCH.exclusive_one_of(schema)

    branches = repaired["properties"]["choice"]["oneOf"]
    assert branches[0]["required"] == ["a"]
    assert branches[1]["required"] == []
    assert branches[2] == {"type": "string"}
    assert "required" not in repaired["properties"]["plain"]
    assert "required" not in schema["properties"]["choice"]["oneOf"][0]


def test_every_catalog_one_of_has_exclusive_object_branches():
    for catalog in BENCH.generate_catalog(100):
        for tool in catalog["tools"]:
            pending = [tool["inputSchema"]]
            while pending:
                node = pending.pop()
                if isinstance(node, list):
                    pending.extend(node)
                    continue
                if not isinstance(node, dict):
                    continue
                pending.extend(node.values())
                objects = [
                    branch for branch in node.get("oneOf", ())
                    if branch.get("type") == "object"
                ]
                for first in objects:
                    for second in objects:
                        if first is second:
                            continue
                        # `second` rejects every value `first` requires
                        # less of, so no object satisfies both.
                        assert not set(second.get("required", ())) <= set(
                            first.get("properties", {})
                        ), tool["name"]


def test_fixture_returns_data_the_drift_turns_must_report():
    fixture = BENCH.McpFixture(BENCH.generate_catalog(100))
    github = BENCH.NAMESPACES.index("github")
    response, status = fixture.handle(f"/benchmark/{github}", {
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "github__list_issues", "arguments": GITHUB_CALL["arguments"]},
    })

    text = response["result"]["content"][0]["text"]
    assert status == 200
    assert "open issues in nearai/ironclaw" in text
    assert "#8101" in text
    assert fixture.calls[0]["name"] == "github__list_issues"
    assert "Created event" in BENCH.fixture_result_text("google_calendar__create_event")
    assert "2026-08-12 at 10:00 UTC" in BENCH.fixture_result_text("gmail__search_messages")
    assert BENCH.fixture_result_text("database__query") == (
        "benchmark tool database__query completed"
    )
    fixture.server.server_close()


def test_drift_scoring_allows_reading_the_calendar_before_creating_the_event():
    drift = _task("topic-drift")
    list_events = {"name": "google_calendar__list_events", "arguments": {}}

    assert BENCH.score_turn(drift, 1, [list_events, CALENDAR_CALL])["completed"]
    assert BENCH.score_task(
        drift, [GITHUB_CALL, list_events, CALENDAR_CALL], []
    )["completed"]
    # Reading alone does not create the event.
    assert not BENCH.score_turn(drift, 1, [list_events])["completed"]


# --- re-scoring a finished run ------------------------------------------------


def _stored_observation(task_id, repetition, counts, *, completed, called=()):
    return {
        "schema_version": BENCH.OBSERVATION_SCHEMA_VERSION,
        "observation_id": f"prefetch-semantic:500:{task_id}:{repetition}",
        "arm": "prefetch-semantic",
        "catalog": {"tool_count": 500},
        "task": {
            "id": task_id, "completed": completed, "correct_tool_recalled": not called,
            "expected_tools": [], "called_tools": list(called),
            "unauthorized_tool_leaks": 0,
        },
        "counts": {"tool_search_calls": 0, "result_read_calls": 0,
                   "bridged_tool_calls": 0, **counts},
        "tokens": {"input": 100},
        "latency_ms": {"end_to_end": 1000},
        "ui_probe_success": True,
        "failure": None if completed else "task_incomplete",
    }


def _write_run(run_dir, observations, traces):
    (run_dir / "llm-traces").mkdir(parents=True)
    (run_dir / "observations.jsonl").write_text(
        "".join(json.dumps(item) + "\n" for item in observations), encoding="utf-8"
    )
    for group, trace in traces.items():
        (run_dir / "llm-traces" / f"{group}.json").write_text(
            json.dumps(trace), encoding="utf-8"
        )


def _search_then_read_steps(tag):
    return [
        _tool_step([{"id": f"s{tag}", "name": "tool_search", "arguments": {}}]),
        _tool_step([_read(f"r{tag}")], [_ref_result(f"s{tag}", "tool_search", f"r{tag}")]),
        _text_step(f"none BENCHMARK_DONE_{tag}"),
    ]


def test_rescore_run_recomputes_no_tool_scoring_from_the_stored_trace(tmp_path):
    read_counts = {"tool_calls": 2, "tool_search_calls": 1, "result_read_calls": 1}
    shell_counts = {"tool_calls": 1}
    observations = [
        _stored_observation("no-match", 0, read_counts, completed=False),
        _stored_observation("no-match", 1, read_counts, completed=False),
        # A real tool call stays a failure.
        _stored_observation("no-match", 2, shell_counts, completed=False),
    ]
    trace = _trace(
        *_search_then_read_steps("a"), *_search_then_read_steps("b"),
        _tool_step([{"id": "x", "name": "builtin__shell", "arguments": {}}]),
    )
    _write_run(tmp_path, observations, {"prefetch-semantic-500-no-match": trace})

    rescored, report = BENCH.rescore_run(tmp_path)

    assert [item["task"]["completed"] for item in rescored] == [True, True, False]
    assert [item["failure"] for item in rescored] == [None, None, "task_incomplete"]
    assert [item["counts"]["discovery_result_read_calls"] for item in rescored] == [1, 1, 0]
    assert [item["counts"]["result_read_calls"] for item in rescored] == [1, 1, 0]
    assert report["rescored_observations"] == 3
    assert report["unverified_groups"] == []
    assert [change["observation_id"] for change in report["completion_changes"]] == [
        "prefetch-semantic:500:no-match:0", "prefetch-semantic:500:no-match:1",
    ]
    # The source run is only read.
    assert BENCH.load_observations(tmp_path / "observations.jsonl") == observations


def test_rescore_run_keeps_a_group_whose_trace_does_not_split_cleanly(tmp_path):
    stored = _stored_observation(
        "denied-capability", 0,
        {"tool_calls": 2, "tool_search_calls": 1, "result_read_calls": 0},
        completed=False,
    )
    missing = _stored_observation("no-match", 0, {"tool_calls": 1}, completed=False)
    _write_run(tmp_path, [stored, missing], {
        "prefetch-semantic-500-denied-capability": _trace(*_search_then_read_steps("a")),
    })

    rescored, report = BENCH.rescore_run(tmp_path)

    assert rescored == [stored, missing]
    assert report["rescored_observations"] == 0
    assert report["unverified_groups"] == [
        {
            "group": "prefetch-semantic-500-denied-capability",
            "reason": (
                "prefetch-semantic:500:denied-capability:0: re-sliced "
                "result_read_calls 1 != stored 0"
            ),
        },
        {"group": "prefetch-semantic-500-no-match", "reason": "trace missing"},
    ]


def test_rescore_keeps_the_stored_scoring_of_tasks_with_expected_tools(tmp_path):
    stored = _stored_observation("natural-language-alias", 0, {"tool_calls": 1},
                                 completed=True, called=("google_calendar__list_events",))
    trace = _trace(_tool_step([{
        "id": "c", "name": "mcp-benchmark-google-calendar__google_calendar__list_events",
        "arguments": {},
    }]))
    _write_run(tmp_path, [stored], {
        "prefetch-semantic-500-natural-language-alias": trace,
    })

    rescored, report = BENCH.rescore_run(tmp_path)

    assert rescored[0]["task"] == stored["task"]
    assert rescored[0]["counts"]["discovery_result_read_calls"] == 0
    assert report["rescored_observations"] == 1


def test_rescore_main_writes_a_fresh_output_and_refuses_the_source(tmp_path):
    run_dir = tmp_path / "run"
    _write_run(
        run_dir,
        [_stored_observation(
            "no-match", 0,
            {"tool_calls": 2, "tool_search_calls": 1, "result_read_calls": 1},
            completed=False,
        )],
        {"prefetch-semantic-500-no-match": _trace(*_search_then_read_steps("a"))},
    )
    out = tmp_path / "out"

    with pytest.raises(RuntimeError, match="must differ"):
        BENCH.rescore_main(run_dir, run_dir)
    assert BENCH.rescore_main(run_dir, out) == 0
    with pytest.raises(RuntimeError, match="already exists"):
        BENCH.rescore_main(run_dir, out)

    summary = json.loads((out / "summary.json").read_text())
    assert summary["schema_version"] == BENCH.SUMMARY_SCHEMA_VERSION
    assert summary["rescore"]["source_dir"] == str(run_dir)
    assert summary["aggregates"][0]["completion_rate"] == 1.0
    assert summary["aggregates"][0]["discovery_result_read_calls_total"] == 1
    assert BENCH.load_observations(out / "observations.jsonl")[0]["task"]["completed"]


def test_rescore_flag_skips_the_live_run(tmp_path, monkeypatch):
    calls = []
    monkeypatch.setattr(BENCH, "rescore_main", lambda source, out: calls.append((source, out)) or 0)
    monkeypatch.setattr(
        BENCH.sys, "argv",
        ["run_benchmark.py", "--rescore", str(tmp_path / "run"),
         "--output-dir", str(tmp_path / "out")],
    )
    monkeypatch.delenv("NEARAI_API_KEY", raising=False)
    monkeypatch.delenv("LIVE_OPENAI_COMPATIBLE_API_KEY", raising=False)

    assert BENCH.main() == 0
    assert calls == [(tmp_path / "run", tmp_path / "out")]


def test_rescore_reads_older_supported_schemas_but_a_resume_does_not(tmp_path, monkeypatch):
    stored = _stored_observation("no-match", 0, {"tool_calls": 0}, completed=True)
    _write_run(tmp_path, [stored], {"prefetch-semantic-500-no-match": _trace()})
    # A later schema bump must not strand runs the re-score can still read.
    monkeypatch.setattr(BENCH, "OBSERVATION_SCHEMA_VERSION", stored["schema_version"] + 1)

    rescored, report = BENCH.rescore_run(tmp_path)

    assert rescored[0]["schema_version"] == stored["schema_version"]
    assert report["rescored_observations"] == 1
    with pytest.raises(ValueError, match="expected"):
        BENCH.load_observations(tmp_path / "observations.jsonl")
    monkeypatch.setattr(BENCH, "RESCORE_MIN_SCHEMA_VERSION", stored["schema_version"] - 3)
    rescored, _ = BENCH.rescore_run(tmp_path)
    assert rescored[0]["task"]["completed"]
    stale = {**stored, "schema_version": stored["schema_version"] - 4}
    _write_run(tmp_path / "stale", [stale], {})
    with pytest.raises(ValueError, match=r"expected \d+ to \d+"):
        BENCH.rescore_run(tmp_path / "stale")


def test_max_tools_sweeps_only_the_selection_arms():
    assert BENCH.max_tools_sweep("prefetch-lexical", [25, 50, 25]) == [25, 50]
    assert BENCH.max_tools_sweep("prefetch-semantic", [150]) == [150]
    assert BENCH.max_tools_sweep("prefetch-lexical", None) == [None]
    assert BENCH.max_tools_sweep("namespaces", [25, 50]) == [None]


def test_observation_id_carries_max_tools_only_when_swept():
    assert BENCH.observation_id("off", 100, "no-match", 2) == "off:100:no-match:2"
    assert (
        BENCH.observation_id("prefetch-lexical", 500, "no-match", 0, 50)
        == "prefetch-lexical:500:max_tools=50:no-match:0"
    )


def test_max_tools_sets_and_records_the_selection_arms_cap():
    catalogs = BENCH.generate_catalog(100)
    exported = {"REBORN_TOOL_PREFETCH_MAX_TOOLS": "40"}

    env, config = BENCH.arm_env("prefetch-lexical", catalogs, exported, 25)
    assert env["REBORN_TOOL_PREFETCH_MAX_TOOLS"] == "25"
    assert config["max_tools"] == 25
    assert config["prefetch_tuning"]["REBORN_TOOL_PREFETCH_MAX_TOOLS"] == "25"

    # Without the flag the runtime default (or the exported value) applies.
    env, config = BENCH.arm_env("prefetch-lexical", catalogs, exported)
    assert "REBORN_TOOL_PREFETCH_MAX_TOOLS" not in env
    assert "max_tools" not in config
    assert config["prefetch_tuning"]["REBORN_TOOL_PREFETCH_MAX_TOOLS"] == "40"

    # A disclosure-only arm has no selection to cap.
    env, config = BENCH.arm_env("namespaces", catalogs, {}, 25)
    assert "REBORN_TOOL_PREFETCH_MAX_TOOLS" not in env
    assert "max_tools" not in config


def _swept(arm, tool_count, max_tools, completed, turns=None):
    config = {"prefetch": "lexical"}
    if max_tools is not None:
        config["max_tools"] = max_tools
    return {
        "arm": arm,
        "catalog": {"tool_count": tool_count},
        "config": config,
        "task": {"id": "topic-drift", "completed": completed, "unauthorized_tool_leaks": 0},
        "latency_ms": {"end_to_end": 100},
        "failure": None,
        "turns": turns,
    }


def test_aggregates_group_by_arm_size_and_max_tools():
    observations = [
        _swept("prefetch-lexical", 100, 50, True),
        _swept("prefetch-lexical", 100, 25, False),
        _swept("prefetch-lexical", 100, None, True),
        _swept("prefetch-lexical", 100, 25, True),
        _swept("prefetch-lexical", 500, 25, True),
    ]
    aggregates = BENCH.aggregate_observations(observations)
    assert [
        (item["arm"], item["tool_count"], item["max_tools"], item["observations"])
        for item in aggregates
    ] == [
        ("prefetch-lexical", 100, None, 1),
        ("prefetch-lexical", 100, 25, 2),
        ("prefetch-lexical", 100, 50, 1),
        ("prefetch-lexical", 500, 25, 1),
    ]
    assert aggregates[1]["completion_rate"] == 0.5
    assert all(item["config_consistent"] for item in aggregates)

    turn = {"replied": True}
    turn_aggregates = BENCH.aggregate_turns([
        _swept("prefetch-lexical", 100, 50, True, turns=[turn]),
        _swept("prefetch-lexical", 100, 25, True, turns=[turn]),
    ])
    assert [item["max_tools"] for item in turn_aggregates] == [25, 50]


def test_a_max_tools_sweep_resumes_each_value_separately(tmp_path, monkeypatch):
    ran = []

    async def fake_group(
        _live_qa, _binary, _output_dir, arm, tool_count, task, repetitions,
        _observations_path, _recorder, max_tools,
    ):
        ran.append((arm, max_tools, tuple(repetitions)))
        return [
            {"observation_id": BENCH.observation_id(
                arm, tool_count, task["id"], repetition, max_tools,
            )}
            for repetition in repetitions
        ]

    monkeypatch.setattr(BENCH, "run_task_group", fake_group)
    args = BENCH.argparse.Namespace(
        binary=tmp_path / "ironclaw", output_dir=tmp_path, repetitions=2,
        max_tools=[25, 50],
    )
    completed = {
        BENCH.observation_id("prefetch-lexical", 100, "no-match", 0, 25),
        BENCH.observation_id("prefetch-lexical", 100, "no-match", 1, 25),
        BENCH.observation_id("prefetch-lexical", 100, "no-match", 0, 50),
    }
    observations = []
    asyncio.run(BENCH._run_matrix(
        object(), args, ["namespaces", "prefetch-lexical"], [100],
        [_task("no-match")], observations, completed,
        tmp_path / "observations.jsonl", None,
    ))
    assert ran == [
        ("namespaces", None, (0, 1)),
        ("prefetch-lexical", 50, (1,)),
    ]
    assert BENCH.observation_id("prefetch-lexical", 100, "no-match", 1, 50) in completed


def test_max_tools_flag_is_repeatable_and_positive(monkeypatch):
    monkeypatch.setattr(sys, "argv", [
        "run_benchmark.py", "--output-dir", "out", "--max-tools", "25",
        "--max-tools", "150",
    ])
    assert BENCH.parse_args().max_tools == [25, 150]
    monkeypatch.setattr(sys, "argv", ["run_benchmark.py", "--output-dir", "out"])
    assert BENCH.parse_args().max_tools is None
    monkeypatch.setattr(sys, "argv", [
        "run_benchmark.py", "--output-dir", "out", "--max-tools", "0",
    ])
    with pytest.raises(SystemExit):
        BENCH.parse_args()
