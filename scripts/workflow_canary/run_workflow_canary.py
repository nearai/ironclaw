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


def _spawn_mock_telegram(
    python: Path, log_dir: Path
) -> tuple[subprocess.Popen[str], str]:
    """Start the mock Telegram Bot API server and return (process, url)."""
    proc = subprocess.Popen(
        [
            str(python),
            str(Path(__file__).parent / "telegram_mock.py"),
            "--port",
            "0",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    match = wait_for_port_line(
        proc, re.compile(r"MOCK_TELEGRAM_PORT=(\d+)"), timeout=15.0
    )
    url = f"http://127.0.0.1:{match.group(1)}"

    # Drain remaining stdout to a log file so the pipe doesn't fill —
    # same lesson as scripts/live_canary/common.py f59981d3.
    log_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_dir / "telegram_mock.log"
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


async def _run_scenarios(
    args: argparse.Namespace, log_dir: Path, results: list[ProbeResult]
) -> None:
    selected = args.scenario or list(SCENARIOS)

    python = venv_python(args.venv)
    mock_telegram_proc, mock_telegram_url = _spawn_mock_telegram(python, log_dir)
    print(
        f"[workflow-canary] mock telegram listening at {mock_telegram_url}",
        flush=True,
    )

    try:
        stack = await start_gateway_stack(
            venv_dir=args.venv,
            owner_user_id="workflow-canary-owner",
            temp_prefix="ironclaw-workflow-canary",
            gateway_token_prefix="workflow-canary",
            extra_gateway_env={
                # Route the WASM telegram tool's outbound calls to our
                # mock. The remap is honored by IronClaw's WASM HTTP
                # client when the binary is built with debug_assertions.
                "IRONCLAW_TEST_HTTP_REMAP": (
                    f"api.telegram.org={mock_telegram_url}"
                ),
                # The auth-live-canary stack disables routines by default
                # (ROUTINES_ENABLED=false in build_gateway_env). The
                # workflow-canary scenarios fire routines as their core
                # under-test surface, so re-enable + tighten the cron
                # tick interval so backdated routines fire within ~2 s
                # instead of the default 15 s.
                "ROUTINES_ENABLED": "true",
                "ROUTINES_CRON_INTERVAL": "2",
                "ROUTINES_DEFAULT_COOLDOWN": "0",
            },
            log_dir=log_dir,
        )
    except Exception:
        stop_process(mock_telegram_proc)
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
                output_dir=args.output_dir,
                log_dir=log_dir,
            )
            results.extend(scenario_results)
    finally:
        stop_gateway_stack(stack)
        stop_process(mock_telegram_proc)


def _write_results(results: list[ProbeResult], output_dir: Path) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "results": [asdict(r) for r in results],
    }
    path = output_dir / "results.json"
    path.write_text(json.dumps(payload, indent=2, default=str))
    return path


def main() -> int:
    args = parse_args()

    if not args.skip_python_bootstrap:
        bootstrap_python(args.venv)
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
