#!/usr/bin/env python3
"""Run the real-model tool-disclosure benchmark against the shipping server.

The benchmark uses a loopback MCP fixture behind IronClaw's debug-only HTTP
rewrite seam. The model and agent loop are real; only tool side effects are
synthetic and recorded for deterministic scoring.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import http.client
import importlib.util
import json
import math
import os
import re
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request
import urllib.parse
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
# v3: every `oneOf` branch that is an object requires its own properties
# (see `exclusive_one_of`), so `google_calendar__create_event` accepts a
# correct `schedule`.
GENERATOR_VERSION = "tool-search-scale-v3"
OBSERVATION_SCHEMA_VERSION = 7
SUMMARY_SCHEMA_VERSION = 7
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

# Core tools other behaviour leans on (see `REBORN_TOOL_PREFETCH_ALWAYS` in
# `.env.example`): delivery, trigger creation, the extension lifecycle and
# persistent memory. The `-floor` arm advertises them on top of the four
# mandatory bridges so the floor's cost and benefit is measurable.
PREFETCH_FLOOR_EXTRAS = (
    "outbound_deliver", "outbound_delivery_targets_list", "trigger_create",
    "extension_search", "extension_install", "extension_register_hosted_mcp",
    "extension_remove", "memory_search", "memory_read", "memory_write",
    "memory_tree",
)

# Each arm fixes the runtime's tool-surface settings. `disclosure` is the
# `REBORN_TOOL_DISCLOSURE` value; `prefetch` is `REBORN_TOOL_PREFETCH`
# (None leaves it unset, which is `off`); `retrieval` is
# `REBORN_TOOL_RETRIEVAL` (None leaves it unset, which is `native`);
# `always` is `REBORN_TOOL_PREFETCH_ALWAYS`; `classifier` is
# `REBORN_TOOL_PREFETCH_CLASSIFIER` (None leaves it unset, which is
# `local`). An `opt_in` arm runs only when named with `--arm`.
ARM_SETTINGS: dict[str, dict[str, Any]] = {
    "off": {"disclosure": "off"},
    "compact": {"disclosure": "compact"},
    "signatures": {"disclosure": "signatures"},
    "namespaces": {"disclosure": "namespaces"},
    "bridged": {"disclosure": "bridged"},
    "prefetch-lexical": {
        "disclosure": "namespaces", "prefetch": "lexical", "retrieval": "native",
    },
    "prefetch-lexical-floor": {
        "disclosure": "namespaces", "prefetch": "lexical", "retrieval": "native",
        "always": PREFETCH_FLOOR_EXTRAS,
    },
    "prefetch-semantic": {
        "disclosure": "namespaces", "prefetch": "semantic", "retrieval": "hybrid",
    },
    # Jev, served by the configured provider (TypeSafe by default), chooses
    # the tools. The mode only has to be on: the ranker and thresholds belong
    # to the local classifier. Opt-in because it needs a paid key and sends
    # each opening message and the tool catalog to that provider.
    "prefetch-jev": {
        "disclosure": "namespaces", "prefetch": "lexical", "classifier": "jev",
        "opt_in": True,
    },
}
ARMS = tuple(ARM_SETTINGS)
DEFAULT_ARMS = tuple(
    arm for arm, settings in ARM_SETTINGS.items() if not settings.get("opt_in")
)

# Settings every arm owns. They are removed from the inherited environment
# so a value exported in the operator's shell cannot leak into another arm.
ARM_CONTROLLED_ENV = (
    "REBORN_TOOL_DISCLOSURE",
    "REBORN_TOOL_DISCLOSURE_PROFILE_PINS",
    "REBORN_TOOL_PREFETCH",
    "REBORN_TOOL_PREFETCH_ALWAYS",
    "REBORN_TOOL_RETRIEVAL",
    "REBORN_TOOL_PREFETCH_CLASSIFIER",
)
# Selection tuning knobs are passed through from the operator's shell (unset
# means the runtime default) and recorded with every observation.
PREFETCH_TUNING_ENV = (
    "REBORN_TOOL_PREFETCH_MAX_TOOLS",
    "REBORN_TOOL_PREFETCH_TOKEN_BUDGET",
    "REBORN_TOOL_PREFETCH_MIN_SIMILARITY",
    "REBORN_TOOL_PREFETCH_MIN_RELATIVE",
)
# The Jev classifier the `prefetch-jev` arm runs. The generated home has no
# `[tool_selection.jev]` table, so the server uses the runtime defaults
# (`crates/app/ironclaw_config/src/tool_prefetch.rs`; a unit test keeps these
# in step) unless the operator exports one of the overrides below, which the
# server reads too: this endpoint, this model, the key in this variable, and
# this timeout for one whole classification. `jev-latest` is an alias that
# moves between Jev releases, so each observation also records the model the
# server reported answering with (`served_models`, from the classifier's log
# line).
JEV_ENDPOINT = "https://api.typesafe.ai/v1/systemone"
JEV_MODEL = "jev-latest"
JEV_API_KEY_ENV = "TYPESAFE_API_KEY"
JEV_TIMEOUT_MS = 500
# Overrides passed through from the operator's shell to the server, so the
# arm can target any provider serving the same decisions API. Unset or blank
# means the default above.
JEV_ENDPOINT_OVERRIDE_ENV = "REBORN_TOOL_PREFETCH_JEV_ENDPOINT"
JEV_MODEL_OVERRIDE_ENV = "REBORN_TOOL_PREFETCH_JEV_MODEL"
JEV_API_KEY_ENV_OVERRIDE_ENV = "REBORN_TOOL_PREFETCH_JEV_API_KEY_ENV"
# Jev input price, in US dollars per million tokens: TypeSafe's published
# price for `jev-1.13.0`. Another provider may charge differently and
# publishes its own price; set this variable to record and cost with it. It
# counts input tokens only.
JEV_USD_PER_MILLION_INPUT_TOKENS = 0.042
JEV_PRICE_OVERRIDE_ENV = "TOOL_BENCHMARK_JEV_USD_PER_MILLION_INPUT_TOKENS"
EMBEDDING_PROVIDERS = ("openai", "openai_compatible")
OPENAI_EMBEDDING_BASE_URL = "https://api.openai.com/v1"

# A two-turn conversation whose second user message moves to a namespace the
# first did not mention. Each turn is scored and measured on its own.
TOPIC_DRIFT_TURNS = (
    {
        "prompt": "List the open issues in the nearai/ironclaw GitHub repository.",
        "expected": ("github__list_issues",),
    },
    {
        "prompt": (
            "Thanks. Something unrelated now: put an issue triage meeting in my "
            "Google Calendar on 2026-08-14 from 15:00 to 15:30 UTC."
        ),
        "expected": ("google_calendar__create_event",),
    },
)

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
    {
        # Turn 1 names only GitHub issues, so a turn-start selection made
        # from it has no reason to include Calendar; turn 2 then needs it.
        "id": "topic-drift",
        "turns": TOPIC_DRIFT_TURNS,
        "expected": tuple(
            name for turn in TOPIC_DRIFT_TURNS for name in turn["expected"]
        ),
    },
    {
        # The same conversation with an idle gap longer than a shortened
        # tool-selection cache lifetime between the turns. It only means
        # something once the runtime re-selects tools after the cache has
        # gone cold; until then turn 2 behaves exactly as in `topic-drift`.
        # Opt-in: run it with `--task topic-drift-idle`.
        "id": "topic-drift-idle",
        "turns": TOPIC_DRIFT_TURNS,
        "expected": tuple(
            name for turn in TOPIC_DRIFT_TURNS for name in turn["expected"]
        ),
        "idle_gap_seconds": 8.0,
        "server_env": {
            "REBORN_TOOL_PREFETCH_RESELECT": "on",
            "REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS": "5",
            "REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS": "1",
        },
        "default": False,
    },
)
TASK_IDS = tuple(task["id"] for task in TASKS)
DEFAULT_TASK_IDS = tuple(task["id"] for task in TASKS if task.get("default", True))

# Server settings a task may set (see `server_env` above). Like the arm
# settings they are removed from the inherited environment, so every task
# that does not set them runs with the runtime defaults.
TASK_CONTROLLED_ENV = (
    "REBORN_TOOL_PREFETCH_RESELECT",
    "REBORN_TOOL_PREFETCH_CACHE_LIFETIME_SECS",
    "REBORN_TOOL_PREFETCH_CACHE_MARGIN_SECS",
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


def exclusive_one_of(schema: Any) -> Any:
    """A copy of `schema` in which each `oneOf` object branch requires its properties.

    `oneOf` accepts a value only when exactly one branch matches. An object
    branch that lists properties but requires none matches every object, so
    two such branches together reject every object. The corpus schema of
    `google_calendar__create_event` has exactly this shape (`schedule` is one
    of `{recurrence}` or `{start_at, end_at}`), and the runtime's validator
    rejected every well-formed call to it. Requiring each branch's own
    properties makes the branches exclusive whenever neither property set
    contains the other, as in the corpus. The shared corpus file is left
    alone; the benchmark repairs its copy.
    """
    if isinstance(schema, list):
        return [exclusive_one_of(item) for item in schema]
    if not isinstance(schema, dict):
        return schema
    repaired = {key: exclusive_one_of(value) for key, value in schema.items()}
    branches = repaired.get("oneOf")
    if isinstance(branches, list):
        repaired["oneOf"] = [
            {**branch, "required": list(branch["properties"])}
            if isinstance(branch, dict)
            and branch.get("type") == "object"
            and isinstance(branch.get("properties"), dict)
            and branch["properties"]
            and "required" not in branch
            else branch
            for branch in branches
        ]
    return repaired


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
            "inputSchema": exclusive_one_of(tool["parameters"]),
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


def _redact_url(url: str, *, drop_query: bool = False) -> str:
    """Drop any user-info (credentials) from a URL before it is recorded,
    and with `drop_query` its query and fragment too."""
    parsed = urllib.parse.urlsplit(url)
    if drop_query and (parsed.query or parsed.fragment):
        parsed = parsed._replace(query="", fragment="")
        url = urllib.parse.urlunsplit(parsed)
    if parsed.username is None and parsed.password is None:
        return url
    host = parsed.hostname or ""
    if parsed.port is not None:
        host = f"{host}:{parsed.port}"
    return urllib.parse.urlunsplit(parsed._replace(netloc=host))


def embeddings_config(env: dict[str, str]) -> dict[str, Any]:
    """The embeddings endpoint a dense or hybrid ranker will use, for the record.

    Mirrors the runtime's own requirements (`ironclaw_llm` embeddings
    factory): a known `EMBEDDING_PROVIDER`, an `EMBEDDING_MODEL`, an
    `EMBEDDING_BASE_URL` for `openai_compatible`, and for `openai` a key in
    the variable `EMBEDDING_API_KEY_ENV` names (default `EMBEDDING_API_KEY`).
    Raises ValueError naming what is missing, so an arm that needs embeddings
    refuses to start instead of quietly running without them. The key itself
    is never recorded, only the name of the variable that holds it.
    """
    provider = (env.get("EMBEDDING_PROVIDER") or "").strip()
    model = (env.get("EMBEDDING_MODEL") or "").strip()
    base_url = (env.get("EMBEDDING_BASE_URL") or "").strip()
    key_env = (env.get("EMBEDDING_API_KEY_ENV") or "").strip() or "EMBEDDING_API_KEY"
    problems = []
    if provider not in EMBEDDING_PROVIDERS:
        problems.append(
            f"EMBEDDING_PROVIDER must be one of {', '.join(EMBEDDING_PROVIDERS)} "
            f"(got {provider or 'nothing'})"
        )
    if not model:
        problems.append("EMBEDDING_MODEL is not set")
    if provider == "openai_compatible" and not base_url:
        problems.append("EMBEDDING_BASE_URL is not set")
    if provider == "openai" and not env.get(key_env):
        problems.append(f"the OpenAI embeddings key variable {key_env} is not set")
    if problems:
        raise ValueError(
            "this arm ranks with embeddings and needs an embeddings endpoint: "
            + "; ".join(problems)
            + " (see scripts/tool_discovery_benchmark/README.md for a local "
            "text-embeddings-inference endpoint)"
        )
    dimension = (env.get("EMBEDDING_DIMENSION") or "").strip()
    return {
        "provider": provider,
        "model": model,
        "base_url": _redact_url(base_url or OPENAI_EMBEDDING_BASE_URL),
        "dimension": int(dimension) if dimension.isdigit() else None,
        "api_key_env": key_env if env.get(key_env) else None,
    }


def _env_value(env: dict[str, str], name: str) -> str | None:
    """`env[name]` trimmed, or None when it is unset or blank."""
    value = (env.get(name) or "").strip()
    return value or None


def jev_config(env: dict[str, str]) -> dict[str, Any]:
    """The Jev settings the server will run with, for the record.

    `env` is the operator's environment: the endpoint, model and key
    variable overrides (`JEV_*_OVERRIDE_ENV`), each defaulting to the
    runtime default, and the price override. Raises ValueError when the key
    variable is unset or blank, the same condition on which the server
    refuses to start, so the arm stops before any server does; and when the
    price override is not a non-negative number. There is no fallback to the
    local classifier. The key itself is never recorded, only the name of its
    variable; the endpoint is recorded without userinfo or query.
    """
    api_key_env = _env_value(env, JEV_API_KEY_ENV_OVERRIDE_ENV) or JEV_API_KEY_ENV
    if not (env.get(api_key_env) or "").strip():
        raise ValueError(
            "this arm classifies with Jev and needs its provider's API key: "
            f"{api_key_env} is not set; the arm never falls back to the "
            "local classifier (see scripts/tool_discovery_benchmark/README.md)"
        )
    raw_price = _env_value(env, JEV_PRICE_OVERRIDE_ENV)
    price = JEV_USD_PER_MILLION_INPUT_TOKENS
    if raw_price is not None:
        try:
            price = float(raw_price)
        except ValueError:
            price = -1.0
        if not math.isfinite(price) or price < 0:
            raise ValueError(
                f"{JEV_PRICE_OVERRIDE_ENV}={raw_price!r} is not a non-negative "
                "price in US dollars per million input tokens"
            )
    return {
        "endpoint": _redact_url(
            _env_value(env, JEV_ENDPOINT_OVERRIDE_ENV) or JEV_ENDPOINT,
            drop_query=True,
        ),
        "model": _env_value(env, JEV_MODEL_OVERRIDE_ENV) or JEV_MODEL,
        "api_key_env": api_key_env,
        "timeout_ms": JEV_TIMEOUT_MS,
        "usd_per_million_input_tokens": price,
    }


def jev_server_env(jev: dict[str, Any]) -> dict[str, str]:
    """The server variables that make it run the recorded Jev settings.

    Set explicitly, not inherited, so the recorded endpoint, model and key
    variable are the ones the server uses. The endpoint is the operator's
    own value: the recorded one may be redacted.
    """
    return {
        JEV_MODEL_OVERRIDE_ENV: jev["model"],
        JEV_API_KEY_ENV_OVERRIDE_ENV: jev["api_key_env"],
    }


def max_tools_sweep(arm: str, values: list[int] | None) -> list[int | None]:
    """The `--max-tools` values an arm runs: every one for a selection arm,
    and only the runtime default (None) otherwise or without the flag."""
    if not values or ARM_SETTINGS[arm].get("prefetch") is None:
        return [None]
    return list(dict.fromkeys(values))


def observation_id(
    arm: str, tool_count: int, task_id: str, repetition: int,
    max_tools: int | None = None,
) -> str:
    """The observation's id, which is also its resume key. A `--max-tools`
    value is part of it, so a sweep shares one output directory; without
    one the id is unchanged from earlier runs."""
    if max_tools is None:
        return f"{arm}:{tool_count}:{task_id}:{repetition}"
    return f"{arm}:{tool_count}:max_tools={max_tools}:{task_id}:{repetition}"


def arm_env(
    arm: str, catalogs: list[dict[str, Any]], env: dict[str, str],
    max_tools: int | None = None,
) -> tuple[dict[str, str], dict[str, Any]]:
    """The server environment an arm sets, and the settings to record with it.

    `env` is the operator's environment, read for the selection tuning knobs,
    the embeddings endpoint and the Jev key; the variables in
    `ARM_CONTROLLED_ENV` come only from the arm. Raises ValueError when the
    arm needs an embeddings endpoint or a Jev key that is not configured.
    Only an arm with a `classifier` records `config.classifier` and
    `config.jev`; their absence means the local classifier. A `max_tools`
    (`--max-tools`) sets `REBORN_TOOL_PREFETCH_MAX_TOOLS` for a selection
    arm and is recorded as `config.max_tools`; its absence means the
    runtime default (or the operator's exported value, in
    `config.prefetch_tuning`).
    """
    settings = ARM_SETTINGS[arm]
    extra_env = {"REBORN_TOOL_DISCLOSURE": settings["disclosure"]}
    prefetch = settings.get("prefetch")
    retrieval = settings.get("retrieval")
    always = tuple(settings.get("always", ()))
    if prefetch is not None:
        extra_env["REBORN_TOOL_PREFETCH"] = prefetch
    if retrieval is not None:
        extra_env["REBORN_TOOL_RETRIEVAL"] = retrieval
    if always:
        extra_env["REBORN_TOOL_PREFETCH_ALWAYS"] = ",".join(always)
    if arm == "bridged":
        extra_env["REBORN_TOOL_DISCLOSURE_PROFILE_PINS"] = json.dumps({
            "interactive_tools": [
                canonical_capability_id(catalogs, "github__get_pull_request"),
                canonical_capability_id(catalogs, "google_calendar__list_events"),
                canonical_capability_id(catalogs, "gmail__search_messages"),
            ]
        })
    embeddings = (
        embeddings_config(env) if retrieval in {"dense", "hybrid"} else None
    )
    config = {
        "disclosure": settings["disclosure"],
        "prefetch": prefetch or "off",
        "retrieval": retrieval or "native",
        "prefetch_always": list(always),
        "prefetch_tuning": (
            {name: env.get(name) for name in PREFETCH_TUNING_ENV}
            if prefetch is not None else None
        ),
        "embeddings": embeddings,
    }
    if prefetch is not None and max_tools is not None:
        extra_env["REBORN_TOOL_PREFETCH_MAX_TOOLS"] = str(max_tools)
        config["prefetch_tuning"]["REBORN_TOOL_PREFETCH_MAX_TOOLS"] = str(max_tools)
        config["max_tools"] = max_tools
    classifier = settings.get("classifier")
    if classifier is not None:
        jev = jev_config(env) if classifier == "jev" else None
        if jev is not None:
            extra_env.update(jev_server_env(jev))
            endpoint = _env_value(env, JEV_ENDPOINT_OVERRIDE_ENV)
            if endpoint is not None:
                extra_env[JEV_ENDPOINT_OVERRIDE_ENV] = endpoint
        extra_env["REBORN_TOOL_PREFETCH_CLASSIFIER"] = classifier
        config["classifier"] = classifier
        config["jev"] = jev
    return extra_env, config


# What the fixture returns for tools whose answer the model has to use.
# Every other tool returns `benchmark tool <name> completed`. A task whose
# reply must report data needs that data here: given only the completion
# line for `github__list_issues`, the model went looking for the real issues
# (the GitHub API, then installing a GitHub extension) until an
# authentication prompt or the turn timeout stopped it.
FIXTURE_RESULTS = {
    "gmail__search_messages": (
        "Project Aurora meeting is 2026-08-12 at 10:00 UTC for 30 minutes."
    ),
    "github__list_issues": (
        "2 open issues in nearai/ironclaw: #8101 \"Tool search misses calendar "
        "tools\" (label: bug); #8102 \"Document the tool prefetch settings\" "
        "(label: docs)."
    ),
    "google_calendar__create_event": (
        "Created event benchmark-event-1 in the primary calendar."
    ),
}


def fixture_result_text(name: str) -> str:
    """The text the fixture returns for a call to the MCP tool `name`."""
    return FIXTURE_RESULTS.get(name, f"benchmark tool {name} completed")


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
            text = fixture_result_text(call["name"])
            value = {"content": [{"type": "text", "text": text}]}
        else:
            return {
                "jsonrpc": "2.0", "id": request_id,
                "error": {"code": -32601, "message": "method not found"},
            }, 200
        return {"jsonrpc": "2.0", "id": request_id, "result": value}, 200


def tool_definitions_signature(body: object) -> str | None:
    """Hash the `tools` array of one model request, or None if it has none.

    The array is re-serialized compactly with key order preserved, so two
    requests hash equal exactly when their tools arrays are equal element by
    element and in the same order (whitespace aside).
    """
    if not isinstance(body, dict):
        return None
    tools = body.get("tools")
    if not isinstance(tools, list) or not tools:
        return None
    encoded = json.dumps(tools, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def tool_definition_signature_changes(requests: list[dict[str, Any]]) -> int | None:
    """Count changes in the advertised tools array between consecutive requests.

    #6986 requires the tools array to stay byte-identical across the model
    calls of one run, so a compliant run reports 0. Requests without a tools
    array (for example a tools-free side call) are skipped rather than counted
    as changes. None means no tool-bearing request was observed, so the metric
    is unknown rather than zero.
    """
    signatures = [
        request["tools_signature"]
        for request in requests
        if isinstance(request.get("tools_signature"), str)
    ]
    if not signatures:
        return None
    return sum(
        current != previous
        for previous, current in zip(signatures, signatures[1:])
    )


# The completion marker each benchmark conversation's user messages carry;
# a trailing `_T<n>` names the turn of a multi-turn task.
CONVERSATION_MARKER = re.compile(r"BENCHMARK_DONE_\w+")
TURN_SUFFIX = re.compile(r"_T\d+$")


def conversation_key(marker: str) -> str:
    """The conversation a completion marker belongs to: the marker without
    its turn suffix, so every turn of one repetition shares one key."""
    return TURN_SUFFIX.sub("", marker)


def request_conversation(body: object) -> str | None:
    """The benchmark conversation one model request belongs to, or None.

    One server runs every repetition of a case, and a repetition whose turn
    timed out keeps calling the model while the next one runs, so the relay
    sees several conversations interleaved. Each conversation's user
    messages carry its own completion marker; the first one found names the
    conversation.
    """
    if not isinstance(body, dict):
        return None
    messages = body.get("messages")
    if not isinstance(messages, list):
        return None
    for message in messages:
        if not isinstance(message, dict) or message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, list):
            content = " ".join(
                part.get("text", "") for part in content
                if isinstance(part, dict) and isinstance(part.get("text"), str)
            )
        if not isinstance(content, str):
            continue
        found = CONVERSATION_MARKER.search(content)
        if found:
            return conversation_key(found.group(0))
    return None


def own_requests(
    requests: list[dict[str, Any]], conversation: str,
) -> list[dict[str, Any]]:
    """The relayed requests of one conversation, in order.

    A request whose conversation is unknown is kept: only a request that
    names another conversation is left out.
    """
    return [
        request for request in requests
        if request.get("conversation") in (None, conversation)
    ]


# Rough characters-per-token ratio for JSON tool schemas. It is an estimate,
# not a tokenizer count: compare arms with it, not with provider billing.
SCHEMA_CHARS_PER_TOKEN = 4


def advertised_tools(body: object) -> dict[str, Any] | None:
    """Count and size the `tools` array of one model request.

    Returns the advertised tool names (OpenAI-style `function.name`, or a
    top-level `name`), how many there are, and an estimate of the schema
    tokens they add: the length of the array serialized as compact JSON,
    divided by `SCHEMA_CHARS_PER_TOKEN` and rounded up. None when the request
    has no tools array.
    """
    if not isinstance(body, dict):
        return None
    tools = body.get("tools")
    if not isinstance(tools, list) or not tools:
        return None
    names = []
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        function = tool.get("function")
        name = function.get("name") if isinstance(function, dict) else tool.get("name")
        if isinstance(name, str):
            names.append(name)
    encoded = json.dumps(tools, separators=(",", ":"), ensure_ascii=False)
    return {
        "names": names,
        "count": len(tools),
        "schema_tokens": math.ceil(len(encoded) / SCHEMA_CHARS_PER_TOKEN),
    }


def advertised_metrics(requests: list[dict[str, Any]]) -> dict[str, Any]:
    """Per-request advertised tool counts and schema-token estimates.

    Only tool-bearing requests are listed, in the order the model saw them.
    """
    bearing = [
        request for request in requests
        if isinstance(request.get("tool_count"), int)
    ]
    return {
        "tool_count_per_request": [request["tool_count"] for request in bearing],
        "schema_tokens_per_request": [
            request["tool_schema_tokens"] for request in bearing
        ],
        "schema_token_estimator": f"compact JSON chars / {SCHEMA_CHARS_PER_TOKEN}",
    }


# Tools every selection advertises (the mandatory floor) or that only serve
# discovery. They are not "used" tools for the hit rate.
SELECTION_FLOOR_TOOLS = frozenset({
    "tool_search", "tool_describe", "tool_call", "capability_info",
    "result_read", "builtin__result_read",
})
_BENCHMARK_TOOL_PREFIX = re.compile(r"^mcp[-_]benchmark[-_][a-z0-9_-]+?__(.+)$")


def tool_key(name: str) -> str:
    """Normalize a tool name so its spellings compare equal.

    The model may name a tool by its provider name, its dotted capability id
    (`mcp-benchmark-gmail.gmail__search_messages`) or the bare MCP tool name
    (`gmail__search_messages`). Dots become `__`, then a benchmark package
    prefix is dropped, leaving the MCP tool name.
    """
    normalized = name.replace(".", "__")
    match = _BENCHMARK_TOOL_PREFIX.match(normalized)
    return match.group(1) if match else normalized


def selection_metrics(
    requests: list[dict[str, Any]], calls: list[dict[str, Any]],
) -> dict[str, Any]:
    """Turn-0 selection hit rate and misses for one observation.

    The turn-0 selection is the tools array of the observation's first
    tool-bearing model request. `used_tools` are the distinct tools the model
    called, directly or as a `tool_call` target, leaving out the floor and
    discovery tools. `hit_rate` is the share of `used_tools` that were in
    the selection (None when no tool was used or no request was recorded).
    `misses` counts the calls, after turn 0, to a used tool that was not in
    the selection; such a tool is only reachable through the
    `tool_search`/`tool_call` bridges.
    """
    first = next(
        (request for request in requests if isinstance(request.get("tool_names"), list)),
        None,
    )
    selected = list(first["tool_names"]) if first is not None else None
    selected_keys = {tool_key(name) for name in selected or ()}
    used: list[str] = []
    missed: list[str] = []
    misses = 0
    for call in calls:
        key = tool_key(_attempted_target(call))
        if key in SELECTION_FLOOR_TOOLS:
            continue
        if key not in used:
            used.append(key)
        if selected is not None and key not in selected_keys:
            misses += 1
            if key not in missed:
                missed.append(key)
    hit_rate = None
    if selected is not None and used:
        hit_rate = round(
            sum(key in selected_keys for key in used) / len(used), 4
        )
    return {
        "turn0_tool_count": len(selected) if selected is not None else None,
        "turn0_tools": selected,
        "used_tools": used,
        "hit_rate": hit_rate,
        "misses": misses if selected is not None else None,
        "missed_tools": missed if selected is not None else None,
    }


# Turn-start selection writes its per-conversation facts (selection latency,
# the core-set fallback and, for Jev, the slices and input tokens) only to
# the server's debug log, on this target. None of these lines carries user
# text or keys. The server's stderr filter is `IRONCLAW_REBORN_LOG` (default
# `info`), so the prefetch arms raise this one target to `debug`.
SELECTION_LOG_TARGET = "ironclaw::reborn::tool_prefetch"
SERVER_LOG_FILTER_ENV = "IRONCLAW_REBORN_LOG"
# The file `start_reborn_server` in the live-QA runner appends the server's
# stderr to, under the directory it is given (the task group's case dir).
SERVER_STDERR_LOG = "ironclaw-reborn-serve.stderr.log"
# The messages parsed: the loop host's record of each conversation's first
# selection, and the Jev classifier's success and failure lines. A unit test
# checks they still appear in the Rust sources.
SELECTION_LOG_SELECTED = "selected the conversation's tools from its opening request"
SELECTION_LOG_JEV_SCORED = "Jev scored the candidate tools"
SELECTION_LOG_JEV_FAILED = "Jev tool classification failed"
# The loop host's record of how the fitted tool index came by its vectors
# before a selection: already stored, embedded during the fit, or missing,
# and whether a hybrid ranker's dense side fell back. Rankers without
# vectors (BM25F) write none.
SELECTION_LOG_INDEX_VECTORS = "tool index vectors at selection time"
_SELECTION_LOG_KINDS = (
    ("selected", SELECTION_LOG_SELECTED),
    ("jev_scored", SELECTION_LOG_JEV_SCORED),
    ("jev_failed", SELECTION_LOG_JEV_FAILED),
    ("index_vectors", SELECTION_LOG_INDEX_VECTORS),
)
_INDEX_VECTOR_COUNTS = ("documents", "stored", "loaded", "embedded", "missing")
_ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")


def selection_log_env(env: dict[str, str]) -> dict[str, str]:
    """The server log filter that adds the selection target at `debug`.

    Keeps the operator's `IRONCLAW_REBORN_LOG` directives (default `info`)
    except any for the selection target itself, which this one replaces.
    """
    directives = [
        directive.strip()
        for directive in (env.get(SERVER_LOG_FILTER_ENV) or "").split(",")
        if directive.strip()
        and directive.split("=", 1)[0].strip() != SELECTION_LOG_TARGET
    ]
    return {
        SERVER_LOG_FILTER_ENV: ",".join(
            [*(directives or ["info"]), f"{SELECTION_LOG_TARGET}=debug"]
        )
    }


def _log_field(fields: str, name: str) -> str | None:
    """One `name=value` field of a `tracing` fmt line, quotes removed."""
    match = re.search(
        rf'(?:^|\s){re.escape(name)}=("(?:[^"\\]|\\.)*"|\S+)', fields
    )
    if match is None:
        return None
    value = match.group(1)
    return value[1:-1] if value.startswith('"') else value


def _log_int(fields: str, name: str) -> int | None:
    value = _log_field(fields, name)
    return int(value) if value is not None and value.isdigit() else None


def parse_selection_log(text: str) -> list[dict[str, Any]]:
    """The selection events in a slice of server stderr, in order.

    Reads `tracing`'s default text format (ANSI colour codes are stripped):
    `<time> DEBUG <spans>: ironclaw::reborn::tool_prefetch: <message>
    <fields>`. Only scalar fields are read; the `chosen` and `floor` lists
    are skipped.
    """
    marker = f"{SELECTION_LOG_TARGET}: "
    events = []
    for raw in text.splitlines():
        line = _ANSI_ESCAPE.sub("", raw)
        at = line.find(marker)
        if at < 0:
            continue
        rest = line[at + len(marker):]
        for kind, message in _SELECTION_LOG_KINDS:
            if not rest.startswith(message):
                continue
            fields = rest[len(message):]
            event: dict[str, Any] = {
                "kind": kind,
                "latency_ms": _log_int(fields, "latency_ms"),
            }
            if kind == "selected":
                event["classifier"] = _log_field(fields, "classifier")
                event["fallback"] = _log_field(fields, "fallback") or None
            elif kind == "index_vectors":
                for name in _INDEX_VECTOR_COUNTS:
                    event[name] = _log_int(fields, name)
                dense_fallback = _log_field(fields, "dense_fallback")
                event["dense_fallback"] = (
                    None if dense_fallback in (None, "", "none") else dense_fallback
                )
            else:
                event["model"] = _log_field(fields, "model")
                # The models the server reported answering with,
                # comma-separated; absent on a failure line.
                served = _log_field(fields, "served_model")
                event["served_models"] = [
                    model for model in (served or "").split(",") if model
                ]
                event["slices"] = _log_int(fields, "slices")
                event["input_tokens"] = _log_int(fields, "input_tokens")
                event["error_kind"] = _log_field(fields, "error_kind")
            events.append(event)
            break
    return events


def jev_cost_usd(
    input_tokens: int | None, usd_per_million_input_tokens: float | None = None,
) -> float | None:
    """What `input_tokens` Jev input tokens cost at the recorded price
    (`config.jev.usd_per_million_input_tokens`; the default price without
    one)."""
    if input_tokens is None:
        return None
    price = (
        JEV_USD_PER_MILLION_INPUT_TOKENS
        if usd_per_million_input_tokens is None else usd_per_million_input_tokens
    )
    return round(input_tokens * price / 1_000_000, 8)


def selection_log_metrics(
    text: str, usd_per_million_input_tokens: float | None = None,
) -> dict[str, Any]:
    """Turn-0 selection facts from one observation's slice of server stderr.

    Each observation is one conversation, so it holds one first selection.
    `turn0_latency_ms` is the loop host's time for the classify call
    (including a failed Jev call before the core-set fallback).
    `core_set_fallbacks` counts selections that fell back to the core tool
    set because the classifier failed, with the failure kinds in
    `fallback_reasons`. `jev` is None unless a Jev line was seen; its
    `input_tokens` sums the successful classifications only, because a
    failure line carries no token count, and `cost_usd` is priced from them.
    `index` is None unless the turn-0 fit logged its vectors: how many of
    the corpus `documents` were already `stored` (of which `loaded` from the
    durable store), `embedded` during the fit, or still `missing`, and the
    hybrid ranker's `dense_fallback` reason (None when the dense side ran).
    """
    events = parse_selection_log(text)
    selected = [event for event in events if event["kind"] == "selected"]
    jev_events = [
        event for event in events if event["kind"] in ("jev_scored", "jev_failed")
    ]
    index_events = [event for event in events if event["kind"] == "index_vectors"]
    first = selected[0] if selected else None
    reasons = [event["fallback"] for event in selected if event["fallback"]]
    jev = None
    if jev_events:
        scored = [event for event in jev_events if event["kind"] == "jev_scored"]
        tokens = _int_values([event["input_tokens"] for event in scored])
        input_tokens = sum(tokens) if tokens else None
        slices = _int_values([event["slices"] for event in jev_events])
        jev = {
            "model": next(
                (event["model"] for event in jev_events if event["model"]), None
            ),
            "served_models": sorted({
                model for event in scored for model in event["served_models"]
            }),
            "classifications": len(jev_events),
            "failures": [
                event["error_kind"] or "unknown"
                for event in jev_events if event["kind"] == "jev_failed"
            ],
            "slices": sum(slices) if slices else None,
            "input_tokens": input_tokens,
            "cost_usd": jev_cost_usd(input_tokens, usd_per_million_input_tokens),
            "latency_ms": _int_values([event["latency_ms"] for event in jev_events]),
        }
    # The first index line is the turn-0 fit, the one this metric is about:
    # were the vectors stored ahead of the conversation, or embedded (or
    # still missing) on its critical path?
    first_index = index_events[0] if index_events else None
    index = None
    if first_index is not None:
        index = {name: first_index[name] for name in _INDEX_VECTOR_COUNTS}
        index["dense_fallback"] = first_index["dense_fallback"]
    return {
        "selections": len(selected),
        "classifier": first["classifier"] if first else None,
        "turn0_latency_ms": first["latency_ms"] if first else None,
        "core_set_fallbacks": len(reasons),
        "fallback_reasons": reasons,
        "jev": jev,
        "index": index,
    }


def read_log_since(path: Path, offset: int) -> str:
    """The text appended to `path` after byte `offset` ("" if it is absent)."""
    try:
        with path.open("rb") as handle:
            handle.seek(offset)
            return handle.read().decode("utf-8", errors="replace")
    except FileNotFoundError:
        return ""


def log_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except FileNotFoundError:
        return 0


class LlmRequestRecorder:
    """Loopback relay in front of the model endpoint that records tools hashes.

    The recorded LLM trace keeps responses but not the request's tools array,
    so the benchmark puts this relay between `ironclaw serve` and the provider.
    It forwards every request and response unchanged (streaming responses are
    relayed as they arrive) and keeps only a timestamp, the tools hash, and
    the advertised tool names, count and estimated schema tokens of each
    model request. Headers, including credentials, and message content are
    never stored.
    """

    _HOP_HEADERS = {
        "connection", "content-length", "host", "keep-alive",
        "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade",
        "accept-encoding",
    }

    def __init__(self, upstream_base_url: str) -> None:
        parsed = urllib.parse.urlsplit(upstream_base_url.rstrip("/"))
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise ValueError(f"unsupported model base URL: {upstream_base_url!r}")
        self.upstream_scheme = parsed.scheme
        self.upstream_host = parsed.hostname
        self.upstream_port = parsed.port
        self.upstream_path = parsed.path
        self.requests: list[dict[str, Any]] = []
        self._lock = threading.Lock()
        recorder = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *_args: object) -> None:
                return

            def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
                recorder.relay(self, None)

            def do_POST(self) -> None:  # noqa: N802 - stdlib callback name
                length = int(self.headers.get("content-length", "0"))
                recorder.relay(self, self.rfile.read(length))

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def base_url(self) -> str:
        # Keep the upstream path so clients that append their own suffix (for
        # example `/v1/...` when the base has none) build the same path they
        # would against the real endpoint; the relay forwards it verbatim.
        return f"http://127.0.0.1:{self.server.server_address[1]}{self.upstream_path}"

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def record(self, body: bytes) -> None:
        try:
            payload = json.loads(body)
        except (ValueError, UnicodeDecodeError):
            return
        if not isinstance(payload, dict) or "messages" not in payload:
            return
        advertised = advertised_tools(payload)
        entry = {
            "monotonic_ns": time.monotonic_ns(),
            "tools_signature": tool_definitions_signature(payload),
            "conversation": request_conversation(payload),
            "tool_names": advertised["names"] if advertised else None,
            "tool_count": advertised["count"] if advertised else None,
            "tool_schema_tokens": advertised["schema_tokens"] if advertised else None,
        }
        with self._lock:
            self.requests.append(entry)

    def relay(self, handler: BaseHTTPRequestHandler, body: bytes | None) -> None:
        if body is not None:
            self.record(body)
        connection_type = (
            http.client.HTTPSConnection
            if self.upstream_scheme == "https"
            else http.client.HTTPConnection
        )
        connection = connection_type(
            self.upstream_host, self.upstream_port, timeout=300
        )
        headers = {
            key: value
            for key, value in handler.headers.items()
            if key.lower() not in self._HOP_HEADERS
        }
        if body is not None:
            headers["Content-Length"] = str(len(body))
        try:
            connection.request(
                handler.command, handler.path, body=body,
                headers=headers,
            )
            response = connection.getresponse()
            handler.send_response(response.status, response.reason)
            for key, value in response.getheaders():
                if key.lower() not in self._HOP_HEADERS:
                    handler.send_header(key, value)
            handler.send_header("Connection", "close")
            handler.end_headers()
            while True:
                chunk = response.read1(65536)
                if not chunk:
                    break
                handler.wfile.write(chunk)
                handler.wfile.flush()
        except OSError as exc:
            # Relay boundary: surface upstream transport failures as a 502 so
            # the server reports a provider error instead of hanging.
            try:
                handler.send_error(502, f"upstream model request failed: {type(exc).__name__}")
            except OSError:
                pass
        finally:
            connection.close()


def resolve_model_base_url(env: dict[str, str]) -> str | None:
    """Mirror the live-QA config writer's choice of model base URL."""
    base_url = env.get("REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL")
    provider_id = env.get("REBORN_WEBUI_V2_LIVE_QA_LLM_PROVIDER_ID", "nearai")
    if provider_id != "nearai" and not base_url:
        base_url = env.get("LIVE_OPENAI_COMPATIBLE_BASE_URL", "https://cloud-api.near.ai/v1")
    return base_url or None


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
    if task_id in TOPIC_DRIFT_TASK_IDS:
        return len(arguments) == len(TOPIC_DRIFT_TURNS) and all(
            _topic_drift_turn_arguments_are_valid(turn, turn_arguments)
            for turn, turn_arguments in enumerate(arguments)
        )
    return True


