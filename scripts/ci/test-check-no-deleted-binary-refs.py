#!/usr/bin/env python3
"""Regression tests for check-no-deleted-binary-refs.py."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/ci/check-no-deleted-binary-refs.py"


class DeletedBinaryReferenceCheckTests(unittest.TestCase):
    def run_checker(self, root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(CHECKER), "--root", str(root)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )

    def test_clean_executable_scope_passes(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            workflow = root / ".github/workflows/reborn-e2e.yml"
            workflow.parent.mkdir(parents=True)
            workflow.write_text(
                "steps:\n  - run: cargo build -p ironclaw --bin ironclaw\n",
                encoding="utf-8",
            )

            result = self.run_checker(root)

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("No executable references", result.stdout)

    def test_deleted_binary_reference_deliberately_fails(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            workflow = root / ".github/workflows/regression.yml"
            workflow.parent.mkdir(parents=True)
            removed_binary = "ironclaw" + "-legacy"
            workflow.write_text(
                f"steps:\n  - run: cargo build --bin {removed_binary}\n",
                encoding="utf-8",
            )

            result = self.run_checker(root)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("regression.yml:2", result.stdout)
        self.assertIn("removed binary", result.stdout)


if __name__ == "__main__":
    unittest.main()
