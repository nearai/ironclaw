#!/usr/bin/env python3
"""Fresh-machine auth canary runner.

Bootstraps the Python E2E environment, installs Playwright Chromium, builds the
libsql binary, and runs a focused auth matrix through both browser and API
paths.
"""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
E2E_DIR = ROOT / "tests" / "e2e"
DEFAULT_VENV = E2E_DIR / ".venv"
DEFAULT_OUTPUT_DIR = ROOT / "artifacts" / "auth-canary"

SMOKE_TESTS = [
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_roundtrip",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_roundtrip",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_roundtrip_via_browser",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_same_server_multi_user_via_browser",
]

FULL_TESTS = SMOKE_TESTS + [
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_provider_error_leaves_extension_unauthed",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_exchange_failure_leaves_extension_unauthed",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_first_chat_auth_attempt_emits_auth_url",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_chat_first_gmail_installs_prompts_and_retries",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_settings_first_gmail_auth_then_chat_runs",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_settings_first_custom_mcp_auth_then_chat_runs",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_tool_oauth_refresh_on_demand",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_refresh_on_demand",
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_mcp_oauth_refresh_on_start",
]

CHANNEL_TESTS = [
    "tests/e2e/scenarios/test_v2_auth_oauth_matrix.py::test_wasm_channel_oauth_roundtrip",
]

PROFILES: dict[str, list[str]] = {
    "smoke": SMOKE_TESTS,
    "full": FULL_TESTS,
    "channels": CHANNEL_TESTS,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Bootstrap a fresh-machine auth canary and run the browser/API auth matrix."
        )
    )
    parser.add_argument(
        "--profile",
        choices=sorted(PROFILES),
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


def run(cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    rendered = " ".join(shlex.quote(part) for part in cmd)
    print(f"+ {rendered}", flush=True)
    subprocess.run(cmd, cwd=cwd or ROOT, env=env, check=True)


def venv_python(venv_dir: Path) -> Path:
    if os.name == "nt":
        return venv_dir / "Scripts" / "python.exe"
    return venv_dir / "bin" / "python"


def ensure_tooling_present() -> None:
    missing = [tool for tool in ("cargo", sys.executable) if not tool]
    if missing:
        raise RuntimeError(f"Missing required tooling: {', '.join(missing)}")


def bootstrap_python(venv_dir: Path) -> Path:
    if not venv_dir.exists():
        run([sys.executable, "-m", "venv", str(venv_dir)])
    python = venv_python(venv_dir)
    run([str(python), "-m", "pip", "install", "--upgrade", "pip"])
    run([str(python), "-m", "pip", "install", "-e", str(E2E_DIR)])
    return python


def install_playwright(python: Path, mode: str) -> None:
    resolved = mode
    if mode == "auto":
        resolved = "with-deps" if os.environ.get("CI") else "plain"
    if resolved == "skip":
        return

    cmd = [str(python), "-m", "playwright", "install"]
    if resolved == "with-deps":
        cmd.append("--with-deps")
    cmd.append("chromium")
    run(cmd, cwd=E2E_DIR)


def cargo_build() -> None:
    run(["cargo", "build", "--no-default-features", "--features", "libsql"])


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
        *PROFILES[args.profile],
        *args.pytest_arg,
    ]
    run(cmd, cwd=ROOT, env=pytest_env())


def main() -> int:
    args = parse_args()
    tests = PROFILES[args.profile]
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
