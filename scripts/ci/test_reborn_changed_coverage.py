#!/usr/bin/env python3
"""Workflow-contract tests for strict changed-line and branch coverage."""

from __future__ import annotations

import ast
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

import reborn_changed_coverage as gate

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
        success = re.search(
            r"all ([1-9][0-9]*) changed-coverage self-tests passed",
            result.stdout,
        )
        self.assertIsNotNone(success, result.stdout)

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
        gate_step = workflow.split(
            "- name: Gate changed Reborn lines and branches", maxsplit=1
        )[1].split("\n      - name:", maxsplit=1)[0]
        self.assertIn('--head "$HEAD_SHA"', gate_step)
        self.assertNotIn(
            "--threshold",
            gate_step,
            "the strict line/branch gate has no threshold CLI and must gate every denominator",
        )
        self.assertRegex(
            workflow,
            re.compile(
                r"- name: Post sticky coverage comment\n"
                r"\s+if: .*always\(\).*github\.event_name == 'pull_request'",
            ),
        )


class ChangedCoverageDiscoveryTests(unittest.TestCase):
    """The gate must resolve production paths from the crate inventory.

    A `crates/ironclaw_*` pattern matches nothing once crates move into family
    directories, and this gate's "nothing matched" outcome is a silent pass
    ("no Reborn production lines added", exit 0). These pin the discovery so
    that shape cannot come back (docs/reborn/target-architecture/CHECKLIST.md
    WS10, #6963).
    """

    def test_no_string_literal_reintroduces_the_flat_crate_keying(self) -> None:
        """No *operative* string may key on `crates/ironclaw_*` again.

        Checked over the AST rather than the raw text so that prose explaining
        the retired pattern — in this gate's own module docstring, and in the
        comments that say why it was retired — does not read as a violation.
        """
        source = (ROOT / "scripts/ci/reborn_changed_coverage.py").read_text(
            encoding="utf-8"
        )
        tree = ast.parse(source)
        docstrings = {
            id(node.body[0].value)
            for node in ast.walk(tree)
            if isinstance(
                node, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)
            )
            and node.body
            and isinstance(node.body[0], ast.Expr)
            and isinstance(node.body[0].value, ast.Constant)
            and isinstance(node.body[0].value.value, str)
        }

        offenders = [
            node.value
            for node in ast.walk(tree)
            if isinstance(node, ast.Constant)
            and isinstance(node.value, str)
            and id(node) not in docstrings
            and ("crates/ironclaw_*" in node.value or "crates/ironclaw_[" in node.value)
        ]

        self.assertEqual(
            offenders,
            [],
            "the flat-tree keying is what goes silently dark under family "
            "directories; discovery must come from the crate inventory",
        )

    def test_diff_pathspecs_are_derived_from_the_crate_inventory(self) -> None:
        production = gate.ProductionPaths(ROOT)
        pathspecs = production.diff_pathspecs()

        self.assertTrue(pathspecs)
        crate_dirs = {
            spec.rsplit("/src/", 1)[0] for spec in pathspecs
        }
        for crate_dir in crate_dirs:
            with self.subTest(crate_dir=crate_dir):
                self.assertTrue(
                    (ROOT / crate_dir / "Cargo.toml").is_file(),
                    "every pathspec must name a directory that really owns a Cargo.toml",
                )
        self.assertEqual(len(pathspecs), 2 * len(crate_dirs))

    def test_production_classification_survives_family_nesting(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for index in range(25):
                crate = root / "crates/substrates" / f"ironclaw_nested{index}"
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "ironclaw_nested{index}"\n', encoding="utf-8"
                )
            production = gate.ProductionPaths(root)

            self.assertTrue(
                production.is_production(
                    "crates/substrates/ironclaw_nested0/src/lib.rs"
                )
            )
            # Attributable but not the crate's own `src/` — excluded, as the
            # old regex excluded `crates/ironclaw_safety/fuzz/src/main.rs`.
            self.assertFalse(
                production.is_production(
                    "crates/substrates/ironclaw_nested0/fuzz/src/main.rs"
                )
            )

    def test_unattributable_crate_path_is_refused_not_ignored(self) -> None:
        production = gate.ProductionPaths(ROOT)

        production.reject_unattributable("crates/AGENTS.md")
        production.reject_unattributable("src/main.rs")
        with self.assertRaises(gate.GateError):
            production.reject_unattributable("crates/not_a_crate/src/lib.rs")

    def test_missing_crate_tree_is_a_gate_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(gate.GateError):
                gate.ProductionPaths(Path(temp))


if __name__ == "__main__":
    unittest.main()
