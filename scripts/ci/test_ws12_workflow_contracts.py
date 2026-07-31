#!/usr/bin/env python3
"""Sabotage tests for scheduled, merge, and release WS12 lanes."""

from __future__ import annotations

import copy
import unittest
from pathlib import Path

from ws12_workflow_contracts import (
    REQUIRED_MARKERS,
    load_workflows,
    validate_workflow_texts,
)

ROOT = Path(__file__).resolve().parents[2]


class WorkflowContractSabotageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflows = load_workflows(ROOT)

    def test_checked_in_workflows_cover_every_lane(self) -> None:
        self.assertEqual(validate_workflow_texts(self.workflows), [])

    def test_removing_each_lane_marker_fails_loudly(self) -> None:
        for path, markers in REQUIRED_MARKERS.items():
            for marker in markers:
                with self.subTest(path=path, marker=marker):
                    sabotaged = copy.deepcopy(self.workflows)
                    sabotaged[path] = sabotaged[path].replace(marker, "")
                    errors = validate_workflow_texts(sabotaged)
                    self.assertTrue(
                        any(path in error and marker in error for error in errors),
                        errors,
                    )

    def test_missing_workflow_fails_loudly(self) -> None:
        sabotaged = copy.deepcopy(self.workflows)
        path = next(iter(REQUIRED_MARKERS))
        del sabotaged[path]

        self.assertIn(f"missing workflow: {path}", validate_workflow_texts(sabotaged))

    def test_unconditional_skip_fails_loudly(self) -> None:
        path = ".github/workflows/nightly-deep-ci.yml"
        conditions = (
            "if: false",
            "if:  false",
            "if: 'false'",
            "if: |\n      false",
        )
        for condition in conditions:
            with self.subTest(condition=condition):
                sabotaged = copy.deepcopy(self.workflows)
                sabotaged[path] += f"\n  disabled-lane:\n    {condition}\n"

                self.assertTrue(
                    any(
                        "unconditionally skipped" in error
                        for error in validate_workflow_texts(sabotaged)
                    )
                )

    def test_code_style_runs_workflow_and_shard_sabotage_tests(self) -> None:
        workflow = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_ws12_suite_shards.py", workflow)
        self.assertIn("python3 scripts/ci/test_ws12_workflow_contracts.py", workflow)

    def test_code_style_fast_checks_share_one_runner_without_losing_gates(self) -> None:
        workflow = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("  fast-checks:", workflow)
        for marker in (
            "cargo fmt --all -- --check",
            "EmbarkStudios/cargo-deny-action@",
            "git ls-files -ci --exclude-standard",
            "scripts/ci/check-include-str-paths.sh",
            "scripts/ci/check-hermetic-env.sh",
            "scripts/check_no_panics.py --reborn-baseline",
            "scripts/ci/check-composition-budget.sh",
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, workflow)

        for retired_job in (
            "  clippy-matrix:",
            "  format:",
            "  deny-check:",
            "  tracked-ignored-files:",
            "  static-checks:",
            "  no-panics:",
            "  composition-budget:",
        ):
            with self.subTest(retired_job=retired_job):
                self.assertNotIn(retired_job, workflow)


if __name__ == "__main__":
    unittest.main()
