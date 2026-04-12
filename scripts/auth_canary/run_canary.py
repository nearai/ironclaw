#!/usr/bin/env python3
"""Fresh-machine auth canary runner.

Bootstraps the Python E2E environment, installs Playwright Chromium, builds the
libsql binary, and runs a focused auth matrix through both browser and API
paths.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.auth_registry import AUTH_PROFILES
from scripts.live_canary.common import (
    DEFAULT_VENV,
    ROOT,
    bootstrap_python,
    cargo_build,
    install_playwright,
    run,
    venv_python,
)

DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-canary"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Bootstrap a fresh-machine auth canary and run the browser/API auth matrix."
        )
    )
    parser.add_argument(
        "--profile",
        choices=sorted(AUTH_PROFILES),
        default="smoke",
        help="Test profile to run. smoke is the default scheduled canary.",
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
        default="auto",
        help=(
            "How to install Playwright browsers. auto uses --with-deps in CI and "
            "plain locally."
        ),
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build and rely on the pytest fixture to use an existing binary.",
    )
    parser.add_argument(
        "--skip-python-bootstrap",
        action="store_true",
        help="Skip venv creation and pip install.",
    )
    parser.add_argument(
        "--pytest-arg",
        action="append",
        default=[],
        help="Extra argument to pass through to pytest. Repeat for multiple values.",
    )
    parser.add_argument(
        "--list-tests",
        action="store_true",
        help="Print the resolved test list and exit.",
    )
    return parser.parse_args()


def ensure_tooling_present() -> None:
    missing = [tool for tool in ("cargo", sys.executable) if not tool]
    if missing:
        raise RuntimeError(f"Missing required tooling: {', '.join(missing)}")


def pytest_env() -> dict[str, str]:
    env = os.environ.copy()
    env.setdefault("PYTHONUNBUFFERED", "1")
    return env


def run_pytest(args: argparse.Namespace, python: Path) -> None:
    output_dir = args.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)
    junit = output_dir / "auth-canary-junit.xml"

    cmd = [
        str(python),
        "-m",
        "pytest",
        "-v",
        "--timeout=120",
        f"--junitxml={junit}",
        *AUTH_PROFILES[args.profile],
        *args.pytest_arg,
    ]
    run(cmd, cwd=ROOT, env=pytest_env())


def main() -> int:
    args = parse_args()
    tests = AUTH_PROFILES[args.profile]
    if args.list_tests:
        for test in tests:
            print(test)
        return 0

    ensure_tooling_present()
    python = venv_python(args.venv)
    if not args.skip_python_bootstrap:
        python = bootstrap_python(args.venv)
        install_playwright(python, args.playwright_install)
    elif not python.exists():
        raise RuntimeError(
            f"Virtualenv Python not found at {python}. Remove --skip-python-bootstrap or create it first."
        )

    if not args.skip_build:
        cargo_build()

    run_pytest(args, python)
    print(
        f"\nAuth canary profile '{args.profile}' passed. Artifacts: {args.output_dir}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
