"""Replay every harvested live-QA trace through the recorded-model adapter.

Every trace is replayed response-by-response through the same mock LLM adapter
used by standalone Reborn. The closed inventory fails if a harvested operation
lacks an explicit provider-fixture classification.
"""

import json
from pathlib import Path

import httpx
import pytest
from mock_llm_trace import (
    capture_trace_tool_call_id_aliases,
    resolve_trace_result_bindings,
    trace_tool_results_with_recorded_ids,
)
from provider_capability_inventory import (
    ALL_CLASSIFIED_CAPABILITY_IDS,
    LIVE_ONLY_TOOLS,
    PROVIDER_WIRE_PREFIXES,
    capability_id_to_wire_name,
)

pytest_plugins = ["reborn_webui_harness"]

ROOT = Path(__file__).resolve().parents[3]
TRACE_DIR = ROOT / "tests/fixtures/llm_traces/reborn_qa/live_canary"
MANIFEST_PATH = TRACE_DIR / "case-manifest.json"


def _model_cases() -> list[str]:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    no_model = set(manifest["no_model_cases"])
    # Quarantined traces encode the retired activation flow; their fixtures
    # live under quarantined_retired_activation/ and are not replayable here.
    quarantined = set(manifest.get("quarantined_model_cases", []))
    return [
        case
        for case in manifest["selected_cases"]
        if case not in no_model and case not in quarantined
    ]


MODEL_CASES = _model_cases()


def test_exact_trace_result_binding_uses_only_the_requested_call():
    arguments = {
        "document_id": {
            "$trace_result": {
                "tool_call_id": "call_create",
                "pointer": "/document/documentId",
            }
        }
    }
    observed = [
        {
            "tool_call_id": "call_create",
            "content": {"document": {"documentId": "fresh-document"}},
        },
        {
            "tool_call_id": "call_similar",
            "content": {"document": {"documentId": "wrong-document"}},
        },
    ]

    assert resolve_trace_result_bindings(arguments, observed) == {
        "document_id": "fresh-document"
    }


def test_exact_trace_result_binding_does_not_guess_missing_calls():
    marker = {
        "$trace_result": {
            "tool_call_id": "call_missing",
            "pointer": "/document/documentId",
        }
    }
    observed = [
        {
            "tool_call_id": "call_similar",
            "content": {"document": {"documentId": "wrong-document"}},
        }
    ]

    with pytest.raises(ValueError, match="no tool call with id"):
        resolve_trace_result_bindings(marker, observed)


def test_exact_trace_result_binding_rejects_duplicate_observed_ids():
    marker = {
        "$trace_result": {
            "tool_call_id": "call_create",
            "pointer": "/document/documentId",
        }
    }
    observed = [
        {
            "tool_call_id": "call_create",
            "content": {"document": {"documentId": "first-document"}},
        },
        {
            "tool_call_id": "call_create",
            "content": {"document": {"documentId": "second-document"}},
        },
    ]

    with pytest.raises(ValueError, match="multiple tool results with id"):
        resolve_trace_result_bindings(marker, observed)


def test_exact_trace_result_binding_uses_results_from_prior_user_turns():
    marker = {
        "$trace_result": {
            "tool_call_id": "call_create",
            "pointer": "/document/documentId",
        }
    }
    messages = [
        {
            "role": "assistant",
            "tool_calls": [
                {
                    "id": "runtime_create",
                    "function": {
                        "name": "google_docs__create_document",
                        "arguments": json.dumps({"title": "first turn"}),
                    },
                }
            ],
        },
        {
            "role": "tool",
            "tool_call_id": "runtime_create",
            "content": json.dumps({"document": {"documentId": "fresh-document"}}),
        },
        {"role": "user", "content": "Now read that document"},
    ]
    state = {
        "tool_call_id_aliases": {"call_create": "runtime_create"},
    }

    observed = trace_tool_results_with_recorded_ids(state, messages)

    assert resolve_trace_result_bindings(marker, observed) == "fresh-document"


