#!/usr/bin/env python3
"""Unit tests for scripts/live-canary/run.sh dispatch behavior.

Run with::

    python3 -m pytest scripts/live-canary/test_run_dispatch.py -v

Or directly::

    python3 scripts/live-canary/test_run_dispatch.py
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RUN_SH = ROOT / "scripts" / "live-canary" / "run.sh"


class RunShDispatchTests(unittest.TestCase):
    def run_dispatch(self, *, cases: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmpdir:
            env = {
                **os.environ,
                "ARTIFACT_ROOT": tmpdir,
                "CASES": cases,
                "LANE": "reborn-webui-v2-live-qa",
                "PLAYWRIGHT_INSTALL": "skip",
                "PROVIDER": "reborn-webui-v2",
                "PYTHON_BIN": "echo",
                "TIMESTAMP": "dispatch-test",
            }
            return subprocess.run(
                [str(RUN_SH)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                check=True,
            )

    def test_reborn_all_cases_dispatches_non_telegram_qa_flag(self):
        result = self.run_dispatch(cases="all")

        self.assertIn("scripts/reborn_webui_v2_live_qa/run_live_qa.py", result.stdout)
        self.assertIn("--non-telegram-qa-cases", result.stdout)
        self.assertNotIn("--all-cases", result.stdout)
        self.assertNotIn("--case all", result.stdout)

    def test_reborn_specific_cases_dispatch_as_repeated_case_flags(self):
        result = self.run_dispatch(
            cases="qa_3b_endpoint_status_live_chat, qa_8b_hn_keyword_live_chat"
        )

        self.assertIn("--case qa_3b_endpoint_status_live_chat", result.stdout)
        self.assertIn("--case qa_8b_hn_keyword_live_chat", result.stdout)
        self.assertNotIn("--all-cases", result.stdout)

    def test_retired_legacy_gateway_lanes_are_not_dispatchable(self):
        retired_lanes = (
            "auth-smoke",
            "auth-full",
            "auth-channels",
            "auth-live-seeded",
            "auth-browser-consent",
            "workflow-canary",
        )

        for lane in retired_lanes:
            with self.subTest(lane=lane), tempfile.TemporaryDirectory() as tmpdir:
                env = {
                    **os.environ,
                    "ARTIFACT_ROOT": tmpdir,
                    "LANE": lane,
                    "PROVIDER": "mock",
                    "PYTHON_BIN": "echo",
                    "TIMESTAMP": "dispatch-test",
                }
                result = subprocess.run(
                    [str(RUN_SH)],
                    cwd=ROOT,
                    env=env,
                    text=True,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    check=False,
                )

                self.assertEqual(result.returncode, 2)
                self.assertIn(f"Unknown live canary lane: {lane}", result.stdout)
                self.assertIn("MIGRATION.md", result.stdout)

    def test_workflow_exposes_only_retained_dispatch_choices(self):
        workflow = (ROOT / ".github" / "workflows" / "live-canary.yml").read_text(
            encoding="utf-8"
        )
        choices = workflow.split("options:", 1)[1].split("scenario:", 1)[0]

        for lane in (
            "auth-smoke",
            "auth-full",
            "auth-channels",
            "auth-live-seeded",
            "auth-browser-consent",
            "workflow-canary",
        ):
            with self.subTest(lane=lane):
                self.assertNotIn(f"- {lane}", choices)

        self.assertIn("- reborn-webui-v2-live-qa", choices)


if __name__ == "__main__":
    unittest.main()