TOPIC_DRIFT_TASK_IDS = frozenset({"topic-drift", "topic-drift-idle"})


def _topic_drift_turn_arguments_are_valid(
    turn: int, arguments: dict[str, Any],
) -> bool:
    if turn == 0:
        return arguments.get("owner") == "nearai" and arguments.get("repo") == "ironclaw"
    schedule = arguments.get("schedule")
    return (
        isinstance(schedule, dict)
        and _contains_casefold(schedule.get("start_at"), "2026-08-14")
        and _contains_casefold(schedule.get("start_at"), "15:00")
        and _contains_casefold(schedule.get("end_at"), "2026-08-14")
        and _contains_casefold(schedule.get("end_at"), "15:30")
    )


def score_turn(
    task: dict[str, Any], turn: int, calls: list[dict[str, Any]],
) -> dict[str, Any]:
    """Score one turn of a multi-turn task against that turn's expected tools.

    `calls` are the fixture calls made during the turn. The turn completed
    when its expected tools were called in order with valid arguments.
    """
    expected = list(task["turns"][turn]["expected"])
    called = [call["name"] for call in calls]
    ordered = _ordered_expected_calls(expected, calls)
    completed = ordered is not None
    if ordered is not None and task["id"] in TOPIC_DRIFT_TASK_IDS:
        completed = all(
            _topic_drift_turn_arguments_are_valid(turn, call.get("arguments", {}))
            for call in ordered
        )
    return {
        "completed": completed,
        "correct_tool_recalled": all(name in called for name in expected),
        "expected_tools": expected,
        "called_tools": called,
    }


