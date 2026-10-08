#!/usr/bin/env python3
"""Smoke comparison of turn-start tool selection (Jev) against plain
progressive disclosure, on the live model.

Reuses the tool-discovery benchmark's catalog, MCP fixture, server start-up,
trace metrics and scoring (`run_benchmark.py`), but sends each turn over the
server's HTTP API, so no browser is involved. For each arm it boots one
`ironclaw serve` with the same deterministic catalog and runs every task in a
fresh conversation, recording per turn: wall-clock time, provider token
usage, model calls, and `tool_search` / `tool_describe` calls.

Needs a live model key in the environment (LLM_API_KEY with LLM_BACKEND /
LLM_BASE_URL / LLM_MODEL, or NEARAI_API_KEY, or LIVE_OPENAI_COMPATIBLE_API_KEY)
and TYPESAFE_API_KEY for the Jev arm. The server is started with a filtered
copy of the environment, so both arms differ only in REBORN_TOOL_SELECTION.

    python3 scripts/tool_discovery_benchmark/selection_smoke.py \
        --output-dir /tmp/ironclaw-selection-smoke
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import statistics
import sys
import time
import unittest.mock
import urllib.error
import uuid
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_benchmark as rb  # noqa: E402

ARMS = {"baseline": "off", "jev": "jev"}
DEFAULT_TASKS = (
    "natural-language-alias",
    "ambiguous-relevant-set",
    "nested-argument-vocabulary",
    "cross-namespace-workflow",
)
# Only these reach the server; anything else in the caller's environment
# (other tool-selection or disclosure settings, say) would skew an arm.
PASSED_EXACT = {
    "PATH", "LANG", "LC_ALL", "TMPDIR", "TYPESAFE_API_KEY", "NEARAI_API_KEY",
    "HOME", "SSL_CERT_FILE", "SSL_CERT_DIR",
}
PASSED_PREFIXES = ("LLM_", "LIVE_OPENAI_COMPATIBLE_", "REBORN_WEBUI_V2_LIVE_QA_", "OPENAI_")
LLM_KEY_VARS = ("NEARAI_API_KEY", "LIVE_OPENAI_COMPATIBLE_API_KEY")
SELECTION_LOG = "ironclaw::reborn::tool_selection=debug"


def scrub_environment(args: argparse.Namespace) -> dict[str, str]:
    """The environment the server runs in: a filtered copy of the caller's.
    `os.environ` itself is left alone, so the harness keeps the operator's."""
    keep = PASSED_EXACT | {args.jev_api_key_env, args.llm_api_key_env or ""}
    env = {
        name: value
        for name, value in os.environ.items()
        if name in keep or name.startswith(PASSED_PREFIXES) or name.upper().endswith("_PROXY")
    }
    env["RUST_LOG"] = f"ironclaw=warn,ironclaw_webui=info,{SELECTION_LOG}"
    # The generated server home is configured from the live-QA variables and
    # defaults to the NEAR AI provider. Fill them from the binary's own LLM_*
    # settings when those are what the caller has, then drop the LLM_* ones so
    # the model is configured in exactly one place. A variable named by
    # --llm-api-key-env or --jev-api-key-env stays: the config refers to it.
    for target, source in (
        ("REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID", "LLM_BACKEND"),
        ("REBORN_WEBUI_V2_LIVE_QA_LLM_MODEL", "LLM_MODEL"),
        ("REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL", "LLM_BASE_URL"),
    ):
        if env.get(source) and not env.get(target):
            env[target] = env[source]
    if env.get("LLM_API_KEY") and not env.get("NEARAI_API_KEY"):
        env.setdefault("LIVE_OPENAI_COMPATIBLE_API_KEY", env["LLM_API_KEY"])
    selected = {args.llm_api_key_env, args.jev_api_key_env}
    for name in [name for name in env if name.startswith("LLM_") and name not in selected]:
        del env[name]
    for target, value in (
        ("REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID", args.llm_provider),
        ("REBORN_WEBUI_V2_LIVE_QA_LLM_MODEL", args.llm_model),
        ("REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV", args.llm_api_key_env),
    ):
        if value:
            env[target] = value
    return env


# The benchmark fixture answers most calls with "benchmark tool <name>
# completed", which a model reads as an empty result and keeps digging. These
# give the tasks' tools a believable answer so a turn ends when the work is done.
PLAUSIBLE_RESULTS = {
    "google_calendar__list_events": (
        "2 upcoming events: (1) Design review, 2026-10-09 15:00-16:00 UTC; "
        "(2) Team sync, 2026-10-12 09:30-10:00 UTC."
    ),
    "hubspot__search_contacts": (
        "1 contact found: Ada Lovelace <ada@example.com>, Analytical Engines Ltd, id 4821."
    ),
    "google_drive__upload_file": "Uploaded report.csv (text/csv, 16 bytes), file id drv_7f3a.",
    "google_calendar__create_event": "Event created: Project Aurora meeting, id evt_91c2.",
}


