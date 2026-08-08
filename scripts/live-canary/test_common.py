#!/usr/bin/env python3
"""Unit tests for shared live canary helpers.

Run with::

    python3 -m pytest scripts/live-canary/test_common.py -v

Or directly::

    python3 scripts/live-canary/test_common.py
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


_SPEC = importlib.util.spec_from_file_location(
    "live_canary_common",
    Path(__file__).resolve().parents[1] / "live_canary" / "common.py",
)
common = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = common
_SPEC.loader.exec_module(common)


class InstallPlaywrightTests(unittest.TestCase):
    def test_with_deps_retries_browser_only_install_when_dependency_install_fails(self):
        calls: list[list[str]] = []

        def fake_run(cmd: list[str], **_kwargs: object) -> None:
            calls.append(cmd)
            if "--with-deps" in cmd:
                raise subprocess.CalledProcessError(100, cmd)

        with patch.object(common, "run", side_effect=fake_run):
            common.install_playwright(Path("/venv/bin/python"), "with-deps")

        self.assertEqual(
            calls,
            [
                [
                    "/venv/bin/python",
                    "-m",
                    "playwright",
                    "install",
                    "--with-deps",
                    "chromium",
                ],
                ["/venv/bin/python", "-m", "playwright", "install", "chromium"],
            ],
        )

    def test_plain_install_failure_is_not_swallowed(self):
        error = subprocess.CalledProcessError(1, ["playwright"])

        with patch.object(common, "run", side_effect=error):
            with self.assertRaises(subprocess.CalledProcessError):
                common.install_playwright(Path("/venv/bin/python"), "plain")

    def test_gateway_env_propagates_compact_google_benchmark_flag(self):
        with patch.dict(
            common.os.environ,
            {"IRONCLAW_COMPACT_GOOGLE_CAPABILITIES_ENABLED": "false"},
        ):
            env = common.build_gateway_env(
                owner_user_id="owner",
                gateway_port=3000,
                http_port=3001,
                gateway_token="token",
                db_path=Path("/tmp/db"),
                home_dir=Path("/tmp/home"),
                tools_dir=Path("/tmp/tools"),
                channels_dir=Path("/tmp/channels"),
                mock_llm_url="http://127.0.0.1:3002",
                secrets_master_key="secret",
            )

        self.assertEqual(
            env["IRONCLAW_COMPACT_GOOGLE_CAPABILITIES_ENABLED"], "false"
        )


if __name__ == "__main__":
    unittest.main()