def test_exact_trace_result_binding_rejects_paged_evidence_preview():
    marker = {
        "$trace_result": {
            "tool_call_id": "call_create",
            "pointer": "/detail/preview",
        }
    }
    observed = [
        {
            "tool_call_id": "call_create",
            "content": {
                "schema_version": 1,
                "status": "success",
                "trust": "provider",
                "detail": {
                    "byte_len": 48,
                    "total_bytes": 48,
                    "next_offset": 48,
                    "preview": json.dumps(
                        {"document": {"documentId": "partial-document"}}
                    ),
                },
            },
        }
    ]

    with pytest.raises(ValueError, match="has no JSON Pointer"):
        resolve_trace_result_bindings(marker, observed)


def test_exact_trace_result_binding_unwraps_complete_preview_and_pointer_escapes():
    preview = json.dumps({"files": [{"a/b~c": "escaped-hit"}]})
    marker = {
        "$trace_result": {
            "tool_call_id": "call_list",
            "pointer": "/files/0/a~1b~0c",
        }
    }
    observed = [
        {
            "tool_call_id": "call_list",
            "content": {
                "schema_version": 1,
                "status": "success",
                "trust": "provider",
                "detail": {
                    "byte_len": len(preview),
                    "total_bytes": len(preview),
                    "preview": preview,
                },
            },
        }
    ]

    assert resolve_trace_result_bindings(marker, observed) == "escaped-hit"


def test_trace_aliasing_rejects_indistinguishable_parallel_calls():
    state = {
        "pending_tool_calls": [
            {
                "id": "call_first",
                "name": "google-docs__create_document",
                "arguments": {"title": "same"},
            },
            {
                "id": "call_second",
                "name": "google-docs__create_document",
                "arguments": {"title": "same"},
            },
        ],
        "tool_call_id_aliases": {},
    }
    messages = [
        {
            "role": "assistant",
            "tool_calls": [
                {
                    "id": "runtime_second",
                    "function": {
                        "name": "google_docs__create_document",
                        "arguments": json.dumps({"title": "same"}),
                    },
                },
                {
                    "id": "runtime_first",
                    "function": {
                        "name": "google_docs__create_document",
                        "arguments": json.dumps({"title": "same"}),
                    },
                },
            ],
        }
    ]

    with pytest.raises(ValueError, match="ambiguous trace tool-call aliases"):
        capture_trace_tool_call_id_aliases(state, messages)


def test_trace_aliasing_matches_reordered_calls_by_arguments():
    state = {
        "pending_tool_calls": [
            {
                "id": "call_wanted",
                "name": "google-docs__create_document",
                "arguments": {"title": "wanted"},
            },
            {
                "id": "call_other",
                "name": "google-docs__create_document",
                "arguments": {"title": "other"},
            },
        ],
        "tool_call_id_aliases": {},
    }
    messages = [
        {
            "role": "assistant",
            "tool_calls": [
                {
                    "id": "runtime_other",
                    "function": {
                        "name": "google_docs__create_document",
                        "arguments": json.dumps({"title": "other"}),
                    },
                },
                {
                    "id": "runtime_wanted",
                    "function": {
                        "name": "google_docs__create_document",
                        "arguments": json.dumps({"title": "wanted"}),
                    },
                },
            ],
        }
    ]

    capture_trace_tool_call_id_aliases(state, messages)

    assert state["tool_call_id_aliases"] == {
        "call_wanted": "runtime_wanted",
        "call_other": "runtime_other",
    }


def test_trace_aliasing_discards_mismatched_pending_batch():
    state = {
        "pending_tool_calls": [
            {
                "id": "call_pending",
                "name": "google-docs__create_document",
                "arguments": {},
            }
        ],
        "tool_call_id_aliases": {},
    }

    with pytest.raises(ValueError, match="alias count mismatch"):
        capture_trace_tool_call_id_aliases(state, [])

    assert state["pending_tool_calls"] == []


async def test_trace_install_rejects_missing_tool_call_id(mock_llm_server):
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "create"}},
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "name": "google-docs__create_document",
                            "arguments": {},
                        }
                    ],
                }
            },
        ]
    }

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": "missing-id.json", "trace": trace},
            timeout=15,
        )

    assert response.status_code == 400
    assert response.json()["error"] == ("trace.steps[1].tool_calls[0] is invalid")