class PlausibleFixture(rb.McpFixture):
    def handle(self, path: str, body: dict[str, Any]) -> tuple[dict[str, Any], int]:
        response, status = super().handle(path, body)
        params = body.get("params") if isinstance(body.get("params"), dict) else {}
        text = PLAUSIBLE_RESULTS.get(str(params.get("name") or ""))
        if body.get("method") == "tools/call" and text:
            response["result"] = {"content": [{"type": "text", "text": text}]}
        return response, status


def post(base_url: str, path: str, payload: dict[str, Any]) -> dict[str, Any]:
    return rb._request_json(base_url, path, payload)


def run_turn(
    base_url: str, channel: str, prompt: str, marker: str, timeout: float
) -> tuple[bool, int, float]:
    """Send `prompt` in a new conversation and wait for the reply that ends
    with `marker`. Returns (completed, wall-clock milliseconds, start time)."""
    thread = post(base_url, "/api/webchat/v2/threads", {"client_action_id": str(uuid.uuid4())})
    thread_id = thread["thread"]["thread_id"]
    started = time.monotonic()
    post(
        base_url,
        f"/api/webchat/v2/channels/{channel}/messages",
        {"client_action_id": str(uuid.uuid4()), "thread_id": thread_id, "content": prompt},
    )
    deadline = started + timeout
    while time.monotonic() < deadline:
        try:
            timeline = rb._get_json(base_url, f"/api/webchat/v2/threads/{thread_id}/timeline")
        except (OSError, urllib.error.URLError):
            timeline = {}
        for message in timeline.get("messages", []):
            if (
                message.get("kind") == "assistant"
                and message.get("status") == "finalized"
                and marker in (message.get("content") or "")
            ):
                return True, int((time.monotonic() - started) * 1000), started
        time.sleep(0.25)
    return False, int((time.monotonic() - started) * 1000), started


def selection_log_entries(stderr_path: Path) -> list[dict[str, Any]]:
    """Selection latency and size from the server's debug log, one entry per
    conversation that selected. Best effort: absent when the format moves."""
    entries = []
    if not stderr_path.exists():
        return entries
    text = stderr_path.read_text(encoding="utf-8", errors="replace")
    for line in re.sub(r"\x1b\[[0-9;]*m", "", text).splitlines():
        if not ("selected the conversation's tools" in line or "tool selection failed" in line):
            continue
        latency = re.search(r"latency_ms=(\d+)", line)
        if not latency:
            continue
        failed = re.search(r"error_kind=\"?(\w+)", line)
        entries.append({
            "latency_ms": int(latency.group(1)),
            "chosen": len(re.findall(r"SelectedTool", line)),
            "failed": failed.group(1) if failed else None,
        })
    return entries