def _attempted_target(call: dict[str, Any]) -> str:
    name = str(call.get("name") or "")
    if name != "tool_call":
        return name
    arguments = call.get("arguments")
    if not isinstance(arguments, dict) or not isinstance(arguments.get("name"), str):
        return name
    return arguments["name"]


def _target_arguments(call: dict[str, Any]) -> dict[str, Any]:
    """The arguments the called capability receives.

    A direct call's own arguments; for the `tool_call` bridge, its inner
    `arguments`, which the model may send as an object or a JSON string.
    """
    arguments = call.get("arguments")
    if not isinstance(arguments, dict):
        return {}
    if str(call.get("name") or "") != "tool_call":
        return arguments
    inner = arguments.get("arguments")
    if isinstance(inner, str):
        try:
            inner = json.loads(inner)
        except ValueError:
            return {}
    return inner if isinstance(inner, dict) else {}


# Model tools that only serve discovery. A `result_read` of one of their
# results is discovery too (see `is_discovery_call`).
DISCOVERY_TOOL_NAMES = frozenset({"tool_search", "tool_describe", "capability_info"})
RESULT_READ_CAPABILITY_ID = "builtin.result_read"


def _is_result_read_name(name: str) -> bool:
    return name.replace(".", "__") == RESULT_READ_CAPABILITY_ID.replace(".", "__")