async def test_trace_install_rejects_duplicate_tool_call_id(mock_llm_server):
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "create twice"}},
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_duplicate",
                            "name": "google-docs__create_document",
                            "arguments": {"title": "first"},
                        }
                    ],
                }
            },
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_duplicate",
                            "name": "google-docs__create_document",
                            "arguments": {"title": "second"},
                        }
                    ],
                }
            },
        ]
    }

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": "duplicate-id.json", "trace": trace},
            timeout=15,
        )

    assert response.status_code == 400
    assert "reuses tool call id 'call_duplicate'" in response.json()["error"]


def _load_case(case: str) -> dict:
    return json.loads((TRACE_DIR / f"{case}.json").read_text(encoding="utf-8"))


def _tool_calls(trace: dict) -> list[dict]:
    return [
        call
        for step in trace["steps"]
        for call in step["response"].get("tool_calls", [])
    ]


def _tool_definitions(trace: dict) -> list[dict]:
    names = sorted({call["name"] for call in _tool_calls(trace)})
    return [
        {
            "type": "function",
            "function": {
                "name": name,
                "description": f"Recorded QA capability {name}",
                "parameters": {"type": "object", "additionalProperties": True},
            },
        }
        for name in names
    ]


async def _install_trace(mock_llm_server: str, case: str, trace: dict) -> None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": f"{case}.json", "trace": trace},
            timeout=15,
        )
    response.raise_for_status()


def _apply_request_hint(messages: list[dict], step: dict) -> None:
    """Reconstruct runtime-injected request context omitted from trace responses."""
    request_hint = step.get("request_hint", {})
    hinted_user_input = request_hint.get("last_user_message_contains")
    last_user_input = next(
        (
            message.get("content", "")
            for message in reversed(messages)
            if message.get("role") == "user"
        ),
        "",
    )
    needs_hinted_user_input = (
        hinted_user_input is not None and hinted_user_input not in last_user_input
    )
    min_message_count = request_hint.get("min_message_count", 0)
    padding_target = min_message_count - int(needs_hinted_user_input)
    while len(messages) < padding_target:
        messages.append(
            {"role": "system", "content": "Recorded runtime request context"}
        )
    if needs_hinted_user_input:
        messages.append({"role": "user", "content": hinted_user_input})


async def _replay_every_response(
    mock_llm_server: str,
    case: str,
    trace: dict,
) -> None:
    """Replay and compare every recorded response over OpenAI-compatible HTTP."""
    await _install_trace(mock_llm_server, case, trace)
    # Harvested request hints count the runtime's system prompt as message 1.
    messages = [{"role": "system", "content": "Local recorded-trace replay"}]
    tools = _tool_definitions(trace)
    response_count = 0

    async with httpx.AsyncClient() as client:
        for step_index, step in enumerate(trace["steps"]):
            expected = step["response"]
            if expected["type"] == "user_input":
                messages.append({"role": "user", "content": expected["content"]})
                continue

            _apply_request_hint(messages, step)
            response = await client.post(
                f"{mock_llm_server}/v1/chat/completions",
                json={
                    "model": "mock-model",
                    "messages": messages,
                    "tools": tools,
                    "stream": False,
                },
                timeout=15,
            )
            assert response.status_code == 200, (
                f"{case} step {step_index} failed: {response.text}"
            )
            message = response.json()["choices"][0]["message"]
            response_count += 1

            if expected["type"] == "text":
                assert message["content"] == expected["content"]
                messages.append({"role": "assistant", "content": message["content"]})
                continue

            actual_calls = message.get("tool_calls", [])
            expected_calls = expected["tool_calls"]
            assert len(actual_calls) == len(expected_calls), case
            for actual, recorded in zip(actual_calls, expected_calls, strict=True):
                assert actual["function"]["name"] == recorded["name"]
                assert (
                    json.loads(actual["function"]["arguments"]) == recorded["arguments"]
                )

            messages.append(
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": actual_calls,
                }
            )
            messages.extend(
                {
                    "role": "tool",
                    "tool_call_id": actual["id"],
                    "content": json.dumps({"ok": True, "source": "local replay"}),
                }
                for actual in actual_calls
            )

        state = await client.get(
            f"{mock_llm_server}/__mock/llm_trace",
            timeout=15,
        )
    state.raise_for_status()
    assert state.json() == {
        "source": f"{case}.json",
        "next_response": response_count,
        "response_count": response_count,
        "complete": True,
        "error": None,
    }


