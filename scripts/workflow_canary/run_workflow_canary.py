"""Workflow-canary runner: end-to-end coverage for issue #1044 scenarios.

Where ``auth-live-canary`` covers credential/auth flows, this lane covers
the broader user-facing workflows: chat-driven extension setup, routines
firing on cron schedules, multi-tool pipelines (Telegram → Sheets,
Calendar prep → Telegram, etc.).

Architecture mirrors auth-live-canary's runner:

- Reuses ``scripts.live_canary.common.start_gateway_stack`` for the
  bulk of the work (mock LLM + ironclaw subprocess + drainer threads
  + LLM settings pin via API).
- Adds a Telegram Bot API mock subprocess (``telegram_mock.py``) and
  routes IronClaw's outbound calls to it via
  ``IRONCLAW_TEST_HTTP_REMAP=api.telegram.org=<mock_url>`` so each
  scenario can verify Telegram side-effects without a real bot token.

CLI shape matches ``run_live_canary.py`` so the same lane wrapper
script (``scripts/live-canary/run.sh``) can drive both.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import subprocess
import sys
import time
from dataclasses import asdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.common import (  # noqa: E402
    DEFAULT_VENV,
    E2E_DIR,
    ProbeResult,
    bootstrap_python,
    cargo_build,
    install_playwright,
    start_gateway_stack,
    stop_gateway_stack,
    stop_process,
    venv_python,
    wait_for_port_line,
)

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "workflow-canary"

# Ordered list of scenario keys → (module, function, display name).
# Each scenario function takes (stack, mock_telegram_url, output_dir,
# log_dir) and returns a list[ProbeResult].
SCENARIOS: dict[str, tuple[str, str, str]] = {
    "bug_logger": (
        "scripts.workflow_canary.scenarios.bug_logger",
        "run",
        "Script 1 — Telegram → Google Sheet Bug Logger",
    ),
    "calendar_prep": (
        "scripts.workflow_canary.scenarios.calendar_prep",
        "run",
        "Script 2 — Calendar Prep Assistant",
    ),
    "hn_monitor": (
        "scripts.workflow_canary.scenarios.hn_monitor",
        "run",
        "Script 3 — Hacker News Keyword Monitor",
    ),
    "periodic_reminder": (
        "scripts.workflow_canary.scenarios.periodic_reminder",
        "run",
        "Script 4 — Periodic Reminder via Telegram",
    ),
    "crm_tracker": (
        "scripts.workflow_canary.scenarios.crm_tracker",
        "run",
        "Script 5 — Email → CRM Inbound Tracker",
    ),
    "manual_trigger": (
        "scripts.workflow_canary.scenarios.manual_trigger",
        "run",
        "Manual trigger — POST /api/routines/<id>/trigger",
    ),
    "lifecycle": (
        "scripts.workflow_canary.scenarios.lifecycle",
        "run",
        "Lifecycle — disable / enable / delete via routines API",
    ),
    "dedup_cooldown": (
        "scripts.workflow_canary.scenarios.dedup_cooldown",
        "run",
        "Dedup — cooldown_secs suppresses back-to-back fires",
    ),
    "nl_routine_create": (
        "scripts.workflow_canary.scenarios.nl_routine_create",
        "run",
        "NL routine create — POST /api/chat/send → routine_create tool",
    ),
    "nl_schedule_update": (
        "scripts.workflow_canary.scenarios.nl_schedule_update",
        "run",
        "NL schedule update — POST /api/chat/send → routine_update tool",
    ),
    "telegram_channel_install": (
        "scripts.workflow_canary.scenarios.telegram_channel_install",
        "run",
        "Telegram channel install + setup → /api/extensions",
    ),
    "telegram_round_trip": (
        "scripts.workflow_canary.scenarios.telegram_round_trip",
        "run",
        "Telegram inbound webhook → agent → outbound reply",
    ),
    "routine_visibility_from_telegram": (
        "scripts.workflow_canary.scenarios.routine_visibility_from_telegram",
        "run",
        "Telegram → agent → list-routines reply",
    ),
    "manual_trigger_from_telegram": (
        "scripts.workflow_canary.scenarios.manual_trigger_from_telegram",
        "run",
        "Manual routine trigger → ack returns to Telegram",
    ),
    "first_immediate_run": (
        "scripts.workflow_canary.scenarios.first_immediate_run",
        "run",
        "First check fires within 10s (fire_immediately=True)",
    ),
    "idempotent_disable_enable": (
        "scripts.workflow_canary.scenarios.idempotent_disable_enable",
        "run",
        "Disable / enable double-toggle is idempotent",
    ),
    "cron_timing_accuracy": (
        "scripts.workflow_canary.scenarios.cron_timing_accuracy",
        "run",
        "Cron */1 fires within ±15s of minute boundary",
    ),
    "auth_recovery": (
        "scripts.workflow_canary.scenarios.auth_recovery",
        "run",
        "Unauthenticated tool call surfaces graceful auth path (no 5xx / Error 400)",
    ),
    # log_assertions intentionally registered LAST so it scans the
    # gateway log surface produced by every preceding probe.
    "log_assertions": (
        "scripts.workflow_canary.scenarios.log_assertions",
        "run",
        "Gateway log scan — no chat_id 'default' / naive timestamp / etc.",
    ),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Workflow-canary runner. Exercises end-to-end multi-tool "
            "user workflows from issue #1044."
        )
    )
    parser.add_argument(
        "--scenario",
        action="append",
        choices=sorted(SCENARIOS),
        default=[],
        help=(
            "Limit the run to the listed scenarios. May be repeated. "
            "Default runs all scenarios."
        ),
    )
    parser.add_argument(
        "--venv",
        type=Path,
        default=DEFAULT_VENV,
        help=f"Virtualenv path (default: {DEFAULT_VENV})",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=f"Artifacts directory (default: {DEFAULT_OUTPUT_DIR})",
    )
    parser.add_argument(
        "--playwright-install",
        choices=("auto", "with-deps", "plain", "skip"),
        default="skip",
        help=(
            "How to install Playwright browsers. Default 'skip' since "
            "the workflow-canary scenarios don't drive a browser; the "
            "auth-live-canary lanes own that."
        ),
    )
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-python-bootstrap", action="store_true")
    return parser.parse_args()


def _spawn_mock(
    python: Path,
    *,
    script_name: str,
    port_line_pattern: str,
    log_filename: str,
    log_dir: Path,
) -> tuple[subprocess.Popen[str], str]:
    """Start a workflow-canary mock subprocess (telegram_mock.py /
    sheets_mock.py / calendar_mock.py / etc.) and return (process, url).

    The mock prints ``<MARKER>=<port>`` on stdout (e.g.
    ``MOCK_TELEGRAM_PORT=54321``); ``port_line_pattern`` is the regex
    used to extract that port. After discovery, a daemon thread drains
    the rest of stdout to a log file so the pipe never fills (same fix
    as ``scripts/live_canary/common.py f59981d3``).
    """
    proc = subprocess.Popen(
        [
            str(python),
            str(Path(__file__).parent / script_name),
            "--port",
            "0",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    match = wait_for_port_line(
        proc, re.compile(port_line_pattern), timeout=15.0
    )
    url = f"http://127.0.0.1:{match.group(1)}"

    log_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_dir / log_filename
    import threading

    def _drain() -> None:
        try:
            with log_path.open("a", encoding="utf-8", errors="replace") as fh:
                if proc.stdout is None:
                    return
                for line in proc.stdout:
                    fh.write(line)
                    fh.flush()
        except Exception:
            pass

    threading.Thread(target=_drain, daemon=True).start()
    return proc, url


def _spawn_mock_telegram(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    return _spawn_mock(
        python,
        script_name="telegram_mock.py",
        port_line_pattern=r"MOCK_TELEGRAM_PORT=(\d+)",
        log_filename="telegram_mock.log",
        log_dir=log_dir,
    )


def _spawn_mock_sheets(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    return _spawn_mock(
        python,
        script_name="sheets_mock.py",
        port_line_pattern=r"MOCK_SHEETS_PORT=(\d+)",
        log_filename="sheets_mock.log",
        log_dir=log_dir,
    )


def _spawn_mock_calendar(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    return _spawn_mock(
        python,
        script_name="calendar_mock.py",
        port_line_pattern=r"MOCK_CALENDAR_PORT=(\d+)",
        log_filename="calendar_mock.log",
        log_dir=log_dir,
    )


def _spawn_mock_hn(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    return _spawn_mock(
        python,
        script_name="hn_mock.py",
        port_line_pattern=r"MOCK_HN_PORT=(\d+)",
        log_filename="hn_mock.log",
        log_dir=log_dir,
    )


def _spawn_mock_gmail(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    return _spawn_mock(
        python,
        script_name="gmail_mock.py",
        port_line_pattern=r"MOCK_GMAIL_PORT=(\d+)",
        log_filename="gmail_mock.log",
        log_dir=log_dir,
    )


def _spawn_mock_web_search(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    return _spawn_mock(
        python,
        script_name="web_search_mock.py",
        port_line_pattern=r"MOCK_WEB_SEARCH_PORT=(\d+)",
        log_filename="web_search_mock.log",
        log_dir=log_dir,
    )


async def _run_scenarios(
    args: argparse.Namespace, log_dir: Path, results: list[ProbeResult]
) -> None:
    selected = args.scenario or list(SCENARIOS)

    python = venv_python(args.venv)
    mock_telegram_proc, mock_telegram_url = _spawn_mock_telegram(python, log_dir)
    mock_sheets_proc, mock_sheets_url = _spawn_mock_sheets(python, log_dir)
    mock_calendar_proc, mock_calendar_url = _spawn_mock_calendar(python, log_dir)
    mock_hn_proc, mock_hn_url = _spawn_mock_hn(python, log_dir)
    mock_gmail_proc, mock_gmail_url = _spawn_mock_gmail(python, log_dir)
    mock_web_search_proc, mock_web_search_url = _spawn_mock_web_search(
        python, log_dir
    )
    print(
        f"[workflow-canary] mock telegram     listening at {mock_telegram_url}",
        flush=True,
    )
    print(
        f"[workflow-canary] mock sheets       listening at {mock_sheets_url}",
        flush=True,
    )
    print(
        f"[workflow-canary] mock calendar     listening at {mock_calendar_url}",
        flush=True,
    )
    print(
        f"[workflow-canary] mock hn           listening at {mock_hn_url}",
        flush=True,
    )
    print(
        f"[workflow-canary] mock gmail        listening at {mock_gmail_url}",
        flush=True,
    )
    print(
        f"[workflow-canary] mock web_search   listening at {mock_web_search_url}",
        flush=True,
    )

    mock_procs = [
        mock_telegram_proc,
        mock_sheets_proc,
        mock_calendar_proc,
        mock_hn_proc,
        mock_gmail_proc,
        mock_web_search_proc,
    ]

    try:
        # Comma-separate IRONCLAW_TEST_HTTP_REMAP entries so IronClaw's
        # HostRemapHttpInterceptor builds a multi-host map. Order
        # doesn't matter for the parser — `host=base_url` pairs split on
        # commas. See src/http_intercept.rs:88-118.
        remap = ",".join(
            [
                f"api.telegram.org={mock_telegram_url}",
                f"sheets.googleapis.com={mock_sheets_url}",
                f"www.googleapis.com={mock_calendar_url}",
                f"news.ycombinator.com={mock_hn_url}",
                f"gmail.googleapis.com={mock_gmail_url}",
                f"api.search.brave.com={mock_web_search_url}",
            ]
        )
        stack = await start_gateway_stack(
            venv_dir=args.venv,
            owner_user_id="workflow-canary-owner",
            temp_prefix="ironclaw-workflow-canary",
            gateway_token_prefix="workflow-canary",
            extra_gateway_env={
                "IRONCLAW_TEST_HTTP_REMAP": remap,
                "ROUTINES_ENABLED": "true",
                "ROUTINES_CRON_INTERVAL": "2",
                "ROUTINES_DEFAULT_COOLDOWN": "0",
                # Routes the hardcoded validate_telegram_bot_token getMe
                # call (in src/extensions/manager.rs) to mock_telegram.
                # That getMe path bypasses the standard
                # IRONCLAW_TEST_HTTP_REMAP because it doesn't go through
                # the http_interceptor pipeline.
                "IRONCLAW_TEST_TELEGRAM_API_BASE_URL": mock_telegram_url,
            },
            log_dir=log_dir,
        )
    except Exception:
        for p in mock_procs:
            stop_process(p)
        raise

    try:
        for key in selected:
            module_name, fn_name, display = SCENARIOS[key]
            print(f"\n[workflow-canary] === {display} ===", flush=True)
            module = __import__(module_name, fromlist=[fn_name])
            scenario_fn = getattr(module, fn_name)
            scenario_results = await scenario_fn(
                stack=stack,
                mock_telegram_url=mock_telegram_url,
                mock_sheets_url=mock_sheets_url,
                mock_calendar_url=mock_calendar_url,
                mock_hn_url=mock_hn_url,
                mock_gmail_url=mock_gmail_url,
                mock_web_search_url=mock_web_search_url,
                output_dir=args.output_dir,
                log_dir=log_dir,
            )
            results.extend(scenario_results)
    finally:
        stop_gateway_stack(stack)
        for p in mock_procs:
            stop_process(p)


def _write_results(results: list[ProbeResult], output_dir: Path) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "results": [asdict(r) for r in results],
    }
    path = output_dir / "results.json"
    path.write_text(json.dumps(payload, indent=2, default=str))
    return path


REEXEC_ENV = "WORKFLOW_CANARY_REEXEC"


def main() -> int:
    args = parse_args()

    # Same bootstrap-then-reexec pattern as scripts/auth_live_canary/-
    # run_live_canary.py: install/upgrade the venv (which includes
    # httpx, aiohttp, etc.), then re-launch ourselves under that venv
    # with --skip-python-bootstrap so the rest of the script runs with
    # the right interpreter. Without this, the parent process keeps
    # executing under whatever Python invoked us — typically the
    # system one on CI runners, which doesn't have httpx.
    if not args.skip_python_bootstrap and os.environ.get(REEXEC_ENV) != "1":
        python = bootstrap_python(args.venv)
        if args.playwright_install != "skip":
            install_playwright(python, args.playwright_install)
        if not args.skip_build:
            cargo_build()
        cmd = [
            str(python),
            str(Path(__file__).resolve()),
            *sys.argv[1:],
            "--skip-python-bootstrap",
        ]
        env = os.environ.copy()
        env[REEXEC_ENV] = "1"
        return subprocess.run(cmd, cwd=ROOT, env=env, check=False).returncode
    if (
        args.skip_python_bootstrap
        and not venv_python(args.venv).exists()
        and os.environ.get(REEXEC_ENV) != "1"
    ):
        print(
            f"[workflow-canary] virtualenv not found at {venv_python(args.venv)}; "
            "remove --skip-python-bootstrap to bootstrap it",
            file=sys.stderr,
            flush=True,
        )
        return 1
    if args.playwright_install != "skip":
        install_playwright(venv_python(args.venv), args.playwright_install)
    if not args.skip_build:
        cargo_build()

    log_dir = args.output_dir
    log_dir.mkdir(parents=True, exist_ok=True)

    results: list[ProbeResult] = []
    try:
        asyncio.run(_run_scenarios(args, log_dir, results))
    except Exception as exc:
        print(f"[workflow-canary] error: {exc}", file=sys.stderr, flush=True)
        path = _write_results(results, args.output_dir)
        print(f"[workflow-canary] results: {path}", flush=True)
        return 1

    path = _write_results(results, args.output_dir)
    failures = [r for r in results if not r.success]
    if failures:
        print(
            f"\n[workflow-canary] {len(failures)} probe(s) failed. "
            f"Results: {path}",
            flush=True,
        )
        for r in failures:
            print(
                f"  ✗ {r.provider} / {r.mode}: "
                f"{r.details.get('error', '<no error>')}"
            )
        return 1
    print(
        f"\n[workflow-canary] all {len(results)} probe(s) passed. "
        f"Results: {path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
