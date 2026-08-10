#!/usr/bin/env python3
"""Run the real-model tool-disclosure benchmark against the shipping server.

The benchmark uses a loopback MCP fixture behind IronClaw's debug-only HTTP
rewrite seam. The model and agent loop are real; only tool side effects are
synthetic and recorded for deterministic scoring.
"""

from __future__ import annotations

import argparse
import asyncio
import importlib.util
import json
import os
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
CORPUS_PATH = (
    ROOT
    / "crates/loop/ironclaw_loop_host/tests/fixtures/tool_search_relevance.json"
)
LIVE_QA_PATH = ROOT / "scripts/reborn_webui_v2_live_qa/run_live_qa.py"
GENERATOR_VERSION = "tool-search-scale-v2"
OBSERVATION_SCHEMA_VERSION = 2
SUMMARY_SCHEMA_VERSION = 2
SEED = 7405
AUTH_TOKEN = "reborn-webui-v2-live-qa-token-0123456789abcdef"

NAMESPACES = (
    "browser", "database", "documents", "extensions", "github", "gmail",
    "google-calendar", "google-drive", "google-sheets", "hubspot", "incident",
    "jira", "linear", "media", "memory", "notion", "slack", "stripe",
    "system", "workflow-admin",
)
NAMESPACE_COUNT = len(NAMESPACES)
ACTIONS = (
    "archive_record", "compare_snapshot", "export_summary", "get_status",
    "inspect_artifact", "list_categories", "normalize_dataset",
    "record_checkpoint", "resolve_reference", "review_manifest",
    "summarize_usage", "sync_metadata", "validate_policy", "verify_checksum",
    "view_history", "write_annotation",
)
NOUNS = (
    "artifact", "batch", "bundle", "checkpoint", "entry", "manifest",
    "record", "reference", "snapshot", "summary", "version", "workspace",
)

ARMS = ("off", "compact", "signatures", "namespaces", "bridged")

TASKS = (
    {
        "id": "exact-canonical-id",
        "prompt": (
            "Use exactly the capability {canonical} to inspect pull request 7273 in "
            "nearai/ironclaw. If tool_search is available, first call it with the "
            "query string exactly `{canonical}`; otherwise call the capability "
            "directly. Do not substitute another capability."
        ),
        "expected": ("github__get_pull_request",),
    },
    {
        "id": "natural-language-alias",
        "prompt": "Find the upcoming events in my Google Calendar.",
        "expected": ("google_calendar__list_events",),
    },
    {
        "id": "ambiguous-relevant-set",
        "prompt": "Search the CRM for the contact whose email is ada@example.com.",
        "expected": ("hubspot__search_contacts",),
    },
    {
        "id": "nested-argument-vocabulary",
        "prompt": (
            "Upload a file named report.csv containing exactly `benchmark-report` "
            "to Google Drive using MIME type text/csv. Use the available tool that "
            "accepts mime_type."
        ),
        "expected": ("google_drive__upload_file",),
    },
    {
        "id": "cross-namespace-workflow",
        "prompt": (
            "Find the latest Gmail message about Project Aurora, then create a Google "
            "Calendar event from the meeting details it returns. Complete both steps."
        ),
        "expected": ("gmail__search_messages", "google_calendar__create_event"),
    },
    {
        "id": "no-match",
        "prompt": (
            "Use an authorized tool to translate telepathic cactus signals. If no such "
            "tool exists, say so without calling an unrelated tool."
        ),
        "expected": (),
    },
    {
        "id": "denied-capability",
        "prompt": (
            "Use spawn_subagent to delegate this task. If that capability is not in "
            "your authorized surface, report that it is unavailable without calling "
            "an unrelated tool."
        ),
        "expected": (),
        "forbidden": ("spawn_subagent",),
    },
)


