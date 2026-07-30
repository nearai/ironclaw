#!/usr/bin/env python3
"""Sabotage tests for changed-production-line coverage enforcement."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

from reborn_changed_coverage import evaluate

ROOT = Path(__file__).resolve().parents[2]


def _diff(path: str, start: int, count: int) -> str:
    return (
        f"diff --git a/{path} b/{path}\n"
        f"--- a/{path}\n"
        f"+++ b/{path}\n"
        f"@@ -1,0 +{start},{count} @@\n" + "".join("+changed\n" for _ in range(count))
    )


def _lcov(path: str, lines: dict[int, int]) -> str:
    body = "".join(f"DA:{line},{hits}\n" for line, hits in sorted(lines.items()))
    return f"TN:\nSF:{path}\n{body}end_of_record\n"


class ChangedCoverageTests(unittest.TestCase):
    def test_changed_instrumented_lines_meeting_threshold_pass(self):
        path = "crates/ironclaw_example/src/lib.rs"

        report = evaluate(_diff(path, 10, 2), _lcov(path, {10: 1, 11: 3}), 90.0)

        self.assertTrue(report["passed"])
        self.assertEqual(report["covered_lines"], 2)
        self.assertEqual(report["instrumented_lines"], 2)

    def test_uncovered_changed_line_fails_even_when_another_line_is_covered(self):
        path = "crates/ironclaw_example/src/lib.rs"

        report = evaluate(_diff(path, 10, 2), _lcov(path, {10: 1, 11: 0}), 90.0)

        self.assertFalse(report["passed"])
        self.assertEqual(report["coverage_percent"], 50.0)
        self.assertEqual(report["uncovered"], [f"{path}:11"])

    def test_changed_production_file_absent_from_lcov_fails_loudly(self):
        path = "crates/ironclaw_unlinked/src/lib.rs"

        report = evaluate(_diff(path, 1, 1), "", 90.0)

        self.assertFalse(report["passed"])
        self.assertEqual(report["missing_files"], [path])

    def test_non_instrumentable_changed_lines_do_not_inflate_denominator(self):
        path = "crates/ironclaw_example/src/lib.rs"

        report = evaluate(_diff(path, 10, 3), _lcov(path, {11: 1}), 90.0)

        self.assertTrue(report["passed"])
        self.assertEqual(report["instrumented_lines"], 1)

    def test_test_sources_are_not_counted_as_changed_product_code(self):
        path = "crates/ironclaw_example/tests/contract.rs"

        report = evaluate(_diff(path, 1, 2), "", 90.0)

        self.assertTrue(report["passed"])
        self.assertEqual(report["changed_product_files"], [])

    def test_src_test_modules_are_not_counted_as_changed_product_code(self):
        paths = [
            "crates/ironclaw_example/src/tests.rs",
            "crates/ironclaw_example/src/projection/tests/contract.rs",
        ]

        for path in paths:
            with self.subTest(path=path):
                report = evaluate(_diff(path, 1, 2), "", 90.0)

                self.assertTrue(report["passed"])
                self.assertEqual(report["changed_product_files"], [])

    def test_owned_exemption_removes_file_from_changed_code_gate(self):
        path = "crates/ironclaw_example/src/generated.rs"

        report = evaluate(
            _diff(path, 1, 1),
            "",
            90.0,
            exempt_modules={path},
        )

        self.assertTrue(report["passed"])
        self.assertEqual(report["changed_product_files"], [])

    def test_deleting_gate_input_cannot_pass_vacuously(self):
        with self.assertRaisesRegex(ValueError, "diff input is empty"):
            evaluate("", "", 90.0)

    def test_reborn_workflow_runs_self_test_and_pr_gate(self):
        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_reborn_changed_coverage.py", workflow)
        self.assertIn("python3 scripts/ci/reborn_changed_coverage.py", workflow)
        self.assertIn("github.event.pull_request.base.sha", workflow)
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