async def run_arm(
    args: argparse.Namespace, live_qa: Any, arm: str, env: dict[str, str]
) -> list[dict[str, Any]]:
    case_dir = args.output_dir / "cases" / arm
    # The home generator reads the model settings from the process
    # environment; show it the server's for the length of the call.
    with unittest.mock.patch.dict(os.environ, env, clear=True):
        home = live_qa.create_generated_reborn_home(case_dir / "source-home")
    if arm == "jev":
        # Five times the shipped 2000 ms default, so one slow classification
        # does not end the arm; the server log has each call's latency_ms.
        jev = [f'api_key_env = "{args.jev_api_key_env}"', "timeout_ms = 10000"]
        if args.jev_endpoint:
            jev.append(f'endpoint = "{args.jev_endpoint}"')
        if args.jev_model:
            jev.append(f'model = "{args.jev_model}"')
        with (home / "config.toml").open("a", encoding="utf-8") as config:
            config.write("\n[tool_selection.jev]\n" + "\n".join(jev) + "\n")
    catalogs = rb.generate_catalog(args.tool_count)
    fixture = PlausibleFixture(catalogs)
    fixture.start()
    extra_env = {
        "REBORN_TOOL_DISCLOSURE": "namespaces",
        "REBORN_TOOL_SELECTION": ARMS[arm],
        # The binary's stderr filter is IRONCLAW_REBORN_LOG, not RUST_LOG.
        "IRONCLAW_REBORN_LOG": f"info,{SELECTION_LOG}",
        "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP": f"example.com=127.0.0.1:{fixture.port}",
        **live_qa.case_llm_trace_env(args.output_dir, arm),
    }
    trace_path = args.output_dir / "llm-traces" / f"{arm}.json"
    stderr_path = case_dir / "ironclaw-reborn-serve.stderr.log"
    tasks = [task for task in rb.TASKS if task["id"] in args.task]
    proc = None
    observations = []
    try:
        live_qa.wait_for_ready = rb.wait_for_ready
        proc, base_url = await live_qa.start_reborn_server(
            args.binary, home, case_dir, extra_env, base_env=env
        )
        rb.install_catalog(base_url, catalogs)
        channel = rb._get_json(base_url, "/api/webchat/v2/session")["session_channel_extension_id"]

        # Fail fast, with the server's own words, if the model is not reachable.
        completed, _, _ = run_turn(
            base_url, channel, "Reply with exactly: PREFLIGHT_OK", "PREFLIGHT_OK", 90.0
        )
        if not completed:
            tail = "\n".join(stderr_path.read_text(errors="replace").splitlines()[-25:])
            raise RuntimeError(f"[{arm}] the model did not answer a plain message.\n{tail}")
        time.sleep(1.0)

        for repetition in range(args.repetitions):
            for task in tasks:
                prior_metrics, prior_calls = rb._trace_metrics(live_qa, trace_path)
                selections_before = len(selection_log_entries(stderr_path))
                fixture_before = len(fixture.calls)
                marker = f"SMOKE_DONE_{arm}_{task['id']}_{repetition}".replace("-", "_")
                canonical = (
                    rb.canonical_capability_id(catalogs, task["expected"][0]) if task["expected"] else ""
                )
                prompt = (
                    f"{task['prompt'].format(canonical=canonical)}\n\nAfter completing the task, "
                    f"end your final response with exactly: {marker}"
                )
                replied, latency_ms, started = run_turn(
                    base_url, channel, prompt, marker, args.turn_timeout
                )
                time.sleep(1.0)  # let the trace's last step flush
                if not replied:
                    # A timed-out run keeps going on the server. Wait for it to
                    # go quiet so its model calls are not charged to the next turn.
                    quiet_since, steps = time.monotonic(), -1
                    while time.monotonic() - quiet_since < 20 and time.monotonic() - started < 600:
                        now_steps = rb._trace_metrics(live_qa, trace_path)[0]["model_call_count"]
                        if now_steps != steps:
                            quiet_since, steps = time.monotonic(), now_steps
                        time.sleep(2.0)
                metrics, all_calls = rb._trace_metrics(live_qa, trace_path)
                calls = all_calls[len(prior_calls):]
                names = [call["name"] for call in calls]
                scored = rb.score_task(task, fixture.calls[fixture_before:], calls)
                selection = selection_log_entries(stderr_path)[selections_before:]

                def delta(key: str) -> int | None:
                    return rb._metric_delta(metrics, prior_metrics, key)

                observation = {
                    "arm": arm,
                    "task": task["id"],
                    "repetition": repetition,
                    "replied": replied,
                    "completed": bool(replied and scored["completed"]),
                    "latency_ms": latency_ms,
                    "first_correct_tool_call_ms": rb.first_correct_tool_call_latency_ms(
                        tuple(task["expected"]), fixture.calls[fixture_before:], started
                    ),
                    "model_calls": delta("model_call_count"),
                    "input_tokens": delta("input_tokens"),
                    "cached_input_tokens": delta("cache_read_tokens"),
                    "output_tokens": delta("output_tokens"),
                    "tool_search_calls": names.count("tool_search"),
                    "tool_describe_calls": names.count("tool_describe"),
                    "tools_called": names,
                    "selection": selection[0] if selection else None,
                }
                if arm == "jev" and (not selection or selection[0]["failed"]):
                    # A failed selection falls back to the ordinary surface,
                    # which would make this arm a copy of the baseline.
                    reason = selection[0]["failed"] if selection else "no selection was logged"
                    raise RuntimeError(
                        f"[jev] turn-start selection did not run ({reason}); check the Jev "
                        "endpoint and key (--jev-endpoint, --jev-api-key-env)"
                    )
                observations.append(observation)
                with (args.output_dir / "observations.jsonl").open("a", encoding="utf-8") as out:
                    out.write(json.dumps(observation) + "\n")
                print(
                    f"[smoke] {arm:8} {task['id']:28} rep={repetition} ok={observation['completed']!s:5} "
                    f"{latency_ms:6d} ms  in={observation['input_tokens']} out={observation['output_tokens']} "
                    f"search={observation['tool_search_calls']} describe={observation['tool_describe_calls']}",
                    flush=True,
                )
        return observations
    finally:
        try:
            if proc is not None:
                live_qa.stop_process(proc)
        finally:
            fixture.stop()