def _load_live_qa() -> Any:
    if sys.version_info < (3, 11):
        import tomli

        sys.modules.setdefault("tomllib", tomli)
    spec = importlib.util.spec_from_file_location("ironclaw_live_qa", LIVE_QA_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load live QA helpers from {LIVE_QA_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _corpus_namespace(capability_id: str) -> str:
    owner = capability_id.split(".", 1)[0]
    return {
        "archive": "documents",
        "builtin": "system",
        "csv": "documents",
        "docs": "documents",
        "extension": "extensions",
        "filesystem": "system",
        "image": "media",
        "pdf": "documents",
    }.get(owner, owner)


def generate_catalog(tool_count: int) -> list[dict[str, Any]]:
    corpus = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    corpus_tools = corpus["tools"]
    if tool_count < len(corpus_tools):
        raise ValueError(f"tool_count must be at least {len(corpus_tools)}")
    buckets: dict[str, list[dict[str, Any]]] = {namespace: [] for namespace in NAMESPACES}
    for tool in corpus_tools:
        namespace = _corpus_namespace(tool["capability_id"])
        if namespace not in buckets:
            raise ValueError(f"corpus namespace is not mapped: {namespace}")
        buckets[namespace].append({
            "name": tool["name"],
            "description": tool["description"],
            "inputSchema": tool["parameters"],
            "annotations": {"readOnlyHint": True},
        })
    namespace_offset = SEED % len(NAMESPACES)
    action_offset = SEED % len(ACTIONS)
    noun_offset = SEED % len(NOUNS)
    namespace_order = NAMESPACES[namespace_offset:] + NAMESPACES[:namespace_offset]
    for ordinal in range(tool_count - len(corpus_tools)):
        namespace = min(namespace_order, key=lambda item: len(buckets[item]))
        action = ACTIONS[(ordinal + action_offset) % len(ACTIONS)]
        noun = NOUNS[(ordinal + noun_offset) % len(NOUNS)]
        buckets[namespace].append(
            {
                "name": f"{action}_{ordinal:04}",
                "description": (
                    f"{action} {noun} records in the {namespace} benchmark integration."
                ),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        f"{noun}_id": {"type": "string"},
                        "cursor": {"type": "string"},
                        "limit": {"type": "integer"},
                    },
                    "required": [f"{noun}_id"],
                },
                "annotations": {"readOnlyHint": True},
            }
        )
    for namespace, tools in buckets.items():
        if not tools:
            raise ValueError(
                f"empty benchmark namespace {namespace!r} at tool_count={tool_count}; "
                "increase the catalog size so every MCP package is installable"
            )
    return [{"namespace": namespace, "tools": buckets[namespace]} for namespace in NAMESPACES]


def canonical_capability_id(catalogs: list[dict[str, Any]], tool_name: str) -> str:
    for catalog in catalogs:
        if any(tool["name"] == tool_name for tool in catalog["tools"]):
            return f"mcp-benchmark-{catalog['namespace']}.{tool_name}"
    raise ValueError(f"tool is absent from benchmark catalog: {tool_name}")


class McpFixture:
    def __init__(self, catalogs: list[dict[str, Any]]) -> None:
        self.catalogs = catalogs
        self.calls: list[dict[str, Any]] = []
        self._lock = threading.Lock()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *_args: object) -> None:
                return

            def do_POST(self) -> None:  # noqa: N802 - stdlib callback name
                length = int(self.headers.get("content-length", "0"))
                try:
                    body = json.loads(self.rfile.read(length) or b"{}")
                    result, status = fixture.handle(self.path, body)
                except Exception as exc:  # fixture boundary: return typed JSON-RPC failure
                    result = {
                        "jsonrpc": "2.0", "id": None,
                        "error": {"code": -32603, "message": type(exc).__name__},
                    }
                    status = 500
                payload = json.dumps(result).encode("utf-8")
                self.send_response(status)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def port(self) -> int:
        return int(self.server.server_address[1])

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def handle(self, path: str, body: dict[str, Any]) -> tuple[dict[str, Any], int]:
        namespace_index = int(path.rstrip("/").split("/")[-1])
        request_id = body.get("id")
        method = body.get("method")
        if method == "initialize":
            namespace = self.catalogs[namespace_index]["namespace"]
            value = {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": f"benchmark-{namespace}", "version": "1"},
                "capabilities": {"tools": {}},
            }
        elif method == "notifications/initialized":
            value = {}
        elif method == "tools/list":
            value = {"tools": self.catalogs[namespace_index]["tools"]}
        elif method == "tools/call":
            params = body.get("params") if isinstance(body.get("params"), dict) else {}
            call = {
                "namespace": namespace_index,
                "name": str(params.get("name") or ""),
                "arguments": params.get("arguments") if isinstance(params.get("arguments"), dict) else {},
                "monotonic_ns": time.monotonic_ns(),
            }
            with self._lock:
                self.calls.append(call)
            if call["name"] == "gmail__search_messages":
                text = "Project Aurora meeting is 2026-08-12 at 10:00 UTC for 30 minutes."
            else:
                text = f"benchmark tool {call['name']} completed"
            value = {"content": [{"type": "text", "text": text}]}
        else:
            return {
                "jsonrpc": "2.0", "id": request_id,
                "error": {"code": -32601, "message": "method not found"},
            }, 200
        return {"jsonrpc": "2.0", "id": request_id, "result": value}, 200


