#!/usr/bin/env python3
"""Sabotage tests for scheduled, merge, and release WS12 lanes."""

from __future__ import annotations

import copy
import unittest
from pathlib import Path

from ws12_workflow_contracts import (
    E2E_WORKFLOW,
    REQUIRED_MARKERS,
    load_workflows,
    validate_e2e_scope_filters,
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

    def test_reborn_e2e_scope_filters_pass_as_checked_in(self) -> None:
        self.assertEqual(validate_e2e_scope_filters(self.workflows[E2E_WORKFLOW]), [])

    def test_flat_tree_scope_regex_fails_loudly(self) -> None:
        """Re-narrowing the scope regex to `crates/ironclaw_*` must not pass.

        This is the exact regression the WS10 rewrite exists to prevent: the
        pattern still matches every path in today's flat tree, so nothing looks
        broken until crates move and every E2E job silently stops running.
        """
        sabotaged = self.workflows[E2E_WORKFLOW].replace(
            "grep -Eq '^(crates/|", "grep -Eq '^(crates/ironclaw_[^/]+/|"
        )
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(
            any("substrates/ironclaw_events" in error for error in errors), errors
        )

    def test_flat_tree_paths_glob_fails_loudly(self) -> None:
        sabotaged = self.workflows[E2E_WORKFLOW].replace(
            '- "crates/**"', '- "crates/ironclaw_*/**"'
        )
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(any("push `paths:` filter" in error for error in errors), errors)

    def test_over_broad_scope_regex_fails_loudly(self) -> None:
        """The filter must stay a filter — matching everything is not a fix."""
        sabotaged = self.workflows[E2E_WORKFLOW].replace(
            "grep -Eq '^(crates/|", "grep -Eq '^(|"
        )
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(any("must NOT be in scope" in error for error in errors), errors)

    def test_missing_scope_regex_fails_loudly(self) -> None:
        sabotaged = self.workflows[E2E_WORKFLOW].replace("grep -Eq '^(crates/|", "true #")
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(
            any("could not find the `changes` job scope regex" in e for e in errors),
            errors,
        )

    def test_code_style_runs_workflow_and_shard_sabotage_tests(self) -> None:
        workflow = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_ws12_suite_shards.py", workflow)
        self.assertIn("python3 scripts/ci/test_ws12_workflow_contracts.py", workflow)


if __name__ == "__main__":
    unittest.main()