def summarize(observations: list[dict[str, Any]]) -> dict[str, Any]:
    def mean(values: list[Any]) -> float | None:
        numbers = [value for value in values if isinstance(value, (int, float))]
        return round(statistics.fmean(numbers), 1) if numbers else None

    summary = {}
    for arm in ARMS:
        rows = [row for row in observations if row["arm"] == arm]
        if not rows:
            continue
        latencies = [row["latency_ms"] for row in rows]
        selections = [row["selection"] for row in rows if row["selection"]]
        summary[arm] = {
            "turns": len(rows),
            "completed": sum(row["completed"] for row in rows),
            "latency_ms_mean": mean(latencies),
            "latency_ms_median": round(statistics.median(latencies), 1),
            "first_correct_tool_call_ms_mean": mean(
                [row["first_correct_tool_call_ms"] for row in rows]
            ),
            "model_calls_mean": mean([row["model_calls"] for row in rows]),
            "input_tokens_mean": mean([row["input_tokens"] for row in rows]),
            "cached_input_tokens_mean": mean([row["cached_input_tokens"] for row in rows]),
            "output_tokens_mean": mean([row["output_tokens"] for row in rows]),
            "tool_search_calls_total": sum(row["tool_search_calls"] for row in rows),
            "tool_describe_calls_total": sum(row["tool_describe_calls"] for row in rows),
            "turns_with_discovery": sum(
                1 for row in rows if row["tool_search_calls"] or row["tool_describe_calls"]
            ),
            "selection_latency_ms_mean": mean([entry["latency_ms"] for entry in selections]),
            "selection_failures": [entry["failed"] for entry in selections if entry["failed"]],
        }
    return summary


def print_table(summary: dict[str, Any]) -> None:
    rows = [
        ("turns completed", lambda s: f"{s['completed']}/{s['turns']}"),
        ("time per turn, mean (ms)", lambda s: s["latency_ms_mean"]),
        ("time per turn, median (ms)", lambda s: s["latency_ms_median"]),
        ("of which selection, mean (ms)", lambda s: s["selection_latency_ms_mean"]),
        ("time to first correct tool (ms)", lambda s: s["first_correct_tool_call_ms_mean"]),
        ("model calls per turn", lambda s: s["model_calls_mean"]),
        ("input tokens per turn", lambda s: s["input_tokens_mean"]),
        ("  cached input tokens", lambda s: s["cached_input_tokens_mean"]),
        ("output tokens per turn", lambda s: s["output_tokens_mean"]),
        ("tool_search calls, total", lambda s: s["tool_search_calls_total"]),
        ("tool_describe calls, total", lambda s: s["tool_describe_calls_total"]),
        ("turns that used discovery", lambda s: s["turns_with_discovery"]),
    ]
    arms = list(summary)
    print("\n" + f"{'':32}" + "".join(f"{arm:>14}" for arm in arms))
    for label, value in rows:
        print(f"{label:32}" + "".join(f"{value(summary[arm])!s:>14}" for arm in arms))
    for arm in arms:
        if summary[arm]["selection_failures"]:
            print(f"\n{arm}: selection failed for {summary[arm]['selection_failures']}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=rb.ROOT / "target/debug/ironclaw")
    parser.add_argument("--arm", action="append", choices=list(ARMS))
    parser.add_argument("--tool-count", type=int, default=500)
    parser.add_argument("--task", action="append", choices=[task["id"] for task in rb.TASKS])
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--turn-timeout", type=float, default=180.0)
    parser.add_argument("--llm-provider", help="provider id for the main model, e.g. venice")
    parser.add_argument("--llm-model", help="model id for the main model")
    parser.add_argument("--llm-api-key-env", help="NAME of the variable holding the model key")
    parser.add_argument("--jev-endpoint", help="decisions endpoint for the jev arm (https URL)")
    parser.add_argument("--jev-model", help="model id for the jev arm")
    parser.add_argument(
        "--jev-api-key-env", default="TYPESAFE_API_KEY",
        help="NAME of the variable holding the Jev key",
    )
    args = parser.parse_args()
    args.arm = args.arm or list(ARMS)
    args.task = args.task or list(DEFAULT_TASKS)
    return args


async def async_main(args: argparse.Namespace) -> int:
    env = scrub_environment(args)
    if not any(env.get(name) for name in (*LLM_KEY_VARS, args.llm_api_key_env or "")):
        raise SystemExit(f"a live model key is required: LLM_API_KEY or one of {', '.join(LLM_KEY_VARS)}")
    if "jev" in args.arm and not env.get(args.jev_api_key_env):
        raise SystemExit(f"{args.jev_api_key_env} is required for the jev arm")
    if not args.binary.exists():
        raise SystemExit(f"binary not found: {args.binary} (run `cargo build -p ironclaw`)")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    live_qa = rb._load_live_qa()
    observations: list[dict[str, Any]] = []
    for arm in args.arm:
        observations.extend(await run_arm(args, live_qa, arm, env))
    summary = summarize(observations)
    (args.output_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print_table(summary)
    print(f"\nper-turn rows: {args.output_dir / 'observations.jsonl'}")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(async_main(parse_args())))