def _request_json(base_url: str, path: str, payload: dict[str, Any]) -> dict[str, Any]:
    request = urllib.request.Request(
        f"{base_url}{path}",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "authorization": f"Bearer {AUTH_TOKEN}",
            "content-type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.loads(response.read())
    except urllib.error.HTTPError as exc:
        raise RuntimeError(f"{path} returned HTTP {exc.code}: {exc.read()[:500]!r}") from exc


def _get_json(base_url: str, path: str) -> dict[str, Any]:
    request = urllib.request.Request(
        f"{base_url}{path}",
        headers={"authorization": f"Bearer {AUTH_TOKEN}"},
        method="GET",
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.loads(response.read())


async def wait_for_ready(url: str, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            await asyncio.to_thread(urllib.request.urlopen, url, None, 2)
            return
        except (OSError, urllib.error.URLError):
            await asyncio.sleep(0.2)
    raise TimeoutError(f"server did not become ready at {url}")


def install_catalog(base_url: str, catalogs: list[dict[str, Any]]) -> list[str]:
    package_ids = []
    for index, catalog in enumerate(catalogs):
        namespace = catalog["namespace"]
        desired_id = f"benchmark-{namespace}"
        registration = _request_json(
            base_url,
            "/api/webchat/v2/extensions/register-hosted-mcp",
            {
                "desired_id": desired_id,
                "desired_name": f"Benchmark {namespace}",
                "endpoint": f"https://example.com/benchmark/{index}",
                "auth_selection": {"kind": "no_auth"},
            },
        )
        package_id = registration["package_ref"]["id"]
        _request_json(
            base_url,
            "/api/webchat/v2/extensions/install",
            {
                "package_ref": {"kind": "extension", "id": package_id},
                "client_action_id": f"tool-benchmark-{uuid.uuid4()}",
            },
        )
        _request_json(
            base_url,
            f"/api/webchat/v2/extensions/{package_id}/setup",
            {
                "action": "submit",
                "payload": {"secrets": {}},
                "client_action_id": f"tool-benchmark-setup-{uuid.uuid4()}",
            },
        )
        package_ids.append(package_id)
    _request_json(base_url, "/api/webchat/v2/settings/tools", {"enabled": True})
    projections = _get_json(base_url, "/api/webchat/v2/extensions").get("extensions", [])
    installed = {
        item.get("package_ref", {}).get("id"): item
        for item in projections
        if isinstance(item, dict) and isinstance(item.get("package_ref"), dict)
    }
    invalid = {
        package_id: {
            "state": installed.get(package_id, {}).get("installation_state"),
            "tool_count": len(installed.get(package_id, {}).get("tools") or []),
            "activation_error": installed.get(package_id, {}).get("activation_error"),
        }
        for package_id in package_ids
        if installed.get(package_id, {}).get("installation_state") != "active"
        or not installed.get(package_id, {}).get("tools")
    }
    if invalid:
        raise RuntimeError(f"hosted MCP catalog failed active read-back: {invalid}")
    return package_ids


def _ordered_expected_calls(
    expected: list[str], calls: list[dict[str, Any]],
) -> list[dict[str, Any]] | None:
    selected = []
    cursor = 0
    for expected_name in expected:
        while cursor < len(calls) and calls[cursor]["name"] != expected_name:
            cursor += 1
        if cursor == len(calls):
            return None
        selected.append(calls[cursor])
        cursor += 1
    return selected


def _contains_casefold(value: object, expected: str) -> bool:
    return isinstance(value, str) and expected.casefold() in value.casefold()


def _task_arguments_are_valid(task_id: str, calls: list[dict[str, Any]]) -> bool:
    if not calls:
        return task_id in {"no-match", "denied-capability"}
    arguments = [call.get("arguments", {}) for call in calls]
    if task_id == "exact-canonical-id":
        first = arguments[0]
        return (
            first.get("owner") == "nearai"
            and first.get("repo") == "ironclaw"
            and str(first.get("pull_number")) == "7273"
        )
    if task_id == "natural-language-alias":
        return True
    if task_id == "ambiguous-relevant-set":
        first = arguments[0]
        return first.get("email") == "ada@example.com" or _contains_casefold(
            first.get("query"), "ada@example.com"
        )
    if task_id == "nested-argument-vocabulary":
        first = arguments[0]
        return (
            first.get("name") == "report.csv"
            and first.get("content") == "benchmark-report"
            and first.get("mime_type") == "text/csv"
        )
    if task_id == "cross-namespace-workflow":
        search, create = arguments
        schedule = create.get("schedule")
        return (
            _contains_casefold(search.get("query"), "project aurora")
            and isinstance(schedule, dict)
            and _contains_casefold(schedule.get("start_at"), "2026-08-12")
            and _contains_casefold(schedule.get("start_at"), "10:00")
            and _contains_casefold(schedule.get("end_at"), "2026-08-12")
            and _contains_casefold(schedule.get("end_at"), "10:30")
        )
    return True


def _attempted_target(call: dict[str, Any]) -> str:
    name = str(call.get("name") or "")
    if name != "tool_call":
        return name
    arguments = call.get("arguments")
    if not isinstance(arguments, dict) or not isinstance(arguments.get("name"), str):
        return name
    return arguments["name"]


def _matches_forbidden(target: str, forbidden: tuple[str, ...]) -> bool:
    normalized = target.replace('.', '__')
    return any(
        normalized == item or normalized.endswith(f"__{item}")
        for item in forbidden
    )


def score_task(
    task: dict[str, Any],
    calls: list[dict[str, Any]],
    attempted_calls: list[dict[str, Any]],
) -> dict[str, Any]:
    called = [call["name"] for call in calls]
    expected = list(task["expected"])
    forbidden = tuple(task.get("forbidden", ()))
    unauthorized = sum(
        _matches_forbidden(_attempted_target(call), forbidden)
        for call in attempted_calls
    )
    if expected:
        correct = all(name in called for name in expected)
        ordered = _ordered_expected_calls(expected, calls)
        completed = (
            ordered is not None
            and _task_arguments_are_valid(task["id"], ordered)
            and unauthorized == 0
        )
    else:
        correct = not called
        discovery_names = {"tool_search", "tool_describe", "capability_info"}
        non_discovery_attempts = [
            call for call in attempted_calls if str(call.get("name")) not in discovery_names
        ]
        completed = correct and not non_discovery_attempts and unauthorized == 0
    return {
        "completed": completed,
        "correct_tool_recalled": correct,
        "expected_tools": expected,
        "called_tools": called,
        "unauthorized_tool_leaks": unauthorized,
    }


def first_correct_tool_call_latency_ms(
    expected: tuple[str, ...], calls: list[dict[str, Any]], started: float,
) -> int | None:
    expected_names = set(expected)
    first = next((call for call in calls if call["name"] in expected_names), None)
    if first is None:
        return None
    return int((first["monotonic_ns"] / 1_000_000) - (started * 1000))


def aggregate_observations(observations: list[dict[str, Any]]) -> list[dict[str, Any]]:
    groups: dict[tuple[str, int], list[dict[str, Any]]] = {}
    for observation in observations:
        key = (observation["arm"], observation["catalog"]["tool_count"])
        groups.setdefault(key, []).append(observation)
    aggregates = []
    for (arm, tool_count), items in sorted(groups.items()):
        latencies = [item["latency_ms"]["end_to_end"] for item in items]
        completed = sum(bool(item["task"]["completed"]) for item in items)
        failure_categories: dict[str, int] = {}
        for item in items:
            failure = item.get("failure")
            if isinstance(failure, str):
                failure_categories[failure] = failure_categories.get(failure, 0) + 1
        aggregates.append({
            "arm": arm,
            "tool_count": tool_count,
            "observations": len(items),
            "completion_rate": completed / len(items),
            "latency_ms_median": statistics.median(latencies),
            "latency_ms_worst": max(latencies),
            "latency_ms_spread": max(latencies) - min(latencies),
            "unauthorized_tool_leaks": sum(
                item["task"]["unauthorized_tool_leaks"] for item in items
            ),
            "failure_categories": failure_categories,
        })
    return aggregates


def _trace_metrics(
    live_qa: Any, trace_path: Path,
) -> tuple[dict[str, object], list[dict[str, Any]]]:
    if not trace_path.exists():
        return {
            "model_call_count": 0, "tool_call_count": 0, "input_tokens": 0,
            "output_tokens": 0, "cache_read_tokens": 0,
            "uncached_input_tokens": 0, "cost_usd": "0",
        }, []
    metrics = live_qa.parse_case_llm_trace_metrics(trace_path)
    payload = json.loads(trace_path.read_text(encoding="utf-8"))
    calls = []
    for model_turn, step in enumerate(payload.get("steps", [])):
        response = step.get("response") if isinstance(step, dict) else None
        if not isinstance(response, dict):
            continue
        for call in response.get("tool_calls", []):
            if isinstance(call, dict) and isinstance(call.get("name"), str):
                calls.append({
                    "name": call["name"],
                    "model_turn": model_turn,
                    "arguments": call.get("arguments")
                    if isinstance(call.get("arguments"), dict) else {},
                })
    return metrics, calls


def discovery_turn_count(calls: list[dict[str, Any]]) -> int:
    discovery_names = {"tool_search", "tool_describe", "capability_info"}
    return len({
        call["model_turn"]
        for call in calls
        if call.get("name") in discovery_names and isinstance(call.get("model_turn"), int)
    })


def run_cache_metadata(
    repetitions: list[int], group_position: int, repetition: int,
) -> dict[str, object]:
    return {
        "thermal_class": "cold" if group_position == 0 else "warm",
        "repetition": repetition,
        "resumed_group": bool(repetitions and repetitions[0] != 0),
    }


async def git_head() -> str:
    proc = await asyncio.create_subprocess_exec(
        "git",
        "rev-parse",
        "HEAD",
        cwd=ROOT,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await proc.communicate()
    if proc.returncode != 0:
        reason = stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git rev-parse HEAD failed: {reason or 'no error output'}")
    head = stdout.decode("utf-8", errors="strict").strip()
    if not head:
        raise RuntimeError("git rev-parse HEAD returned an empty commit")
    return head


def _metric_delta(after: dict[str, object], before: dict[str, object], key: str) -> int | None:
    left = after.get(key)
    right = before.get(key)
    if isinstance(left, int) and isinstance(right, int):
        return left - right
    return None


async def run_task_group(
    live_qa: Any,
    binary: Path,
    output_dir: Path,
    arm: str,
    tool_count: int,
    task: dict[str, Any],
    repetitions: list[int],
    observations_path: Path,
) -> list[dict[str, Any]]:
    group_name = f"{arm}-{tool_count}-{task['id']}"
    case_dir = output_dir / "cases" / group_name
    home = live_qa.create_generated_reborn_home(case_dir / "source-home")
    catalogs = generate_catalog(tool_count)
    fixture = McpFixture(catalogs)
    fixture.start()
    trace_env = live_qa.case_llm_trace_env(output_dir, group_name)
    extra_env = {
        "REBORN_TOOL_DISCLOSURE": arm,
        "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP": f"example.com=127.0.0.1:{fixture.port}",
        **trace_env,
    }
    if arm == "bridged":
        extra_env["REBORN_TOOL_DISCLOSURE_PROFILE_PINS"] = json.dumps({
            "interactive_tools": [
                canonical_capability_id(catalogs, "github__get_pull_request"),
                canonical_capability_id(catalogs, "google_calendar__list_events"),
                canonical_capability_id(catalogs, "gmail__search_messages"),
            ]
        })
    proc = None
    try:
        live_qa.wait_for_ready = wait_for_ready
        proc, base_url = await live_qa.start_reborn_server(binary, home, case_dir, extra_env)
        packages = await asyncio.to_thread(install_catalog, base_url, catalogs)
        ctx = live_qa.LiveQaContext(
            base_url=base_url, output_dir=output_dir, reborn_home=home, env=extra_env
        )
        trace_path = output_dir / "llm-traces" / f"{group_name}.json"
        prior_metrics, prior_trace_calls = _trace_metrics(live_qa, trace_path)
        observations = []
        for group_position, repetition in enumerate(repetitions):
            case_name = f"{group_name}-{repetition}"
            print(
                f"[tool-benchmark] arm={arm} tools={tool_count} "
                f"task={task['id']} repetition={repetition}", flush=True,
            )
            calls_before = len(fixture.calls)
            marker = f"BENCHMARK_DONE_{case_name}".replace("-", "_")
            started = time.monotonic()
            result = await live_qa._live_chat_case(
                ctx,
                case_name=case_name,
                prompt=(
                    f"{task['prompt'].format(canonical=canonical_capability_id(catalogs, task['expected'][0]) if task['expected'] else '')}"
                    "\n\nAfter completing the task, end your final response "
                    f"with exactly: {marker}"
                ),
                marker=marker,
                required_text=[marker],
                timeout=180.0,
                enforce_marker=True,
            )
            latency_ms = int((time.monotonic() - started) * 1000)
            calls = fixture.calls[calls_before:]
            metrics, all_trace_calls = _trace_metrics(live_qa, trace_path)
            trace_calls = all_trace_calls[len(prior_trace_calls):]
            tool_names = [call["name"] for call in trace_calls]
            scored = score_task(task, calls, trace_calls)
            observation = {
                "schema_version": OBSERVATION_SCHEMA_VERSION,
                "observation_id": f"{arm}:{tool_count}:{task['id']}:{repetition}",
                "catalog": {
                    "generator_version": GENERATOR_VERSION,
                    "seed": SEED,
                    "tool_count": tool_count,
                    "namespace_count": NAMESPACE_COUNT,
                },
                "arm": arm,
                "model": {
                    "provider": os.environ.get(
                        "REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID", "nearai"
                    ),
                    "model": os.environ.get(
                        "REBORN_WEBUI_V2_LIVE_QA_LLM_MODEL",
                        os.environ.get(
                            "LIVE_OPENAI_COMPATIBLE_MODEL",
                            "deepseek-ai/DeepSeek-V4-Flash",
                        ),
                    ),
                    "temperature": 0.0,
                },
                "run": run_cache_metadata(repetitions, group_position, repetition),
                "task": {"id": task["id"], **scored},
                "counts": {
                    "model_turns": _metric_delta(
                        metrics, prior_metrics, "model_call_count"
                    ),
                    "tool_calls": _metric_delta(
                        metrics, prior_metrics, "tool_call_count"
                    ),
                    "synthetic_tool_calls": len(calls),
                    "tool_search_calls": tool_names.count("tool_search"),
                    "tool_describe_calls": tool_names.count("tool_describe"),
                    "discovery_turns": discovery_turn_count(trace_calls),
                },
                "tokens": {
                    "input": _metric_delta(metrics, prior_metrics, "input_tokens"),
                    "cached_input": _metric_delta(
                        metrics, prior_metrics, "cache_read_tokens"
                    ),
                    "uncached_input": _metric_delta(
                        metrics, prior_metrics, "uncached_input_tokens"
                    ),
                    "output": _metric_delta(metrics, prior_metrics, "output_tokens"),
                    "cost_usd": None,
                },
                "latency_ms": {
                    "time_to_first_correct_tool_call": first_correct_tool_call_latency_ms(
                        tuple(task["expected"]), calls, started
                    ),
                    "end_to_end": latency_ms,
                },
                "cache": {"tool_definition_signature_changes": None},
                "ui_probe_success": result.success,
                "installed_namespaces": len(packages),
                "failure": None
                if result.success and scored["completed"]
                else "task_incomplete",
            }
            append_observation(observations_path, observation)
            observations.append(observation)
            prior_metrics = metrics
            prior_trace_calls = all_trace_calls
            print(
                f"[tool-benchmark] completed={scored['completed']} "
                f"latency_ms={latency_ms}", flush=True,
            )
        return observations
    finally:
        if proc is not None:
            live_qa.stop_process(proc)
        fixture.stop()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/ironclaw")
    parser.add_argument("--arm", action="append", choices=ARMS)
    parser.add_argument("--tool-count", action="append", type=int)
    parser.add_argument("--task", action="append", choices=[task["id"] for task in TASKS])
    parser.add_argument("--repetitions", type=int, default=4)
    return parser.parse_args()


def load_observations(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    by_id: dict[str, dict[str, Any]] = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        observation = json.loads(line)
        if observation.get("schema_version") != OBSERVATION_SCHEMA_VERSION:
            raise ValueError(
                f"{path}:{line_number} has schema_version "
                f"{observation.get('schema_version')!r}; expected "
                f"{OBSERVATION_SCHEMA_VERSION}"
            )
        observation_id = observation.get("observation_id")
        if not isinstance(observation_id, str) or not observation_id:
            raise ValueError(
                f"{path}:{line_number} is missing a non-empty observation_id"
            )
        by_id.setdefault(observation_id, observation)
    return list(by_id.values())


def append_observation(path: Path, observation: dict[str, Any]) -> None:
    observation_id = observation.get("observation_id")
    if not isinstance(observation_id, str) or not observation_id:
        raise ValueError("observation requires a non-empty observation_id")
    if any(
        existing["observation_id"] == observation_id
        for existing in load_observations(path)
    ):
        return
    encoded = json.dumps(observation, sort_keys=True) + "\n"
    with path.open("a", encoding="utf-8") as handle:
        handle.write(encoded)
        handle.flush()
        os.fsync(handle.fileno())


async def async_main(args: argparse.Namespace) -> int:
    if not os.environ.get("NEARAI_API_KEY") and not os.environ.get("LIVE_OPENAI_COMPATIBLE_API_KEY"):
        raise RuntimeError("a live model API key is required")
    if not args.binary.exists():
        raise RuntimeError(f"shipping binary not found: {args.binary}")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    observations_path = args.output_dir / "observations.jsonl"
    live_qa = _load_live_qa()
    arms = args.arm or list(ARMS)
    tool_counts = args.tool_count or [100, 500, 1000]
    tasks = [task for task in TASKS if not args.task or task["id"] in args.task]
    observations = load_observations(observations_path)
    completed_ids = {item["observation_id"] for item in observations}
    for tool_count in tool_counts:
        for arm in arms:
            for task in tasks:
                missing_repetitions = [
                    repetition
                    for repetition in range(args.repetitions)
                    if f"{arm}:{tool_count}:{task['id']}:{repetition}" not in completed_ids
                ]
                if not missing_repetitions:
                    continue
                group = await run_task_group(
                    live_qa, args.binary, args.output_dir, arm, tool_count,
                    task, missing_repetitions, observations_path,
                )
                for observation in group:
                    observations.append(observation)
                    completed_ids.add(observation["observation_id"])
    summary = {
        "schema_version": SUMMARY_SCHEMA_VERSION,
        "head": await git_head(),
        "observation_count": len(observations),
        "observations_path": str(observations_path),
        "provider_usage_available": any(
            (item["tokens"]["input"] or 0) > 0 for item in observations
        ),
        "aggregates": aggregate_observations(observations),
    }
    (args.output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8"
    )
    return 0 if all(item["task"]["completed"] for item in observations) else 1


def main() -> int:
    return asyncio.run(async_main(parse_args()))


if __name__ == "__main__":
    raise SystemExit(main())
