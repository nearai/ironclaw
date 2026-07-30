#!/usr/bin/env python3
"""Workflow-contract tests for strict changed-line and branch coverage."""

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class ChangedCoverageWorkflowTests(unittest.TestCase):
    def test_strict_gate_sabotage_harness_passes(self):
        result = subprocess.run(
            ["bash", "scripts/ci/test-reborn-changed-coverage.sh"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("all 25 changed-coverage self-tests passed", result.stdout)

    def test_reborn_workflow_runs_strict_gate_and_preserves_machine_report(self):
        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_reborn_changed_coverage.py", workflow)
        self.assertIn("python3 scripts/ci/reborn_changed_coverage.py", workflow)
        self.assertIn("tests/integration/changed-coverage-exemptions.toml", workflow)
        self.assertIn("github.event.pull_request.base.sha", workflow)
        self.assertIn("github.event.pull_request.head.sha", workflow)
        self.assertIn('--head "$HEAD_SHA"', workflow)
        self.assertNotIn('--head "$(git rev-parse HEAD)"', workflow)
        self.assertIn("github.event.merge_group.base_sha", workflow)
        self.assertIn("reborn-changed-coverage.json", workflow)
        self.assertRegex(
            workflow,
            re.compile(
                r"- name: Post sticky coverage comment\n"
                r"\s+if: .*always\(\).*github\.event_name == 'pull_request'",
            ),
        )


if __name__ == "__main__":
    unittest.main()