async def test_trace_loader_rejects_consecutive_user_inputs(mock_llm_server):
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "first"}},
            {"response": {"type": "user_input", "content": "second"}},
            {"response": {"type": "text", "content": "done"}},
        ]
    }
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/__mock/llm_trace",
            json={"source": "malformed.json", "trace": trace},
            timeout=15,
        )
    assert response.status_code == 400
    assert "consecutive user_input" in response.json()["error"]


async def test_trace_replay_enforces_request_hints(mock_llm_server):
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "expected input"}},
            {
                "request_hint": {
                    "last_user_message_contains": "expected input",
                    "min_message_count": 2,
                },
                "response": {"type": "text", "content": "done"},
            },
        ]
    }
    await _install_trace(mock_llm_server, "request-hints", trace)
    async with httpx.AsyncClient() as client:
        too_short = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={
                "model": "mock-model",
                "messages": [{"role": "user", "content": "expected input"}],
            },
            timeout=15,
        )
        assert too_short.status_code == 409
        assert "expected at least 2" in too_short.text

        wrong_input = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={
                "model": "mock-model",
                "messages": [
                    {"role": "system", "content": "system"},
                    {"role": "user", "content": "different input"},
                ],
            },
            timeout=15,
        )
        assert wrong_input.status_code == 409
        assert "request hint does not match" in wrong_input.text


@pytest.mark.parametrize("case", MODEL_CASES, ids=MODEL_CASES)
async def test_every_harvested_trace_replays(
    case,
    mock_llm_server,
):
    """Every harvested model case is executable through the mock adapter."""
    trace = _load_case(case)
    await _replay_every_response(mock_llm_server, case, trace)


def test_fixture_catalog_has_no_unowned_cases_or_provider_operations():
    """The manifest, files, and Emulate classifications remain closed sets."""
    fixture_cases = {path.stem for path in TRACE_DIR.glob("qa_*.json")}
    assert set(MODEL_CASES) == fixture_cases

    observed_provider_tools = {
        call["name"]
        for case in MODEL_CASES
        for call in _tool_calls(_load_case(case))
        if call["name"].startswith(PROVIDER_WIRE_PREFIXES)
    }
    classified_tools = {
        capability_id_to_wire_name(capability_id)
        for capability_id in ALL_CLASSIFIED_CAPABILITY_IDS
    }
    assert observed_provider_tools <= classified_tools, (
        f"unclassified={sorted(observed_provider_tools - classified_tools)}"
    )
    assert LIVE_ONLY_TOOLS <= observed_provider_tools


async def test_trace_replay_binds_fresh_provider_ids_into_follow_up_calls(
    mock_llm_server,
):
    """A created resource ID can drive the next real recorded tool call."""
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "create and read"}},
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_create_document",
                            "name": "google-docs__create_document",
                            "arguments": {"title": "wanted"},
                        },
                        {
                            "id": "call_create_other_document",
                            "name": "google-docs__create_document",
                            "arguments": {"title": "other"},
                        },
                    ],
                }
            },
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_read_document",
                            "name": "google-docs__read_content",
                            "arguments": {
                                "document_id": {
                                    "$trace_result": {
                                        "tool_call_id": "call_create_document",
                                        "pointer": "/document/documentId",
                                    }
                                }
                            },
                        }
                    ],
                }
            },
            {"response": {"type": "text", "content": "done"}},
        ]
    }
    tools = _tool_definitions(trace)
    messages = [{"role": "user", "content": "create and read"}]

    await _install_trace(mock_llm_server, "binding-contract", trace)
    async with httpx.AsyncClient() as client:
        created = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={"model": "mock-model", "messages": messages, "tools": tools},
            timeout=15,
        )
        created.raise_for_status()
        created_message = created.json()["choices"][0]["message"]
        created_message["tool_calls"][0]["id"] = "runtime_normalized_call_id"
        created_message["tool_calls"][0]["function"]["name"] = (
            "google_docs__create_document"
        )
        created_message["tool_calls"][1]["id"] = "runtime_other_call_id"
        created_message["tool_calls"][1]["function"]["name"] = (
            "google_docs__create_document"
        )
        created_message["tool_calls"].reverse()
        messages.extend(
            [
                created_message,
                {
                    "role": "tool",
                    "tool_call_id": "runtime_normalized_call_id",
                    "content": json.dumps(
                        {"document": {"documentId": "doc-created-locally"}}
                    ),
                },
                {
                    "role": "tool",
                    "tool_call_id": "runtime_other_call_id",
                    "content": json.dumps(
                        {"document": {"documentId": "wrong-document"}}
                    ),
                },
            ]
        )

        read = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={"model": "mock-model", "messages": messages, "tools": tools},
            timeout=15,
        )
        read.raise_for_status()

    arguments = json.loads(
        read.json()["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
    )
    assert arguments == {"document_id": "doc-created-locally"}


async def test_trace_replay_accepts_direct_calls_from_deferred_tool_catalog(
    mock_llm_server,
):
    """Deferred tools are callable even though only tool_search is advertised."""
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "inspect Slack"}},
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_whoami",
                            "name": "slack__whoami",
                            "arguments": {},
                        }
                    ],
                }
            },
        ]
    }
    await _install_trace(mock_llm_server, "deferred-catalog", trace)
    tools = [
        {
            "type": "function",
            "function": {
                "name": "builtin__tool_search",
                "description": "On-demand tools:\n- slack.whoami",
                "parameters": {"type": "object"},
            },
        }
    ]

    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={
                "model": "mock-model",
                "messages": [{"role": "user", "content": "inspect Slack"}],
                "tools": tools,
            },
            timeout=15,
        )
    response.raise_for_status()
    call = response.json()["choices"][0]["message"]["tool_calls"][0]
    assert call["function"]["name"] == "slack__whoami"


