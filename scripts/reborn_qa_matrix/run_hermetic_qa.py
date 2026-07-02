#!/usr/bin/env python3
"""Hermetic QA matrix runner for Reborn WebUI v2 and ResponsesAPI rows.

The workbook tracks external-existing CI, browser, and live-canary coverage.
This runner intentionally owns only the executable non-live cases added by this
QA branch: served ResponsesAPI checks and served WebUI v2 HTTP/API checks.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "reborn-qa-matrix-hermetic"
DEFAULT_TIMEOUT_SECONDS = 45 * 60
PROVIDER = "reborn-qa-matrix"
MODE = "hermetic"

REMOVED_CI_OWNED_CASES = {
    "openai_compat_owner_crate_regression",
    "reborn_cli_credential_refresh_settings_regression",
    "reborn_cli_docker_railway_entrypoint_regression",
    "reborn_cli_trigger_poller_settings_regression",
    "reborn_operator_logs_service_regression",
    "slack_events_ingress_regression",
    "slack_host_beta_serve_mount_regression",
    "slack_outbound_delivery_rendering_regression",
    "slack_personal_oauth_binding_regression",
    "slack_personal_pairing_regression",
    "support_substrate_product_workflow_regression",
    "webui_v2_auth_surface_composition_regression",
    "webui_v2_automations_trace_outbound_channel_api_regression",
    "webui_v2_composition_regression",
    "webui_v2_extension_oauth_setup_regression",
    "webui_v2_filesystem_api_regression",
    "webui_v2_gateway_middleware_serve_foundation_regression",
    "webui_v2_manual_token_regression",
    "webui_v2_product_auth_account_lifecycle_regression",
    "webui_v2_product_auth_oauth_regression",
    "webui_v2_public_sso_session_regression",
    "webui_v2_route_contract_regression",
    "webui_v2_rust_static_regression",
    "webui_v2_serve_listener_regression",
    "webui_v2_serve_security_config_regression",
    "webui_v2_spa_static_serving_regression",
    "webui_v2_sso_login_startup_regression",
    "webui_v2_sso_user_admission_regression",
}


@dataclass(frozen=True)
class CommandSpec:
    name: str
    argv: list[str]
    env: dict[str, str] = field(default_factory=dict)
    unset_env: list[str] = field(default_factory=list)
    description: str = ""


@dataclass(frozen=True)
class CaseSpec:
    name: str
    feature: str
    category: str
    qa_matrix_test_ids: list[str]
    commands: list[CommandSpec]
    coverage_source: str = "matrix_only_or_new"
    default_enabled: bool = True
    notes: str = ""


def _pytest_command(
    name: str,
    description: str,
    tests: list[str],
) -> CommandSpec:
    packages = [
        "pytest",
        "pytest-asyncio",
        "pytest-timeout",
        "aiohttp",
        "httpx",
        "cryptography",
    ]

    argv = ["uv", "run", "--no-project"]
    for package in packages:
        argv.extend(["--with", package])
    argv.extend(["pytest", *tests, "-q"])
    return CommandSpec(
        name=name,
        description=description,
        env={"CARGO_INCREMENTAL": "0"},
        argv=argv,
    )


def _case(
    name: str,
    feature: str,
    category: str,
    test_ids: list[str],
    command: CommandSpec,
    notes: str,
) -> tuple[str, CaseSpec]:
    return (
        name,
        CaseSpec(
            name=name,
            feature=feature,
            category=category,
            qa_matrix_test_ids=test_ids,
            commands=[command],
            notes=notes,
        ),
    )


CASES = dict(
    [
        _case(
            "openai_compat_beta_routes_regression",
            "OpenAI-Compatible Beta Routes",
            "Served OpenAI-Compatible Route Mount E2E",
            [f"REBCLI-039-TC-{index:02d}" for index in range(1, 9)],
            _pytest_command(
                "openai_compat_route_mount_served_e2e",
                "Served route-mount coverage for Chat Completions, Models, "
                "Responses, bearer gates, and /api/v1 aliases.",
                [
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_openai_compat_route_mounts_require_bearer_served",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_openai_compat_route_mounts_authenticated_aliases_served",
                ],
            ),
            "Rust route/composition contracts remain normal CI coverage.",
        ),
        _case(
            "openai_responses_missing_cancel_shape_regression",
            "OpenAI-compatible Responses retrieve and cancel APIs",
            "Served Responses Retrieve/Cancel API E2E",
            ["REBCLI-058-TC-02"],
            _pytest_command(
                "openai_responses_missing_cancel_shape_served_e2e",
                "Served ResponsesAPI coverage for consistent missing retrieve "
                "and cancel not-found response shape.",
                [
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_responses_lookup_and_cancel_missing_id_match_not_found_shape",
                ],
            ),
            "Existing legacy Responses E2E owns create/retrieve/stream/auth/validation coverage.",
        ),
        _case(
            "openai_responses_external_tools_e2e_regression",
            "OpenAI-compatible Responses external function tools",
            "Served Reborn ResponsesAPI E2E",
            [f"REBCLI-100-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "openai_responses_external_tools_served_e2e",
                "Served external function tool round trips, failure output, "
                "wrong call_id rejection, and mixed internal/external tools.",
                [
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_responses_repeated_external_tools_round_trip",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_responses_external_tool_failure_output_reaches_llm",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_responses_rejects_wrong_external_tool_call_id",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_responses_mixed_internal_and_external_tools_same_assistant_response",
                ],
            ),
            "Served ResponsesAPI coverage; not browser or live-canary coverage.",
        ),
        _case(
            "openai_chat_completions_workflow_regression",
            "OpenAI-compatible Chat Completions API",
            "Served Chat Completions API E2E",
            [f"REBCLI-056-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "openai_chat_completions_served_e2e",
                "Served Chat Completions coverage for non-streaming, "
                "idempotency replay/conflict, SSE streaming, auth, and validation.",
                [
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_chat_completions_non_streaming_served",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_chat_completions_idempotency_replay_and_conflict_served",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_chat_completions_streaming_raw_sse_served",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_chat_completions_auth_and_validation_served",
                ],
            ),
            "Rust handler/streaming contracts remain normal CI coverage.",
        ),
        _case(
            "openai_models_list_api_regression",
            "OpenAI-compatible Models API",
            "Served OpenAI-compatible Models API E2E",
            [f"REBCLI-099-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "openai_models_served_e2e",
                "Served Models API coverage for /v1/models, /api/v1/models, "
                "configured model projection, and auth rejection.",
                [
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_models_v1_lists_configured_mock_model",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_models_api_v1_alias_matches_v1_models",
                    "tests/e2e/scenarios/test_reborn_responses_api.py::test_reborn_models_requires_auth",
                ],
            ),
            "Rust route and host-catalog contracts remain normal CI coverage.",
        ),
        _case(
            "webui_v2_session_thread_message_api_regression",
            "WebUI v2 session, thread, and message APIs",
            "Hermetic WebUI v2 API Regression",
            [f"REBCLI-043-TC-{index:02d}" for index in range(1, 7)]
            + ["REBCLI-043-TC-09", "REBCLI-043-TC-10", "REBCLI-043-TC-11"],
            _pytest_command(
                "webui_v2_session_thread_message_served_e2e",
                "Served session/thread/message API coverage for session "
                "projection, thread lifecycle, messages, auth, and errors.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_session_api.py"],
            ),
            "Rust route/service/execution substrate contracts remain normal CI coverage.",
        ),
        _case(
            "webui_v2_streaming_run_control_api_regression",
            "WebUI v2 streaming and run-control APIs",
            "Served WebUI v2 Streaming and Run-Control API E2E",
            [f"REBCLI-044-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "webui_v2_streaming_run_control_served_e2e",
                "Served SSE, token-shim, cancel, and gate-resolution API coverage.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_streaming_run_control_api.py"],
            ),
            "Support-substrate cargo coverage remains normal CI coverage.",
        ),
        _case(
            "webui_v2_automation_trace_outbound_served_api_regression",
            "WebUI v2 automations, trace, outbound, and channel served APIs",
            "Served WebUI v2 Automation/Trace/Outbound API E2E",
            [f"REBCLI-045-TC-{index:02d}" for index in range(11, 15)],
            _pytest_command(
                "webui_v2_automation_trace_outbound_served_e2e",
                "Served automations, Trace Commons, outbound preferences, "
                "outbound targets, and connectable-channel API coverage.",
                [
                    "tests/e2e/scenarios/test_reborn_webui_v2_automation_trace_outbound_api.py"
                ],
            ),
            "Runtime/tool substrate contracts and live side effects remain outside this lane.",
        ),
        _case(
            "webui_v2_extension_lifecycle_api_regression",
            "WebUI v2 extension lifecycle APIs",
            "Hermetic Extension Lifecycle API Regression",
            [f"REBCLI-046-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "webui_v2_extension_lifecycle_served_e2e",
                "Served extension registry/list/install/setup/activate/remove coverage.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_extensions_api.py"],
            ),
            "Browser extension workflows remain external-existing coverage.",
        ),
        _case(
            "webui_v2_skill_management_api_regression",
            "WebUI v2 skill management APIs",
            "Hermetic Skill Management API Regression",
            [f"REBCLI-047-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "webui_v2_skill_management_served_e2e",
                "Served skill list/install/read/update/delete/search and "
                "auto-activation coverage.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_skills_api.py"],
            ),
            "Rust skill-management contracts remain normal CI coverage.",
        ),
        _case(
            "webui_v2_project_files_api_regression",
            "WebUI v2 project file and filesystem browser APIs",
            "Served WebUI v2 Project Filesystem API E2E",
            [f"REBCLI-049-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "webui_v2_project_files_served_e2e",
                "Served project-file and filesystem-browser API coverage.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_filesystem_api.py"],
            ),
            "Project UI browser journeys remain external-existing coverage.",
        ),
        _case(
            "webui_v2_product_auth_served_api_regression",
            "WebUI v2 product-auth served API route gates",
            "Served WebUI v2 Product-Auth API E2E",
            [
                "REBCLI-059-TC-08",
                "REBCLI-061-TC-09",
                "REBCLI-062-TC-07",
                "REBCLI-062-TC-08",
            ],
            _pytest_command(
                "webui_v2_product_auth_served_e2e",
                "Served product-auth bearer gates, manual-token setup, "
                "account projections, validation, and sanitized failures.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_product_auth_api.py"],
            ),
            "Live OAuth providers and browser auth-card workflows remain outside this lane.",
        ),
        _case(
            "webui_v2_operator_config_api_regression",
            "WebUI v2 LLM and operator configuration APIs",
            "Served WebUI v2 Operator Configuration API E2E",
            [f"REBCLI-048-TC-{index:02d}" for index in range(1, 7)],
            _pytest_command(
                "webui_v2_operator_config_served_e2e",
                "Served operator/LLM provider CRUD, active selection, "
                "diagnostics, logs, status, and secret-redaction coverage.",
                ["tests/e2e/scenarios/test_reborn_webui_v2_operator_api.py"],
            ),
            "LLM/embedding substrate cargo coverage remains normal CI coverage.",
        ),
    ]
)


def parse_duration_seconds(raw: str | None) -> int:
    if raw is None or not raw.strip():
        return DEFAULT_TIMEOUT_SECONDS
    value = raw.strip().lower()
    match = re.fullmatch(r"(\d+)([smh]?)", value)
    if not match:
        raise ValueError(f"invalid duration {raw!r}; use seconds, 30s, 45m, or 1h")
    amount = int(match.group(1))
    unit = match.group(2)
    if unit == "h":
        return amount * 60 * 60
    if unit == "m":
        return amount * 60
    return amount


def render_command(command: CommandSpec) -> str:
    unset_prefix = " ".join(f"unset {shlex.quote(name)};" for name in command.unset_env)
    env_prefix = " ".join(
        f"{name}={shlex.quote(value)}" for name, value in sorted(command.env.items())
    )
    rendered = " ".join(shlex.quote(part) for part in command.argv)
    prefix = " ".join(part for part in [unset_prefix, env_prefix] if part)
    if prefix:
        return f"{prefix} {rendered}"
    return rendered


def _now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _safe_log_name(name: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", name)


def _active_case_items() -> list[tuple[str, CaseSpec]]:
    return list(CASES.items())


def _selected_case_names(args: argparse.Namespace) -> list[str]:
    active_names = list(CASES)
    if not args.case:
        return [name for name, case in CASES.items() if case.default_enabled]

    names: list[str] = []
    for name in args.case:
        if name not in CASES:
            if name in REMOVED_CI_OWNED_CASES:
                raise SystemExit(
                    f"case {name!r} is existing-CI coverage and has been "
                    "removed from the executable QA lane"
                )
            raise SystemExit(
                f"unknown case {name!r}; valid executable cases: "
                f"{', '.join(active_names)}"
            )
        if name not in names:
            names.append(name)
    return names


def _test_ids_for(cases: list[CaseSpec]) -> list[str]:
    return sorted({test_id for case in cases for test_id in case.qa_matrix_test_ids})


def _case_has_matrix_only_command(case: CaseSpec) -> bool:
    return case.coverage_source == "matrix_only_or_new" and bool(case.commands)


def _case_existing_ci_only(case: CaseSpec) -> bool:
    return case.coverage_source == "existing_ci_only"


def _commands_for_case(case: CaseSpec) -> list[CommandSpec]:
    return case.commands


def _removed_existing_ci_commands(case: CaseSpec) -> list[dict[str, str | None]]:
    if not _case_existing_ci_only(case):
        return []
    return [
        {
            "name": command.name,
            "description": command.description,
            "command": render_command(command),
            "coverage_source": case.coverage_source,
            "existing_ci_coverage": "owned_by_existing_ci",
        }
        for command in case.commands
    ]


def write_case_manifest(output_dir: Path, selected_cases: list[str]) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    selected_specs = [CASES[name] for name in selected_cases]
    active_specs = [spec for _, spec in _active_case_items()]
    matrix_path = os.environ.get("REBORN_QA_MATRIX_PATH", "").strip()
    manifest = {
        "generated_at": _now_iso(),
        "selected_cases": selected_cases,
        "default_cases": [
            name for name, spec in CASES.items() if spec.default_enabled
        ],
        "qa_matrix": {
            "source": "local_xlsx",
            "path": matrix_path or None,
            "represented_test_ids": _test_ids_for(active_specs),
            "represented_test_id_count": len(_test_ids_for(active_specs)),
            "selected_represented_test_ids": _test_ids_for(selected_specs),
            "selected_represented_test_id_count": len(_test_ids_for(selected_specs)),
            "matrix_only_or_new_test_ids": _test_ids_for(active_specs),
            "existing_ci_only_test_ids": [],
        },
        "cases": [
            {
                "case": name,
                "feature": spec.feature,
                "category": spec.category,
                "qa_matrix_test_ids": spec.qa_matrix_test_ids,
                "default_enabled": spec.default_enabled,
                "coverage_source": spec.coverage_source,
                "mode": MODE,
                "notes": spec.notes,
                "commands": [
                    {
                        "name": command.name,
                        "description": command.description,
                        "command": render_command(command),
                        "coverage_source": spec.coverage_source,
                        "existing_ci_coverage": (
                            None
                            if spec.coverage_source == "matrix_only_or_new"
                            else "owned_by_existing_ci"
                        ),
                    }
                    for command in _commands_for_case(spec)
                ],
                "removed_existing_ci_commands": _removed_existing_ci_commands(spec),
            }
            for name, spec in _active_case_items()
        ],
    }
    path = output_dir / "case-manifest.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def run_command(
    command: CommandSpec,
    *,
    output_dir: Path,
    case_name: str,
    coverage_source: str,
    timeout_seconds: int,
    dry_run: bool,
) -> dict[str, Any]:
    log_base = f"{_safe_log_name(case_name)}.{_safe_log_name(command.name)}"
    stdout_log = output_dir / f"{log_base}.stdout.log"
    stderr_log = output_dir / f"{log_base}.stderr.log"
    details: dict[str, Any] = {
        "name": command.name,
        "description": command.description,
        "command": render_command(command),
        "stdout_log": str(stdout_log),
        "stderr_log": str(stderr_log),
        "dry_run": dry_run,
        "coverage_source": coverage_source,
        "existing_ci_coverage": (
            None
            if coverage_source == "matrix_only_or_new"
            else "owned_by_existing_ci"
        ),
    }
    if dry_run:
        stdout_log.write_text("", encoding="utf-8")
        stderr_log.write_text("", encoding="utf-8")
        details.update({"success": True, "returncode": None, "latency_ms": 0})
        return details

    env = os.environ.copy()
    for name in command.unset_env:
        env.pop(name, None)
    env.update(command.env)
    started = time.monotonic()
    with stdout_log.open("w", encoding="utf-8") as stdout, stderr_log.open(
        "w", encoding="utf-8"
    ) as stderr:
        try:
            completed = subprocess.run(
                command.argv,
                cwd=ROOT,
                env=env,
                stdout=stdout,
                stderr=stderr,
                text=True,
                timeout=timeout_seconds,
                check=False,
            )
            returncode: int | None = completed.returncode
            success = completed.returncode == 0
            error = None
        except subprocess.TimeoutExpired:
            stderr.write(
                f"\nTimed out after {timeout_seconds} seconds: "
                f"{render_command(command)}\n"
            )
            returncode = None
            success = False
            error = "timeout"
    details.update(
        {
            "success": success,
            "returncode": returncode,
            "latency_ms": int((time.monotonic() - started) * 1000),
        }
    )
    if error:
        details["error"] = error
    return details


def run_case(
    case: CaseSpec,
    *,
    output_dir: Path,
    timeout_seconds: int,
    dry_run: bool,
) -> dict[str, Any]:
    started = time.monotonic()
    command_results: list[dict[str, Any]] = []
    failed = False
    for command in _commands_for_case(case):
        if failed:
            command_results.append(
                {
                    "name": command.name,
                    "description": command.description,
                    "command": render_command(command),
                    "success": False,
                    "skipped": True,
                    "reason": "previous command failed",
                    "coverage_source": case.coverage_source,
                    "existing_ci_coverage": (
                        None
                        if case.coverage_source == "matrix_only_or_new"
                        else "owned_by_existing_ci"
                    ),
                }
            )
            continue
        result = run_command(
            command,
            output_dir=output_dir,
            case_name=case.name,
            coverage_source=case.coverage_source,
            timeout_seconds=timeout_seconds,
            dry_run=dry_run,
        )
        command_results.append(result)
        failed = not bool(result["success"])

    success = all(bool(result.get("success")) for result in command_results)
    return {
        "provider": PROVIDER,
        "mode": MODE,
        "case": case.name,
        "feature": case.feature,
        "category": case.category,
        "success": success,
        "latency_ms": int((time.monotonic() - started) * 1000),
        "details": {
            "qa_matrix_test_ids": case.qa_matrix_test_ids,
            "commands": command_results,
            "removed_existing_ci_commands": _removed_existing_ci_commands(case),
            "notes": case.notes,
        },
    }


def write_results(
    output_dir: Path,
    *,
    selected_cases: list[str],
    timeout_seconds: int,
    dry_run: bool,
    results: list[dict[str, Any]],
) -> Path:
    passed = sum(1 for result in results if result["success"])
    failed = len(results) - passed
    selected_specs = [CASES[name] for name in selected_cases]
    payload = {
        "provider": PROVIDER,
        "mode": MODE,
        "generated_at": _now_iso(),
        "success": failed == 0,
        "dry_run": dry_run,
        "run_existing_ci_coverage": False,
        "selected_cases": selected_cases,
        "timeout_seconds": timeout_seconds,
        "summary": {
            "passed": passed,
            "failed": failed,
            "total": len(results),
            "qa_matrix_test_ids": _test_ids_for(selected_specs),
            "matrix_only_or_new_qa_matrix_test_ids": _test_ids_for(selected_specs),
            "existing_ci_only_qa_matrix_test_ids": [],
        },
        "results": results,
    }
    path = output_dir / "results.json"
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return path


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=f"artifact directory (default: {DEFAULT_OUTPUT_DIR})",
    )
    parser.add_argument(
        "--case",
        action="append",
        help="case name to execute; may be repeated; defaults to all default cases",
    )
    parser.add_argument(
        "--timeout",
        default=os.environ.get("COMMAND_TIMEOUT"),
        help="per-command timeout, e.g. 1800, 30m, or 1h",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="write manifest/results without executing commands",
    )
    parser.add_argument(
        "--list-cases",
        action="store_true",
        help="print available cases and exit",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.list_cases:
        for name, spec in _active_case_items():
            default = "default" if spec.default_enabled else "targeted"
            print(f"{name}\t{default}\t{','.join(spec.qa_matrix_test_ids)}")
        return 0

    timeout_seconds = parse_duration_seconds(args.timeout)
    selected_cases = _selected_case_names(args)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    write_case_manifest(args.output_dir, selected_cases)
    results = [
        run_case(
            CASES[name],
            output_dir=args.output_dir,
            timeout_seconds=timeout_seconds,
            dry_run=args.dry_run,
        )
        for name in selected_cases
    ]
    results_path = write_results(
        args.output_dir,
        selected_cases=selected_cases,
        timeout_seconds=timeout_seconds,
        dry_run=args.dry_run,
        results=results,
    )
    print(str(results_path))
    return 0 if all(result["success"] for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
