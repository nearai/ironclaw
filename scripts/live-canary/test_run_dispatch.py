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

    def test_reborn_all_cases_dispatches_all_cases_flag(self):
        result = self.run_dispatch(cases="all")

        self.assertIn("scripts/reborn_webui_v2_live_qa/run_live_qa.py", result.stdout)
        self.assertIn("--all-cases", result.stdout)
        self.assertNotIn("--case all", result.stdout)

    def test_reborn_specific_cases_dispatch_as_repeated_case_flags(self):
        result = self.run_dispatch(
            cases="qa_3b_endpoint_status_live_chat, qa_8b_hn_keyword_live_chat"
        )

        self.assertIn("--case qa_3b_endpoint_status_live_chat", result.stdout)
        self.assertIn("--case qa_8b_hn_keyword_live_chat", result.stdout)
        self.assertNotIn("--all-cases", result.stdout)


if __name__ == "__main__":
    unittest.main()