async def test_trace_replay_stops_after_failed_capability_result(mock_llm_server):
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "inspect Slack"}},
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_whoami",
                            "name": "slack__whoami",
                            "arguments": {},
                        }
                    ],
                }
            },
            {"response": {"type": "text", "content": "done"}},
        ]
    }
    await _install_trace(mock_llm_server, "failed-result", trace)
    tools = _tool_definitions(trace)
    messages = [{"role": "user", "content": "inspect Slack"}]

    async with httpx.AsyncClient() as client:
        first = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={"model": "mock-model", "messages": messages, "tools": tools},
            timeout=15,
        )
        first.raise_for_status()
        assistant = first.json()["choices"][0]["message"]
        messages.extend(
            [
                assistant,
                {
                    "role": "tool",
                    "name": "slack__whoami",
                    "tool_call_id": assistant["tool_calls"][0]["id"],
                    "content": json.dumps({"status": "failed"}),
                },
            ]
        )
        failed = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={"model": "mock-model", "messages": messages, "tools": tools},
            timeout=15,
        )

    assert failed.status_code == 409
    assert "failed capability result" in failed.text


async def test_trace_replay_accepts_exact_expected_capability_failure(mock_llm_server):
    trace = {
        "steps": [
            {"response": {"type": "user_input", "content": "inspect Slack"}},
            {
                "response": {
                    "type": "tool_calls",
                    "tool_calls": [
                        {
                            "id": "call_whoami",
                            "name": "slack__whoami",
                            "arguments": {},
                        }
                    ],
                }
            },
            {
                "request_hint": {
                    "expected_failed_tool_result_contains": "channel_not_found"
                },
                "response": {"type": "text", "content": "reported honestly"},
            },
        ]
    }
    await _install_trace(mock_llm_server, "expected-failed-result", trace)
    tools = _tool_definitions(trace)
    messages = [{"role": "user", "content": "inspect Slack"}]

    async with httpx.AsyncClient() as client:
        first = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={"model": "mock-model", "messages": messages, "tools": tools},
            timeout=15,
        )
        first.raise_for_status()
        assistant = first.json()["choices"][0]["message"]
        messages.extend(
            [
                assistant,
                {
                    "role": "tool",
                    "name": "slack__whoami",
                    "tool_call_id": assistant["tool_calls"][0]["id"],
                    "content": json.dumps(
                        {"status": "error", "error": "channel_not_found"}
                    ),
                },
            ]
        )
        final = await client.post(
            f"{mock_llm_server}/v1/chat/completions",
            json={"model": "mock-model", "messages": messages, "tools": tools},
            timeout=15,
        )

    final.raise_for_status()
    assert final.json()["choices"][0]["message"]["content"] == "reported honestly"
