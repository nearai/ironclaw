"""Contract tests for the canonical integration-test inventory."""

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/ci/lib"))

import integration_test_inventory as inventory  # noqa: E402


class IntegrationTestInventoryTests(unittest.TestCase):
    def run_group_runner(
        self, root: Path, registrations: list[tuple[str, str]]
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        manifest = "".join(
            f'[[test]]\nname = "{name}"\npath = "{path}"\n\n'
            for name, path in registrations
        )
        (root / "Cargo.toml").write_text(manifest, encoding="utf-8")

        bin_dir = root / "bin"
        bin_dir.mkdir()
        command_log = root / "commands.log"
        for command, body in (
            (
                "timeout",
                """#!/usr/bin/env bash
printf 'timeout:%s\\n' "$*" >>"${COMMAND_LOG}"
while [[ "$1" == --* ]]; do shift; done
shift
"$@"
""",
            ),
            (
                "cargo",
                """#!/usr/bin/env bash
printf 'cargo:%s\\n' "$*" >>"${COMMAND_LOG}"
""",
            ),
        ):
            executable = bin_dir / command
            executable.write_text(body, encoding="utf-8")
            executable.chmod(0o755)

        env = os.environ.copy()
        env.update(
            {
                "COMMAND_LOG": str(command_log),
                "PATH": f"{bin_dir}:{env['PATH']}",
                "REBORN_GROUP_TEST_TIMEOUT": "9m",
            }
        )
        completed = subprocess.run(
            [str(ROOT / "scripts/ci/run-reborn-group-tests.sh")],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        log = command_log.read_text(encoding="utf-8") if command_log.exists() else ""
        return completed, log

    def test_preserves_current_registration_projections(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text(
                """
test = [
  { name = "reborn_integration_before", path = "tests/integration/duplicate.rs" },
  { name = 7, path = "tests/integration/ignored_name.rs" },
  { name = "ignored_path", path = 7 },
  { name = "outside_scope", path = "tests/other.rs" },
  { name = "reborn_group_shared", path = "tests/integration/group_shared/main.rs" },
  { name = "reborn_integration_after", path = "tests/integration/duplicate.rs" },
  { name = "reborn_integration_after", path = "tests/integration/second.rs" },
]
""",
                encoding="utf-8",
            )

            self.assertEqual(
                inventory.cargo_test_names(root),
                ["reborn_group_shared", "reborn_integration_after", "reborn_integration_before"],
            )
            self.assertEqual(
                inventory.planner_test_lanes(root),
                {
                    "tests/integration/duplicate.rs": 1,
                    "tests/integration/group_shared/main.rs": "groups",
                    "tests/integration/second.rs": 1,
                },
            )

            (root / "Cargo.toml").write_text(
                'test = [{ name = "new_test", path = "tests/integration/new_test.rs" }]\n',
                encoding="utf-8",
            )
            self.assertEqual(inventory.cargo_test_names(root), ["new_test"])
            for projection in (inventory.planner_test_lanes, inventory.inventory_document):
                with self.subTest(projection=projection.__name__):
                    with self.assertRaisesRegex(ValueError, "unsupported.*new_test"):
                        projection(root)

    def test_document_is_versioned_and_self_validating(self) -> None:
        document = inventory.inventory_document(ROOT)

        self.assertEqual(inventory.INTEGRATION_PARTITION_COUNT, 4)
        self.assertEqual(document["schema_version"], 1)
        self.assertEqual(
            document["partition_count"], inventory.INTEGRATION_PARTITION_COUNT
        )
        self.assertEqual(inventory.validate_inventory_document(document), document)

        invalid_fields = (
            ("schema_version", True),
            ("schema_version", 1.0),
            ("partition_count", 0),
            ("partition_count", 4.0),
        )
        for field, value in invalid_fields:
            malformed = dict(document)
            malformed[field] = value
            with self.subTest(field=field, value=value):
                with self.assertRaisesRegex(ValueError, field):
                    inventory.validate_inventory_document(malformed)

    def test_live_repository_group_topology_is_valid(self) -> None:
        completed = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/ci/lib/integration_test_inventory.py"),
                "--validate-group-topology",
                str(ROOT),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(
            completed.returncode,
            0,
            "live integration group topology validation failed:\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}",
        )

    def test_group_runner_rejects_invalid_topology_before_execution(self) -> None:
        cases = (
            ([], (), "No integration test group directories"),
            ([], ("group_orphan/main.rs",), "unregistered integration test group"),
            ([], ("group_incomplete/scenario.rs",), "missing main.rs"),
            (
                [("reborn_group_declared", "tests/integration/group_actual/main.rs")],
                ("group_actual/main.rs",),
                "group registration path mismatch",
            ),
            (
                [("reborn_group_missing", "tests/integration/group_missing/main.rs")],
                (),
                "missing main.rs",
            ),
            (
                [
                    ("reborn_group_valid", "tests/integration/group_valid/main.rs"),
                    ("reborn_group_outside", "tests/other.rs"),
                ],
                ("group_valid/main.rs",),
                "group registration path mismatch",
            ),
        )
        for registrations, group_entries, error in cases:
            with self.subTest(error=error), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for entry in group_entries:
                    group_file = root / "tests/integration" / entry
                    group_file.parent.mkdir(parents=True)
                    group_file.touch()

                completed, command_log = self.run_group_runner(root, registrations)

                self.assertNotEqual(completed.returncode, 0)
                self.assertIn(error, completed.stderr)
                self.assertEqual(command_log, "")

    def test_group_runner_preserves_valid_execution_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for suffix in ("zeta", "alpha"):
                group = root / f"tests/integration/group_{suffix}"
                group.mkdir(parents=True)
                (group / "main.rs").touch()

            completed, command_log = self.run_group_runner(
                root,
                [
                    ("reborn_group_zeta", "tests/integration/group_zeta/main.rs"),
                    ("reborn_group_alpha", "tests/integration/group_alpha/main.rs"),
                ],
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(
                command_log.splitlines(),
                [
                    "timeout:--signal=INT --kill-after=30s 9m cargo test -p "
                    "ironclaw_integration_tests --test reborn_group_alpha "
                    "--ignore-rust-version -- --nocapture",
                    "cargo:test -p ironclaw_integration_tests --test "
                    "reborn_group_alpha --ignore-rust-version -- --nocapture",
                    "timeout:--signal=INT --kill-after=30s 9m cargo test -p "
                    "ironclaw_integration_tests --test reborn_group_zeta "
                    "--ignore-rust-version -- --nocapture",
                    "cargo:test -p ironclaw_integration_tests --test "
                    "reborn_group_zeta --ignore-rust-version -- --nocapture",
                ],
            )


if __name__ == "__main__":
    unittest.main()