def is_result_read(call: dict[str, Any]) -> bool:
    """Whether a model call is `builtin.result_read`, direct or bridged.

    The model sees the capability as the provider tool `builtin__result_read`;
    under the bridged arm it may instead name it as the `tool_call` target in
    either spelling.
    """
    return _is_result_read_name(_attempted_target(call))


def _matches_forbidden(target: str, forbidden: tuple[str, ...]) -> bool:
    normalized = target.replace('.', '__')
    return any(
        normalized == item or normalized.endswith(f"__{item}")
        for item in forbidden
    )


def is_discovery_call(call: dict[str, Any]) -> bool:
    """Whether a model tool call only served discovery.

    Discovery is a call to `tool_search`, `tool_describe` or
    `capability_info`, or a `result_read` (direct or through the `tool_call`
    bridge) of one of their results: a long search result comes back as a
    reference that the model pages through with `result_read`. A read whose
    source could not be resolved from the trace (`reads_result_of` is None
    or absent) also counts as discovery; a read of a real tool's result does
    not.
    """
    if str(call.get("name")) in DISCOVERY_TOOL_NAMES:
        return True
    if not is_result_read(call):
        return False
    source = call.get("reads_result_of")
    return not isinstance(source, str) or tool_key(source) in DISCOVERY_TOOL_NAMES


