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
                "dependencies": [{"name": "nested"}],
            },
            {
                "id": "alpha",
                "name": "alpha",
                "manifest_path": str(ROOT / "crates/alpha/Cargo.toml"),
                "dependencies": [],
            },
            {
                "id": "nested",
                "name": "nested",
                "manifest_path": str(ROOT / "crates/family/nested/Cargo.toml"),
                "dependencies": [{"name": "alpha"}],
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

    def test_crate_manifest_selects_its_package(self) -> None:
        self.assertEqual(
            selector.changed_production_packages(
                ["crates/alpha/Cargo.toml"],
                metadata(),
            ),
            ["alpha"],
        )

    def test_workspace_manifest_or_lockfile_selects_the_full_workspace(self) -> None:
        for path in ("Cargo.toml", "Cargo.lock"):
            with self.subTest(path=path):
                self.assertEqual(
                    selector.changed_production_packages([path], metadata()),
                    ["alpha", "nested", "root"],
                )

    def test_pull_request_scope_preserves_production_only_selection(self) -> None:
        self.assertEqual(
            selector.classify_clippy_scope(
                ["crates/alpha/src/lib.rs", "crates/alpha/tests/contract.rs"],
                metadata(),
                event="pull_request",
            ),
            {"mode": "selected", "packages": ["alpha"]},
        )

    def test_merge_group_selects_changed_package_test_surfaces(self) -> None:
        self.assertEqual(
            selector.classify_clippy_scope(
                [
                    "crates/alpha/tests/contract.rs",
                    "crates/family/nested/examples/demo.rs",
                ],
                metadata(),
                event="merge_group",
            ),
            {"mode": "selected", "packages": ["alpha", "nested"]},
        )

    def test_merge_group_production_changes_select_reverse_dependency_closure(
        self,
    ) -> None:
        self.assertEqual(
            selector.classify_clippy_scope(
                ["crates/alpha/src/lib.rs"],
                metadata(),
                event="merge_group",
            ),
            {"mode": "selected", "packages": ["alpha", "nested", "root"]},
        )

    def test_merge_group_test_surface_selects_only_owning_package(self) -> None:
        self.assertEqual(
            selector.classify_clippy_scope(
                ["crates/alpha/tests/contract.rs"],
                metadata(),
                event="merge_group",
            ),
            {"mode": "selected", "packages": ["alpha"]},
        )

    def test_merge_group_global_inputs_escalate_to_full(self) -> None:
        for path in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            ".github/workflows/code_style.yml",
            ".github/actions/setup-rust/action.yml",
            "scripts/ci/changed_workspace_packages.py",
        ):
            with self.subTest(path=path):
                self.assertEqual(
                    selector.classify_clippy_scope(
                        [path], metadata(), event="merge_group"
                    ),
                    {"mode": "full", "packages": []},
                )

    def test_merge_group_unknown_crate_path_escalates_to_full(self) -> None:
        for path in ("crates/deleted/src/lib.rs", "Makefile"):
            with self.subTest(path=path):
                self.assertEqual(
                    selector.classify_clippy_scope(
                        [path], metadata(), event="merge_group"
                    ),
                    {"mode": "full", "packages": []},
                )

    def test_empty_merge_group_diff_fails_fast(self) -> None:
        with self.assertRaisesRegex(ValueError, "empty merge-group diff"):
            selector.classify_clippy_scope([], metadata(), event="merge_group")

    def test_non_code_diff_selects_no_clippy_scope(self) -> None:
        for event in ("pull_request", "merge_group"):
            with self.subTest(event=event):
                self.assertEqual(
                    selector.classify_clippy_scope(
                        ["docs/using/cli.mdx"], metadata(), event=event
                    ),
                    {"mode": "none", "packages": []},
                )


if __name__ == "__main__":
    unittest.main()
