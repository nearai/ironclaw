#!/usr/bin/env python3
"""Contracts for changed production-package selection."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts/ci/changed_workspace_packages.py"
SPEC = importlib.util.spec_from_file_location("changed_workspace_packages", MODULE)
assert SPEC is not None and SPEC.loader is not None
selector = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = selector
SPEC.loader.exec_module(selector)


def metadata() -> dict:
    return {
        "workspace_members": ["root", "alpha", "nested"],
        "packages": [
            {
                "id": "root",
                "name": "root",
                "manifest_path": str(ROOT / "Cargo.toml"),
            },
            {
                "id": "alpha",
                "name": "alpha",
                "manifest_path": str(ROOT / "crates/alpha/Cargo.toml"),
            },
            {
                "id": "nested",
                "name": "nested",
                "manifest_path": str(ROOT / "crates/family/nested/Cargo.toml"),
            },
        ],
    }


class ChangedWorkspacePackagesTests(unittest.TestCase):
    def test_selects_direct_production_packages(self) -> None:
        self.assertEqual(
            selector.changed_production_packages(
                ["crates/alpha/src/lib.rs", "crates/family/nested/build.rs"],
                metadata(),
            ),
            ["alpha", "nested"],
        )

    def test_test_and_ci_only_changes_do_not_launch_clippy(self) -> None:
        self.assertEqual(
            selector.changed_production_packages(
                [
                    "crates/alpha/tests/contract.rs",
                    ".github/workflows/code_style.yml",
                ],
                metadata(),
            ),
            [],
        )

    def test_crate_manifest_selects_package_but_workspace_inputs_do_not(self) -> None:
        self.assertEqual(
            selector.changed_production_packages(
                ["Cargo.toml", "Cargo.lock", "crates/alpha/Cargo.toml"],
                metadata(),
            ),
            ["alpha"],
        )


if __name__ == "__main__":
    unittest.main()