def score_task(
    task: dict[str, Any],
    calls: list[dict[str, Any]],
    attempted_calls: list[dict[str, Any]],
) -> dict[str, Any]:
    """Score one observation of a task.

    `calls` are the fixture (MCP) calls that reached the benchmark server;
    `attempted_calls` are every model tool call from the trace. A task with
    expected tools completes when they were called in order with valid
    arguments; a no-tool task completes when nothing reached the fixture and
    every attempted call was discovery (see `is_discovery_call`).
    """
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
        non_discovery_attempts = [
            call for call in attempted_calls if not is_discovery_call(call)
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


def _int_values(values: list[object]) -> list[int]:
    return [
        value for value in values
        if isinstance(value, int) and not isinstance(value, bool)
    ]


def _mean(values: list[int]) -> float | None:
    return round(statistics.fmean(values), 2) if values else None


def _median(values: list[int]) -> float | None:
    return statistics.median(values) if values else None


def _quintile_triple(values: list[int]) -> list[float] | None:
    """20th percentile, median and 80th percentile, as the baseline doc reports.

    Uses `statistics.quantiles(values, n=5, method="inclusive")` (linear
    interpolation); a single value is its own triple.
    """
    if not values:
        return None
    if len(values) == 1:
        return [float(values[0])] * 3
    cuts = statistics.quantiles(values, n=5, method="inclusive")
    return [
        round(cuts[0], 2), round(float(statistics.median(values)), 2),
        round(cuts[3], 2),
    ]


def _selection_aggregates(items: list[dict[str, Any]]) -> dict[str, object]:
    tool_counts: list[int] = []
    schema_tokens: list[int] = []
    for item in items:
        advertised = item.get("advertised") or {}
        tool_counts += _int_values(list(advertised.get("tool_count_per_request") or []))
        schema_tokens += _int_values(
            list(advertised.get("schema_tokens_per_request") or [])
        )
    selections = [item.get("selection") or {} for item in items]
    hit_rates = [
        selection["hit_rate"] for selection in selections
        if isinstance(selection.get("hit_rate"), (int, float))
        and not isinstance(selection.get("hit_rate"), bool)
    ]
    turn0_counts = _int_values([selection.get("turn0_tool_count") for selection in selections])
    misses = _int_values([selection.get("misses") for selection in selections])
    return {
        "advertised_requests": len(tool_counts),
        "advertised_tool_count_p20_median_p80": _quintile_triple(tool_counts),
        "advertised_tool_count_max": max(tool_counts) if tool_counts else None,
        "advertised_schema_tokens_p20_median_p80": _quintile_triple(schema_tokens),
        "advertised_schema_tokens_max": max(schema_tokens) if schema_tokens else None,
        "turn0_tool_count_p20_median_p80": _quintile_triple(turn0_counts),
        "selection_hit_rate_observations": len(hit_rates),
        "selection_hit_rate_mean": (
            round(statistics.fmean(hit_rates), 4) if hit_rates else None
        ),
        "selection_full_hit_observations": (
            sum(rate == 1 for rate in hit_rates) if hit_rates else None
        ),
        "selection_misses_total": sum(misses) if misses else None,
        "observations_with_selection_misses": (
            sum(value > 0 for value in misses) if misses else None
        ),
    }


def _selection_log_aggregates(items: list[dict[str, Any]]) -> dict[str, object]:
    """Turn-0 selection latency, core-set fallbacks and Jev usage per arm.

    Counts only observations whose server log showed a first selection
    (`selection_log_observations`); the Jev figures only those with a Jev
    line (`jev_observations`); the index figures only those whose turn-0 fit
    logged its vectors (`index_observations`: documents embedded or missing
    on the turn's critical path, and how often the dense side fell back).
    All are null for arms without selection.
    """
    logs = [
        item["selection_log"] for item in items
        if isinstance(item.get("selection_log"), dict)
    ]
    with_selection = [log for log in logs if log.get("selections")]
    latencies = _int_values([log.get("turn0_latency_ms") for log in with_selection])
    fallbacks = sum(bool(log.get("core_set_fallbacks")) for log in with_selection)
    reasons: dict[str, int] = {}
    for log in with_selection:
        for reason in log.get("fallback_reasons") or []:
            reasons[reason] = reasons.get(reason, 0) + 1
    indexes = [log["index"] for log in logs if isinstance(log.get("index"), dict)]
    index_fallbacks: dict[str, int] = {}
    for index in indexes:
        if index.get("dense_fallback"):
            reason = index["dense_fallback"]
            index_fallbacks[reason] = index_fallbacks.get(reason, 0) + 1
    embedded_at_selection = _int_values([index.get("embedded") for index in indexes])
    missing_at_selection = _int_values([index.get("missing") for index in indexes])
    jevs = [log["jev"] for log in logs if isinstance(log.get("jev"), dict)]
    slices = _int_values([jev.get("slices") for jev in jevs])
    tokens = _int_values([jev.get("input_tokens") for jev in jevs])
    costs = [
        jev["cost_usd"] for jev in jevs
        if isinstance(jev.get("cost_usd"), (int, float))
        and not isinstance(jev.get("cost_usd"), bool)
    ]
    # The models the server reported answering with, not the requested
    # model: with an alias such as `jev-latest` only these say which release
    # answered. The requested model is in each observation's `config.jev`.
    models = sorted({
        model
        for jev in jevs
        for model in jev.get("served_models") or []
        if isinstance(model, str)
    })
    return {
        "selection_log_observations": len(with_selection),
        "turn0_selection_latency_ms_p20_median_p80": _quintile_triple(latencies),
        "turn0_selection_latency_ms_max": max(latencies) if latencies else None,
        "core_set_fallback_observations": fallbacks if with_selection else None,
        "core_set_fallback_rate": (
            round(fallbacks / len(with_selection), 4) if with_selection else None
        ),
        "core_set_fallback_reasons": reasons if with_selection else None,
        "index_observations": len(indexes),
        "index_embedded_at_selection_total": (
            sum(embedded_at_selection) if embedded_at_selection else None
        ),
        "index_missing_at_selection_total": (
            sum(missing_at_selection) if missing_at_selection else None
        ),
        "index_dense_fallback_rate": (
            round(sum(index_fallbacks.values()) / len(indexes), 4) if indexes else None
        ),
        "index_dense_fallback_reasons": index_fallbacks if indexes else None,
        "jev_observations": len(jevs),
        "jev_models": models or None,
        "jev_slices_per_conversation_p20_median_p80": _quintile_triple(slices),
        "jev_slices_per_conversation_max": max(slices) if slices else None,
        "jev_input_tokens_total": sum(tokens) if tokens else None,
        "jev_input_tokens_mean": _mean(tokens),
        "jev_cost_usd_total": round(sum(costs), 8) if costs else None,
        "jev_cost_usd_per_conversation_mean": (
            round(statistics.fmean(costs), 8) if costs else None
        ),
    }


def _reported_token_counts(
    items: list[dict[str, Any]],
) -> tuple[list[int], list[int]]:
    """Input and cached-input tokens, only where the provider reported usage.

    A model call always consumes input, so an input count of zero (or null)
    means the provider reported no usage for that observation; it is left
    out rather than averaged in as zero. Cached input is taken only from
    observations whose input was reported. Some providers report usage but
    not cache reads, which the trace records as 0; the summary's
    `provider_usage_available` and the doc must say which case applies.
    """
    inputs: list[int] = []
    cached: list[int] = []
    for item in items:
        tokens = item.get("tokens") or {}
        value = tokens.get("input")
        if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
            continue
        inputs.append(value)
        cached_value = tokens.get("cached_input")
        if isinstance(cached_value, int) and not isinstance(cached_value, bool):
            cached.append(cached_value)
    return inputs, cached


def _efficiency_aggregates(items: list[dict[str, Any]]) -> dict[str, object]:
    def counts(key: str) -> list[int]:
        return _int_values([(item.get("counts") or {}).get(key) for item in items])

    model_turns = counts("model_turns")
    tool_search = counts("tool_search_calls")
    tool_describe = counts("tool_describe_calls")
    result_read = counts("result_read_calls")
    discovery_reads = counts("discovery_result_read_calls")
    bridged = counts("bridged_tool_calls")
    inputs, cached = _reported_token_counts(items)
    first_correct = _int_values([
        (item.get("latency_ms") or {}).get("time_to_first_correct_tool_call")
        for item in items
    ])
    signature_changes = _int_values([
        (item.get("cache") or {}).get("tool_definition_signature_changes")
        for item in items
    ])
    return {
        "model_turns_mean": _mean(model_turns),
        "model_turns_median": _median(model_turns),
        "model_turns_max": max(model_turns) if model_turns else None,
        "tool_search_calls_total": sum(tool_search) if tool_search else None,
        "tool_search_calls_mean": _mean(tool_search),
        "tool_describe_calls_total": sum(tool_describe) if tool_describe else None,
        "tool_describe_calls_mean": _mean(tool_describe),
        "result_read_calls_total": sum(result_read) if result_read else None,
        "discovery_result_read_calls_total": (
            sum(discovery_reads) if discovery_reads else None
        ),
        "bridged_tool_calls_total": sum(bridged) if bridged else None,
        "token_usage_observations": len(inputs),
        "input_tokens_mean": _mean(inputs),
        "input_tokens_median": _median(inputs),
        "cached_input_tokens_mean": _mean(cached),
        "cached_input_tokens_median": _median(cached),
        "first_correct_tool_observations": len(first_correct),
        "time_to_first_correct_tool_ms_median": _median(first_correct),
        "time_to_first_correct_tool_ms_worst": (
            max(first_correct) if first_correct else None
        ),
        "tool_definition_signature_changes_total": (
            sum(signature_changes) if signature_changes else None
        ),
        "observations_with_tool_definition_changes": (
            sum(value > 0 for value in signature_changes)
            if signature_changes else None
        ),
    }


def _max_tools(observation: dict[str, Any]) -> int | None:
    """The observation's `--max-tools` value; None is the runtime default."""
    return (observation.get("config") or {}).get("max_tools")


def _group_order(key: tuple[Any, ...]) -> tuple[Any, ...]:
    """Sort key for (arm, tool_count, max_tools, ...): the runtime default
    (None) first, then the swept values in order."""
    arm, tool_count, max_tools, *rest = key
    return (arm, tool_count, -1 if max_tools is None else max_tools, *rest)


def aggregate_observations(observations: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Aggregates by arm, catalog size and `--max-tools` value."""
    groups: dict[tuple[str, int, int | None], list[dict[str, Any]]] = {}
    for observation in observations:
        key = (
            observation["arm"], observation["catalog"]["tool_count"],
            _max_tools(observation),
        )
        groups.setdefault(key, []).append(observation)
    aggregates = []
    for (arm, tool_count, max_tools), items in sorted(
        groups.items(), key=lambda group: _group_order(group[0]),
    ):
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
            "max_tools": max_tools,
            "observations": len(items),
            "completion_rate": completed / len(items),
            "latency_ms_median": statistics.median(latencies),
            "latency_ms_worst": max(latencies),
            "latency_ms_spread": max(latencies) - min(latencies),
            "unauthorized_tool_leaks": sum(
                item["task"]["unauthorized_tool_leaks"] for item in items
            ),
            "failure_categories": failure_categories,
            "config": items[0].get("config"),
            "config_consistent": all(
                item.get("config") == items[0].get("config") for item in items
            ),
            **_efficiency_aggregates(items),
            **_selection_aggregates(items),
            **_selection_log_aggregates(items),
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
    return metrics, trace_tool_calls(payload)


def _result_ref(content: object) -> str | None:
    """The `detail.result_ref` of one recorded tool result, if it has one."""
    if not isinstance(content, str):
        return None
    try:
        payload = json.loads(content)
    except ValueError:
        return None
    detail = payload.get("detail") if isinstance(payload, dict) else None
    ref = detail.get("result_ref") if isinstance(detail, dict) else None
    return ref if isinstance(ref, str) and ref else None


def trace_tool_calls(payload: object) -> list[dict[str, Any]]:
    """The model's tool calls in one LLM trace, in trace order.

    Each call carries its `name`, the index of its step (`model_turn`) and
    its `arguments`. A `result_read` call also carries `reads_result_of`:
    the tool whose result it reads, or None when the trace does not say.

    The source is resolved through the result reference. A tool result that
    was too long to inline is recorded (in the `expected_tool_results` of a
    later step) with a `detail.result_ref`; the read names that reference in
    its `result_ref` argument. The producer is the first non-`result_read`
    call whose recorded result carries the reference, named by its target
    (so a bridged call resolves to the tool it called). The whole trace is
    indexed before any read is resolved, because a trace can interleave the
    steps of a conversation that outlived its repetition with the next one.
    A producer step whose results were not recorded leaves the read
    unresolved.
    """
    steps = payload.get("steps") if isinstance(payload, dict) else None
    if not isinstance(steps, list):
        return []
    calls: list[dict[str, Any]] = []
    by_id: dict[str, dict[str, Any]] = {}
    for model_turn, step in enumerate(steps):
        response = step.get("response") if isinstance(step, dict) else None
        if not isinstance(response, dict):
            continue
        for call in response.get("tool_calls") or []:
            if not isinstance(call, dict) or not isinstance(call.get("name"), str):
                continue
            entry = {
                "name": call["name"],
                "model_turn": model_turn,
                "arguments": call.get("arguments")
                if isinstance(call.get("arguments"), dict) else {},
            }
            calls.append(entry)
            if isinstance(call.get("id"), str):
                by_id.setdefault(call["id"], entry)
    producers: dict[str, str] = {}
    for step in steps:
        results = step.get("expected_tool_results") if isinstance(step, dict) else None
        for result in results if isinstance(results, list) else []:
            if not isinstance(result, dict):
                continue
            ref = _result_ref(result.get("content"))
            if ref is None or ref in producers:
                continue
            producer = by_id.get(result.get("tool_call_id"))  # type: ignore[arg-type]
            source = (
                _attempted_target(producer) if producer is not None
                else result.get("name")
            )
            if isinstance(source, str) and source and not _is_result_read_name(source):
                producers[ref] = source
    for entry in calls:
        if is_result_read(entry):
            ref = _target_arguments(entry).get("result_ref")
            entry["reads_result_of"] = producers.get(ref) if isinstance(ref, str) else None
    return calls


TRACE_BOUNDARY_READ_ATTEMPTS = 3
TRACE_BOUNDARY_READ_RETRY_SECONDS = 0.5


async def _trace_call_count_at_boundary(
    live_qa: Any, trace_path: Path, prior_count: int,
) -> int | None:
    """How many model tool calls this observation's trace holds so far.

    The server flushes the trace after every model step, so by the time a
    turn's reply is visible its tool calls are on disk. A read that lands
    mid-write is retried; if the trace still cannot be read the count is
    None, and the trace-based metrics of the turns this boundary splits are
    reported as unknown rather than guessed.
    """
    for attempt in range(TRACE_BOUNDARY_READ_ATTEMPTS):
        try:
            _, calls = _trace_metrics(live_qa, trace_path)
            return len(calls) - prior_count
        except (OSError, ValueError, RuntimeError) as exc:
            # RuntimeError covers the live-QA LiveQaError for a bad trace.
            if attempt + 1 == TRACE_BOUNDARY_READ_ATTEMPTS:
                print(
                    "[tool-benchmark] warning: could not read the trace at a "
                    f"turn boundary ({type(exc).__name__}: {exc}); per-turn "
                    "trace metrics will be null", flush=True,
                )
                return None
            await asyncio.sleep(TRACE_BOUNDARY_READ_RETRY_SECONDS)
    return None


def result_read_call_count(calls: list[dict[str, Any]]) -> int:
    """Count model calls to `builtin.result_read`, direct or through `tool_call`.

    Every spelling (see `is_result_read`) counts once per model call.
    """
    return sum(is_result_read(call) for call in calls)


def discovery_result_read_call_count(calls: list[dict[str, Any]]) -> int:
    """Count the `result_read` calls that read a discovery result.

    They are a subset of `result_read_call_count`: real round trips, but
    scored as discovery (see `is_discovery_call`). An unresolved read counts
    here, as it does in scoring.
    """
    return sum(is_result_read(call) and is_discovery_call(call) for call in calls)


def bridged_tool_call_count(calls: list[dict[str, Any]]) -> int:
    """Count model calls routed through the `tool_call` bridge.

    Each call counts once whatever its target, including targets that do not
    resolve; the target itself is scored separately.
    """
    return sum(call.get("name") == "tool_call" for call in calls)


def discovery_turn_count(calls: list[dict[str, Any]]) -> int:
    return len({
        call["model_turn"]
        for call in calls
        if call.get("name") in DISCOVERY_TOOL_NAMES
        and isinstance(call.get("model_turn"), int)
    })


def run_cache_metadata(
    repetitions: list[int], group_position: int, repetition: int,
) -> dict[str, object]:
    return {
        "thermal_class": "cold" if group_position == 0 else "warm",
        "repetition": repetition,
        "resumed_group": bool(repetitions and repetitions[0] != 0),
    }


def _last_signature(requests: list[dict[str, Any]]) -> str | None:
    signatures = [
        request["tools_signature"] for request in requests
        if isinstance(request.get("tools_signature"), str)
    ]
    return signatures[-1] if signatures else None


def _first_signature(requests: list[dict[str, Any]]) -> str | None:
    return next(
        (
            request["tools_signature"] for request in requests
            if isinstance(request.get("tools_signature"), str)
        ),
        None,
    )


def _turn_selection(
    requests: list[dict[str, Any]] | None, trace_calls: list[dict[str, Any]] | None,
) -> dict[str, Any] | None:
    """`selection_metrics` for one turn, measured against that turn's start.

    The selection is the tools array of the turn's first tool-bearing
    request. Without re-selection it is the conversation's turn-0
    selection; once the runtime re-selects at a turn start, it is the new one.
    """
    if requests is None or trace_calls is None:
        return None
    metrics = selection_metrics(requests, trace_calls)
    return {
        "turn_start_tool_count": metrics["turn0_tool_count"],
        "used_tools": metrics["used_tools"],
        "hit_rate": metrics["hit_rate"],
        "misses": metrics["misses"],
        "missed_tools": metrics["missed_tools"],
    }


def turn_metrics(
    task: dict[str, Any],
    marks: list[dict[str, Any]],
    calls: list[dict[str, Any]],
    requests: list[dict[str, Any]] | None,
    trace_calls: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Per-turn metrics for one observation of a multi-turn task.

    `marks` has one entry per turn that was started, in order, shaped
    `{"start": mark, "end": mark | None}`. A mark is a snapshot taken at a
    turn boundary: how many fixture `calls`, relay `requests` and model
    `trace_calls` the observation had by then, and the `monotonic` clock in
    seconds. `end` is None when the turn got no reply, and the turn then
    runs to the end of the lists. A mark's `trace_calls` is None when the
    trace could not be read at that boundary, which makes the trace-based
    metrics of the turns it bounds unknown. `requests` is None when the
    request relay was off.

    Each turn reports its tool calls, its `tool_search` and bridged
    `tool_call` counts, its selection misses (calls to tools the turn's
    first request did not advertise, which only `tool_search` and the
    `tool_call` bridge can reach), the time from the turn's start to its
    first correct tool, and how often the advertised tools changed within the
    turn. `tools_changed_at_turn_start` says whether a later turn's first
    tools array differs from the last one of the turns before it; without an
    idle gap it must be false, because the advertised tools must not change
    within a conversation.
    """
    results = []
    for turn, spec in enumerate(task["turns"]):
        expected = tuple(spec["expected"])
        if turn >= len(marks):
            results.append({
                "turn": turn,
                "started": False,
                "replied": False,
                "completed": False,
                "correct_tool_recalled": False,
                "expected_tools": list(expected),
                "called_tools": [],
                "tool_calls": None,
                "synthetic_tool_calls": None,
                "tool_search_calls": None,
                "bridged_tool_calls": None,
                "selection": None,
                "time_to_first_correct_tool_call_ms": None,
                "tool_bearing_model_requests": None,
                "tool_definition_signature_changes": None,
                "tools_changed_at_turn_start": None,
            })
            continue
        start = marks[turn]["start"]
        end = marks[turn].get("end")
        turn_calls = calls[start["calls"]:end["calls"] if end else None]
        trace_start = start.get("trace_calls")
        trace_end = end.get("trace_calls") if end else len(trace_calls)
        turn_trace = (
            trace_calls[trace_start:trace_end]
            if trace_start is not None and trace_end is not None else None
        )
        turn_requests = (
            requests[start["requests"]:end["requests"] if end else None]
            if requests is not None else None
        )
        changed_at_start = None
        if turn > 0 and requests is not None:
            before = _last_signature(requests[:start["requests"]])
            first = _first_signature(turn_requests or [])
            if before is not None and first is not None:
                changed_at_start = before != first
        results.append({
            "turn": turn,
            "started": True,
            "replied": end is not None,
            **score_turn(task, turn, turn_calls),
            "tool_calls": len(turn_trace) if turn_trace is not None else None,
            "synthetic_tool_calls": len(turn_calls),
            "tool_search_calls": (
                sum(call.get("name") == "tool_search" for call in turn_trace)
                if turn_trace is not None else None
            ),
            "bridged_tool_calls": (
                bridged_tool_call_count(turn_trace) if turn_trace is not None else None
            ),
            "selection": _turn_selection(turn_requests, turn_trace),
            "time_to_first_correct_tool_call_ms": first_correct_tool_call_latency_ms(
                expected, turn_calls, start["monotonic"]
            ),
            "tool_bearing_model_requests": (
                sum(
                    isinstance(request.get("tools_signature"), str)
                    for request in turn_requests
                )
                if turn_requests is not None else None
            ),
            "tool_definition_signature_changes": (
                tool_definition_signature_changes(turn_requests)
                if turn_requests is not None else None
            ),
            "tools_changed_at_turn_start": changed_at_start,
        })
    return results


def _turn_aggregate(turn: int, items: list[dict[str, Any]]) -> dict[str, object]:
    turns = [
        item["turns"][turn] for item in items
        if len(item.get("turns") or []) > turn
    ]
    selections = [entry.get("selection") or {} for entry in turns]
    tool_calls = _int_values([entry.get("tool_calls") for entry in turns])
    tool_search = _int_values([entry.get("tool_search_calls") for entry in turns])
    bridged = _int_values([entry.get("bridged_tool_calls") for entry in turns])
    misses = _int_values([selection.get("misses") for selection in selections])
    first_correct = _int_values([
        entry.get("time_to_first_correct_tool_call_ms") for entry in turns
    ])
    signature_changes = _int_values([
        entry.get("tool_definition_signature_changes") for entry in turns
    ])
    changed_at_start = [
        entry["tools_changed_at_turn_start"] for entry in turns
        if isinstance(entry.get("tools_changed_at_turn_start"), bool)
    ]
    return {
        "turn": turn,
        "started": sum(bool(entry.get("started")) for entry in turns),
        "replied": sum(bool(entry.get("replied")) for entry in turns),
        "completion_rate": (
            sum(bool(entry.get("completed")) for entry in turns) / len(items)
            if items else None
        ),
        "tool_calls_total": sum(tool_calls) if tool_calls else None,
        "tool_calls_mean": _mean(tool_calls),
        "tool_search_calls_total": sum(tool_search) if tool_search else None,
        "bridged_tool_calls_total": sum(bridged) if bridged else None,
        "selection_misses_total": sum(misses) if misses else None,
        "observations_with_selection_misses": (
            sum(value > 0 for value in misses) if misses else None
        ),
        "first_correct_tool_observations": len(first_correct),
        "time_to_first_correct_tool_ms_median": _median(first_correct),
        "time_to_first_correct_tool_ms_worst": (
            max(first_correct) if first_correct else None
        ),
        "tool_definition_signature_changes_total": (
            sum(signature_changes) if signature_changes else None
        ),
        "tools_changed_at_turn_start_observations": (
            sum(changed_at_start) if changed_at_start else None
        ),
    }


def aggregate_turns(observations: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Per-turn aggregates of the multi-turn tasks, by arm, size and task.

    `tool_definition_signature_changes_total` spans whole conversations, so
    it also counts a change at a turn boundary. It must be 0 for a task
    without an idle gap.
    """
    groups: dict[tuple[str, int, int | None, str], list[dict[str, Any]]] = {}
    for observation in observations:
        if not isinstance(observation.get("turns"), list):
            continue
        key = (
            observation["arm"], observation["catalog"]["tool_count"],
            _max_tools(observation), observation["task"]["id"],
        )
        groups.setdefault(key, []).append(observation)
    aggregates = []
    for (arm, tool_count, max_tools, task_id), items in sorted(
        groups.items(), key=lambda group: _group_order(group[0]),
    ):
        signature_changes = _int_values([
            (item.get("cache") or {}).get("tool_definition_signature_changes")
            for item in items
        ])
        idle_gap = items[0].get("idle_gap") or {}
        turn_count = max(len(item["turns"]) for item in items)
        aggregates.append({
            "arm": arm,
            "tool_count": tool_count,
            "max_tools": max_tools,
            "task": task_id,
            "observations": len(items),
            "idle_gap_seconds": idle_gap.get("seconds"),
            "tool_definition_signature_changes_total": (
                sum(signature_changes) if signature_changes else None
            ),
            "observations_with_tool_definition_changes": (
                sum(value > 0 for value in signature_changes)
                if signature_changes else None
            ),
            "turns": [_turn_aggregate(turn, items) for turn in range(turn_count)],
        })
    return aggregates


def task_turns(
    task: dict[str, Any], catalogs: list[dict[str, Any]], case_name: str,
) -> list[tuple[str, str]]:
    """The (prompt, marker) of each user message the task sends.

    A single-turn task sends its `prompt`; a multi-turn task sends each of
    its `turns` in order, with a marker per turn so each reply is awaited on
    its own.
    """
    marker = f"BENCHMARK_DONE_{case_name}".replace("-", "_")
    if "turns" not in task:
        canonical = (
            canonical_capability_id(catalogs, task["expected"][0])
            if task["expected"] else ""
        )
        return [(task["prompt"].format(canonical=canonical), marker)]
    return [
        (turn["prompt"], f"{marker}_T{index + 1}")
        for index, turn in enumerate(task["turns"])
    ]


def _with_marker_instruction(prompt: str, marker: str) -> str:
    return (
        f"{prompt}\n\nAfter completing the task, end your final response "
        f"with exactly: {marker}"
    )


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
    request_recorder: LlmRequestRecorder | None = None,
    max_tools: int | None = None,
) -> list[dict[str, Any]]:
    group_name = (
        f"{arm}-{tool_count}-{task['id']}" if max_tools is None
        else f"{arm}-{tool_count}-max{max_tools}-{task['id']}"
    )
    catalogs = generate_catalog(tool_count)
    # Resolve the arm first: an arm missing its embeddings endpoint must fail
    # here, before a home, fixture or server exists.
    arm_extra_env, arm_config = arm_env(arm, catalogs, dict(os.environ), max_tools)
    # Selection latency, fallbacks and Jev usage are only in the server's
    # debug log, so the selection arms raise that one target.
    reads_selection_log = arm_config["prefetch"] != "off"
    if reads_selection_log:
        arm_extra_env.update(selection_log_env(dict(os.environ)))
    case_dir = output_dir / "cases" / group_name
    server_log = case_dir / SERVER_STDERR_LOG
    home = live_qa.create_generated_reborn_home(case_dir / "source-home")
    fixture = McpFixture(catalogs)
    fixture.start()
    trace_env = live_qa.case_llm_trace_env(output_dir, group_name)
    extra_env = {
        **arm_extra_env,
        **dict(task.get("server_env") or {}),
        "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP": f"example.com=127.0.0.1:{fixture.port}",
        **trace_env,
    }
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
            requests_before = (
                len(request_recorder.requests) if request_recorder is not None else 0
            )
            log_offset = log_size(server_log)
            turns = task_turns(task, catalogs, case_name)
            conversation = conversation_key(turns[0][1])

            def conversation_requests() -> list[dict[str, Any]]:
                # Only this repetition's own model requests: an earlier
                # repetition that timed out may still be running.
                if request_recorder is None:
                    return []
                return own_requests(
                    request_recorder.requests[requests_before:], conversation
                )
            multi_turn = "turns" in task
            idle_gap_seconds = float(task.get("idle_gap_seconds") or 0.0)
            idle_waited = 0.0
            started = time.monotonic()
            turn_marks: list[dict[str, Any]] = [{
                "start": {
                    "calls": 0, "requests": 0, "trace_calls": 0, "monotonic": started,
                },
                "end": None,
            }]

            async def turn_boundary_mark() -> dict[str, Any]:
                mark = {
                    "calls": len(fixture.calls) - calls_before,
                    "requests": len(conversation_requests()),
                    "monotonic": time.monotonic(),
                }
                mark["trace_calls"] = await _trace_call_count_at_boundary(
                    live_qa, trace_path, len(prior_trace_calls)
                )
                return mark

            async def on_turn_complete(turn: int) -> None:
                nonlocal idle_waited
                turn_marks[turn]["end"] = await turn_boundary_mark()
                if turn + 1 >= len(turns):
                    return
                if idle_gap_seconds > 0:
                    before_gap = time.monotonic()
                    await asyncio.sleep(idle_gap_seconds)
                    idle_waited += time.monotonic() - before_gap
                turn_marks.append({
                    "start": {**turn_marks[turn]["end"], "monotonic": time.monotonic()},
                    "end": None,
                })

            (first_prompt, first_marker), *follow_up_turns = turns
            result = await live_qa._live_chat_case(
                ctx,
                case_name=case_name,
                prompt=_with_marker_instruction(first_prompt, first_marker),
                marker=first_marker,
                required_text=[first_marker],
                timeout=180.0,
                enforce_marker=True,
                **({
                    "scripted_follow_ups": [
                        live_qa.ScriptedFollowUp(
                            prompt=_with_marker_instruction(prompt, marker),
                            marker=marker,
                            required_text=(marker,),
                        )
                        for prompt, marker in follow_up_turns
                    ],
                    "on_turn_complete": on_turn_complete,
                } if multi_turn else {}),
            )
            # End-to-end latency leaves out a deliberate idle gap.
            latency_ms = int((time.monotonic() - started - idle_waited) * 1000)
            calls = fixture.calls[calls_before:]
            model_requests = conversation_requests()
            metrics, all_trace_calls = _trace_metrics(live_qa, trace_path)
            trace_calls = all_trace_calls[len(prior_trace_calls):]
            tool_names = [call["name"] for call in trace_calls]
            scored = score_task(task, calls, trace_calls)
            observation = {
                "schema_version": OBSERVATION_SCHEMA_VERSION,
                "observation_id": observation_id(
                    arm, tool_count, task["id"], repetition, max_tools,
                ),
                "catalog": {
                    "generator_version": GENERATOR_VERSION,
                    "seed": SEED,
                    "tool_count": tool_count,
                    "namespace_count": NAMESPACE_COUNT,
                },
                "arm": arm,
                "config": arm_config,
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
                    "result_read_calls": result_read_call_count(trace_calls),
                    "discovery_result_read_calls": (
                        discovery_result_read_call_count(trace_calls)
                    ),
                    "bridged_tool_calls": bridged_tool_call_count(trace_calls),
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
                "cache": {
                    "tool_definition_signature_changes": (
                        tool_definition_signature_changes(model_requests)
                        if request_recorder is not None else None
                    ),
                    "tool_bearing_model_requests": (
                        sum(
                            isinstance(request.get("tools_signature"), str)
                            for request in model_requests
                        )
                        if request_recorder is not None else None
                    ),
                },
                "advertised": (
                    advertised_metrics(model_requests)
                    if request_recorder is not None else None
                ),
                "selection": (
                    selection_metrics(model_requests, trace_calls)
                    if request_recorder is not None else None
                ),
                "selection_log": (
                    selection_log_metrics(
                        read_log_since(server_log, log_offset),
                        (arm_config.get("jev") or {}).get("usd_per_million_input_tokens"),
                    )
                    if reads_selection_log else None
                ),
                "turns": (
                    turn_metrics(
                        task, turn_marks, calls,
                        model_requests if request_recorder is not None else None,
                        trace_calls,
                    )
                    if multi_turn else None
                ),
                "idle_gap": (
                    {
                        "seconds": idle_gap_seconds,
                        "server_env": dict(task.get("server_env") or {}),
                    }
                    if idle_gap_seconds > 0 else None
                ),
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


def _positive_int(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError(f"{value} is not a positive whole number")
    return number


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument(
        "--rescore", type=Path, metavar="RUN_DIR",
        help=(
            "re-score a finished run from RUN_DIR/observations.jsonl and "
            "RUN_DIR/llm-traces without running anything; the corrected "
            "observations and summary go to --output-dir, which must be new"
        ),
    )
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/ironclaw")
    parser.add_argument(
        "--arm", action="append", choices=ARMS,
        help=(
            "run this arm (repeatable); default: every arm except the opt-in "
            f"{', '.join(arm for arm in ARMS if arm not in DEFAULT_ARMS)}"
        ),
    )
    parser.add_argument("--tool-count", action="append", type=int)
    parser.add_argument(
        "--task",
        action="append",
        choices=TASK_IDS,
        help=(
            "run only this task (repeatable); without it every task runs "
            f"except the opt-in {', '.join(sorted(set(TASK_IDS) - set(DEFAULT_TASK_IDS)))}"
        ),
    )
    parser.add_argument("--repetitions", type=int, default=4)
    parser.add_argument(
        "--max-tools", action="append", type=_positive_int,
        help=(
            "set REBORN_TOOL_PREFETCH_MAX_TOOLS for the selection arms "
            "(repeatable, a sweep); without it the runtime default applies"
        ),
    )
    parser.add_argument(
        "--no-request-recorder",
        action="store_true",
        help=(
            "talk to the model endpoint directly; "
            "cache.tool_definition_signature_changes is then null"
        ),
    )
    return parser.parse_args()


# The oldest observation schema `--rescore` reads: version 5 has every field
# re-scoring uses (`task`, `counts` and the stored call counts).
RESCORE_MIN_SCHEMA_VERSION = 5


def load_observations(
    path: Path, *, min_schema_version: int | None = None,
) -> list[dict[str, Any]]:
    """The observations in `path`, first copy of each id, in file order.

    Every line must carry `OBSERVATION_SCHEMA_VERSION`, so a run never
    resumes into older output. `min_schema_version` widens that to a range
    ending at the current version, for reading a finished run.
    """
    if not path.exists():
        return []
    accepted = range(
        OBSERVATION_SCHEMA_VERSION if min_schema_version is None else min_schema_version,
        OBSERVATION_SCHEMA_VERSION + 1,
    )
    by_id: dict[str, dict[str, Any]] = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        observation = json.loads(line)
        version = observation.get("schema_version")
        if not isinstance(version, int) or isinstance(version, bool) or version not in accepted:
            expected = (
                str(OBSERVATION_SCHEMA_VERSION) if len(accepted) == 1
                else f"{accepted.start} to {OBSERVATION_SCHEMA_VERSION}"
            )
            raise ValueError(
                f"{path}:{line_number} has schema_version "
                f"{version!r}; expected {expected}"
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
    arms = args.arm or list(DEFAULT_ARMS)
    tool_counts = args.tool_count or [100, 500, 1000]
    tasks = [
        task for task in TASKS
        if (task["id"] in args.task if args.task else task["id"] in DEFAULT_TASK_IDS)
    ]
    observations = load_observations(observations_path)
    completed_ids = {item["observation_id"] for item in observations}
    inherited = [
        name for name in (*ARM_CONTROLLED_ENV, *TASK_CONTROLLED_ENV)
        if name in os.environ
    ]
    if inherited:
        print(
            "[tool-benchmark] ignoring exported "
            f"{', '.join(inherited)}: each arm or task sets these itself",
            flush=True,
        )
        for name in inherited:
            del os.environ[name]
    # Refuse a misconfigured arm now rather than hours into the matrix.
    probe_catalog = generate_catalog(tool_counts[0])
    for arm in arms:
        try:
            arm_env(arm, probe_catalog, dict(os.environ))
        except ValueError as exc:
            raise RuntimeError(f"arm {arm}: {exc}") from exc
    if any(ARM_SETTINGS[arm].get("classifier") == "jev" for arm in arms):
        print(
            "[tool-benchmark] prefetch-jev sends each task's opening message and "
            "every candidate tool's name, description and parameter names to "
            "the configured Jev provider, a third party", flush=True,
        )
    request_recorder = None
    upstream_base_url = resolve_model_base_url(dict(os.environ))
    if upstream_base_url is not None and not args.no_request_recorder:
        request_recorder = LlmRequestRecorder(upstream_base_url)
        request_recorder.start()
        # The generated Reborn home reads this when writing config.toml.
        os.environ["REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL"] = request_recorder.base_url
    elif any(ARM_SETTINGS[arm].get("prefetch") for arm in arms):
        print(
            "[tool-benchmark] warning: no request relay, so the advertised "
            "and selection metrics of the prefetch arms will be null; set "
            "REBORN_WEBUI_V2_LIVE_QA_LLM_BASE_URL to enable it", flush=True,
        )
    try:
        await _run_matrix(
            live_qa, args, arms, tool_counts, tasks, observations, completed_ids,
            observations_path, request_recorder,
        )
    finally:
        if request_recorder is not None:
            request_recorder.stop()
    summary = build_summary(observations, observations_path, await git_head())
    (args.output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8"
    )
    return 0 if all(item["task"]["completed"] for item in observations) else 1


def build_summary(
    observations: list[dict[str, Any]], observations_path: Path, head: str,
) -> dict[str, Any]:
    """The run summary written to `summary.json`."""
    return {
        "schema_version": SUMMARY_SCHEMA_VERSION,
        "head": head,
        "observation_count": len(observations),
        "observations_path": str(observations_path),
        "provider_usage_available": any(
            ((item.get("tokens") or {}).get("input") or 0) > 0
            for item in observations
        ),
        "aggregates": aggregate_observations(observations),
        "turn_aggregates": aggregate_turns(observations),
    }


# Stored per-observation counts that a re-sliced trace must reproduce before
# the slice is trusted (see `_group_trace_slices`).
_RESCORE_CHECKED_COUNTS = (
    ("tool_search_calls", lambda calls: sum(
        call.get("name") == "tool_search" for call in calls
    )),
    ("result_read_calls", result_read_call_count),
    ("bridged_tool_calls", bridged_tool_call_count),
)


def _group_trace_slices(
    trace_path: Path, items: list[dict[str, Any]],
) -> tuple[list[list[dict[str, Any]]] | None, str | None]:
    """Split one task group's trace back into its observations' tool calls.

    A group (arm, catalog size, task) shares one server and one trace file,
    and the run gave each observation the calls appended to the trace while
    it ran. `counts.tool_calls` records how many, so cutting the trace's
    calls in run order by those counts reproduces each observation's slice,
    including any calls a timed-out earlier conversation appended meanwhile.
    Each slice must reproduce the observation's stored `tool_search_calls`,
    `result_read_calls` and `bridged_tool_calls`, and the counts must add up
    to the whole trace. Otherwise the slices are None, with the reason.
    """
    try:
        payload = json.loads(trace_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None, "trace missing"
    except (OSError, ValueError) as exc:
        return None, f"trace unreadable: {type(exc).__name__}"
    calls = trace_tool_calls(payload)
    sizes = [(item.get("counts") or {}).get("tool_calls") for item in items]
    if not all(isinstance(size, int) and size >= 0 for size in sizes):
        return None, "an observation has no counts.tool_calls"
    if sum(sizes) != len(calls):
        return None, (
            f"observations count {sum(sizes)} tool calls, the trace holds {len(calls)}"
        )
    slices = []
    cursor = 0
    for item, size in zip(items, sizes):
        chunk = calls[cursor:cursor + size]
        cursor += size
        stored = item.get("counts") or {}
        for key, count in _RESCORE_CHECKED_COUNTS:
            if stored.get(key) != count(chunk):
                return None, (
                    f"{item.get('observation_id')}: re-sliced {key} "
                    f"{count(chunk)} != stored {stored.get(key)}"
                )
        slices.append(chunk)
    return slices, None


def rescore_observation(
    task: dict[str, Any], observation: dict[str, Any], trace_calls: list[dict[str, Any]],
) -> dict[str, Any]:
    """One stored observation with its scoring recomputed from its trace calls.

    Only a no-tool task's `task` scoring is recomputed. It needs the names
    that reached the fixture (`task.called_tools`, stored) and the model's
    calls (from the trace). A task with expected tools keeps its stored
    `task`: its argument checks need the fixture calls' arguments, which an
    observation does not store, and nothing this re-score changes affects
    it. `counts.discovery_result_read_calls` is added and `failure` is
    recomputed.
    """
    rescored = json.loads(json.dumps(observation))
    rescored.setdefault("counts", {})["discovery_result_read_calls"] = (
        discovery_result_read_call_count(trace_calls)
    )
    if task["expected"]:
        return rescored
    stored = rescored.get("task") or {}
    fixture_calls = [{"name": name} for name in stored.get("called_tools") or []]
    scored = score_task(task, fixture_calls, trace_calls)
    rescored["task"] = {"id": task["id"], **scored}
    rescored["failure"] = (
        None if rescored.get("ui_probe_success") and scored["completed"]
        else "task_incomplete"
    )
    return rescored


def rescore_run(
    source_dir: Path,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Recompute scoring for a finished run from its stored artifacts.

    Reads `observations.jsonl` (schema `RESCORE_MIN_SCHEMA_VERSION` or
    newer; each observation keeps its own `schema_version`) and
    `llm-traces/<arm>-<size>-<task>.json` from `source_dir` and writes
    nothing. Returns the observations, in their
    original order, with each verified group re-scored by
    `rescore_observation`, and a report: how many observations were
    re-scored, which ones changed `completed`, and the groups kept as stored
    because their trace could not be split reliably.
    """
    observations = load_observations(
        source_dir / "observations.jsonl",
        min_schema_version=RESCORE_MIN_SCHEMA_VERSION,
    )
    tasks = {task["id"]: task for task in TASKS}
    groups: dict[tuple[str, int, str], list[dict[str, Any]]] = {}
    for observation in observations:
        key = (
            observation["arm"], observation["catalog"]["tool_count"],
            observation["task"]["id"],
        )
        groups.setdefault(key, []).append(observation)
    replaced: dict[str, dict[str, Any]] = {}
    unverified = []
    for (arm, tool_count, task_id), items in groups.items():
        group = f"{arm}-{tool_count}-{task_id}"
        task = tasks.get(task_id)
        if task is None:
            unverified.append({"group": group, "reason": "unknown task"})
            continue
        slices, reason = _group_trace_slices(
            source_dir / "llm-traces" / f"{group}.json", items
        )
        if slices is None:
            unverified.append({"group": group, "reason": reason})
            continue
        for item, trace_calls in zip(items, slices):
            replaced[item["observation_id"]] = rescore_observation(task, item, trace_calls)
    rescored = [replaced.get(item["observation_id"], item) for item in observations]
    changed = [
        {
            "observation_id": before["observation_id"],
            "completed": [before["task"]["completed"], after["task"]["completed"]],
        }
        for before, after in zip(observations, rescored)
        if before["task"]["completed"] != after["task"]["completed"]
    ]
    return rescored, {
        "source_dir": str(source_dir),
        "rescored_observations": len(replaced),
        "kept_observations": len(observations) - len(replaced),
        "completion_changes": changed,
        "unverified_groups": unverified,
    }


def rescore_main(source_dir: Path, output_dir: Path) -> int:
    """Write a re-scored copy of a finished run into a fresh `output_dir`."""
    if output_dir.resolve() == source_dir.resolve():
        raise RuntimeError("--output-dir must differ from the run being re-scored")
    observations_path = output_dir / "observations.jsonl"
    if observations_path.exists():
        raise RuntimeError(f"{observations_path} already exists; choose a fresh directory")
    observations, report = rescore_run(source_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    with observations_path.open("w", encoding="utf-8") as handle:
        for observation in observations:
            handle.write(json.dumps(observation, sort_keys=True) + "\n")
    summary = {
        **build_summary(observations, observations_path, asyncio.run(git_head())),
        "rescore": report,
    }
    (output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8"
    )
    print(
        f"[tool-benchmark] re-scored {report['rescored_observations']} observations "
        f"({len(report['completion_changes'])} changed completion, "
        f"{len(report['unverified_groups'])} groups kept as stored) into {output_dir}",
        flush=True,
    )
    return 0


async def _run_matrix(
    live_qa: Any,
    args: argparse.Namespace,
    arms: list[str],
    tool_counts: list[int],
    tasks: list[dict[str, Any]],
    observations: list[dict[str, Any]],
    completed_ids: set[str],
    observations_path: Path,
    request_recorder: LlmRequestRecorder | None,
) -> None:
    sweep = getattr(args, "max_tools", None)
    for tool_count in tool_counts:
        for arm in arms:
            for max_tools in max_tools_sweep(arm, sweep):
                for task in tasks:
                    missing_repetitions = [
                        repetition
                        for repetition in range(args.repetitions)
                        if observation_id(
                            arm, tool_count, task["id"], repetition, max_tools,
                        ) not in completed_ids
                    ]
                    if not missing_repetitions:
                        continue
                    group = await run_task_group(
                        live_qa, args.binary, args.output_dir, arm, tool_count,
                        task, missing_repetitions, observations_path,
                        request_recorder, max_tools,
                    )
                    for observation in group:
                        observations.append(observation)
                        completed_ids.add(observation["observation_id"])


def main() -> int:
    args = parse_args()
    if args.rescore is not None:
        return rescore_main(args.rescore, args.output_dir)
    return asyncio.run(async_main(args))


if __name__ == "__main__":
    raise SystemExit(main())
