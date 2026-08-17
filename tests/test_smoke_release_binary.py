from __future__ import annotations

import importlib.util
import io
import json
import os
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/ci/smoke-release-binary.py"
SPEC = importlib.util.spec_from_file_location("smoke_release_binary", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


def completed(
    stdout: str = "", returncode: int = 0
) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess([], returncode, stdout=stdout, stderr="")


class FakeRunner:
    def __init__(self) -> None:
        self.calls: list[tuple[tuple[str, ...], dict[str, str], Path]] = []
        self.workspace_roots_were_directories: list[bool] = []
        self.responses = {
            ("--version",): completed("ironclaw 1.0.0\n"),
            ("--help",): completed("Commands: serve run extension profile\n"),
            ("profile", "list", "--json"): completed(
                json.dumps(
                    {
                        "profiles": [
                            {"name": "local-dev"},
                            {"name": "production"},
                            {"name": "migration-dry-run"},
                        ]
                    }
                )
            ),
            ("extension", "search", "--json"): completed(
                json.dumps(
                    {
                        "payload": {
                            "extensions": [
                                {
                                    "package_ref": {"id": "first-party-package"},
                                    "runtime_kind": "first_party",
                                    "source": "host_bundled",
                                },
                                {
                                    "package_ref": {"id": "mcp-package"},
                                    "runtime_kind": "mcp_server",
                                    "source": "host_bundled",
                                },
                                {
                                    "package_ref": {"id": "wasm-package"},
                                    "runtime_kind": "wasm_tool",
                                    "source": "host_bundled",
                                },
                            ]
                        }
                    }
                )
            ),
            ("run", "--dry-run"): completed("profile: migration-dry-run\n"),
        }

    def __call__(
        self,
        _binary: Path,
        args: tuple[str, ...],
        environment: dict[str, str],
        working_directory: Path,
    ) -> subprocess.CompletedProcess[str]:
        self.calls.append((args, environment, working_directory))
        workspace_root = environment.get("IRONCLAW_REBORN_WORKSPACE_ROOT")
        self.workspace_roots_were_directories.append(
            workspace_root is not None and Path(workspace_root).is_dir()
        )
        if args == ("extension", "search", "--json"):
            database = (
                Path(environment["IRONCLAW_REBORN_HOME"])
                / "local-dev"
                / "reborn-local-dev.db"
            )
            database.parent.mkdir(parents=True)
            database.write_bytes(b"migrated libsql")
        return self.responses[args]


class ReleaseBinarySmokeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.binary = Path(self.temp_dir.name) / "ironclaw"
        self.binary.write_bytes(b"fake executable selected by the injected runner")
        self.runner = FakeRunner()

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def test_complete_smoke_matrix_is_required_and_uses_isolated_state(self) -> None:
        evidence = SMOKE.smoke_release_binary(self.binary, self.runner)

        self.assertEqual(evidence, SMOKE.REQUIRED_EVIDENCE)
        self.assertEqual(
            [args for args, _, _ in self.runner.calls],
            [
                ("--version",),
                ("--help",),
                ("profile", "list", "--json"),
                ("extension", "search", "--json"),
                ("run", "--dry-run"),
            ],
        )
        environments = [environment for _, environment, _ in self.runner.calls]
        working_directories = {
            working_directory.resolve()
            for _, _, working_directory in self.runner.calls
        }
        self.assertEqual(
            environments[-1]["IRONCLAW_REBORN_PROFILE"], "migration-dry-run"
        )
        self.assertTrue(
            all("IRONCLAW_REBORN_HOME" in environment for environment in environments)
        )
        workspace_roots = {
            environment.get("IRONCLAW_REBORN_WORKSPACE_ROOT")
            for environment in environments
        }
        self.assertEqual(len(workspace_roots), 1)
        workspace_root = workspace_roots.pop()
        self.assertIsNotNone(workspace_root)
        self.assertTrue(all(self.runner.workspace_roots_were_directories))
        workspace_path = Path(workspace_root).resolve()
        self.assertEqual(working_directories, {workspace_path})
        repository_path = ROOT.resolve()
        self.assertFalse(
            workspace_path == repository_path
            or repository_path in workspace_path.parents
        )
        self.assertTrue(
            all("DATABASE_URL" not in environment for environment in environments)
        )

    def test_isolated_environment_preserves_windows_account_for_key_acl(self) -> None:
        root = Path(self.temp_dir.name) / "windows-smoke"
        root.mkdir()
        with mock.patch.dict(
            os.environ,
            {"USERNAME": "runneradmin", "USERDOMAIN": "RUNNER"},
            clear=False,
        ):
            environment = SMOKE._isolated_environment(root)

        self.assertEqual(environment["USERNAME"], "runneradmin")
        self.assertEqual(environment["USERDOMAIN"], "RUNNER")

    def test_nonzero_shipping_command_fails(self) -> None:
        self.runner.responses[("extension", "search", "--json")] = completed(
            "partial output", returncode=7
        )

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "exited 7"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_invalid_json_reports_the_captured_stdout_prefix(self) -> None:
        with self.assertRaisesRegex(
            SMOKE.SmokeFailure, "processed file: master-key"
        ):
            SMOKE._parse_json_object(
                'processed file: master-key\n{"payload": {}}',
                "extension search --json",
            )

    def test_missing_profile_fails(self) -> None:
        self.runner.responses[("profile", "list", "--json")] = completed(
            json.dumps({"profiles": [{"name": "local-dev"}]})
        )

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "migration-dry-run"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_empty_extension_catalog_fails(self) -> None:
        self.runner.responses[("extension", "search", "--json")] = completed(
            json.dumps({"payload": {"extensions": []}})
        )

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "no bundled extensions"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_successful_catalog_without_local_libsql_state_fails(self) -> None:
        def runner_without_database(
            _binary: Path,
            args: tuple[str, ...],
            _environment: dict[str, str],
            _working_directory: Path,
        ) -> subprocess.CompletedProcess[str]:
            return self.runner.responses[args]

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "local libSQL database"):
            SMOKE.smoke_release_binary(self.binary, runner_without_database)

    def test_duplicate_dynamic_extension_ids_fail(self) -> None:
        extension = {
            "package_ref": {"id": "same-package"},
            "runtime_kind": "wasm_tool",
            "source": "host_bundled",
        }
        self.runner.responses[("extension", "search", "--json")] = completed(
            json.dumps({"payload": {"extensions": [extension, extension]}})
        )

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "duplicate"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_missing_bundled_runtime_kind_fails(self) -> None:
        extension = {
            "package_ref": {"id": "wasm-package"},
            "runtime_kind": "wasm_tool",
            "source": "host_bundled",
        }
        self.runner.responses[("extension", "search", "--json")] = completed(
            json.dumps({"payload": {"extensions": [extension]}})
        )

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "bundled runtime kinds"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_non_bundled_catalog_entry_fails(self) -> None:
        extension = {
            "package_ref": {"id": "registry-package"},
            "runtime_kind": "wasm_tool",
            "source": "registry",
        }
        self.runner.responses[("extension", "search", "--json")] = completed(
            json.dumps({"payload": {"extensions": [extension]}})
        )

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "non-bundled"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_migration_profile_must_be_exercised(self) -> None:
        self.runner.responses[("run", "--dry-run")] = completed("profile: local-dev\n")

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "migration dry-run"):
            SMOKE.smoke_release_binary(self.binary, self.runner)

    def test_missing_binary_fails_before_commands_run(self) -> None:
        self.binary.unlink()

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "does not exist"):
            SMOKE.smoke_release_binary(self.binary, self.runner)
        self.assertEqual(self.runner.calls, [])

    def test_archive_extracts_the_only_shipping_binary(self) -> None:
        archive = Path(self.temp_dir.name) / "ironclaw-target.tar.gz"
        with tarfile.open(archive, "w:gz") as package:
            member = tarfile.TarInfo("ironclaw-target/ironclaw")
            payload = b"packaged binary"
            member.size = len(payload)
            package.addfile(member, io.BytesIO(payload))

        evidence = SMOKE.smoke_release_archive(archive, "ironclaw", self.runner)

        self.assertEqual(evidence, SMOKE.REQUIRED_EVIDENCE)

    def test_archive_rejects_missing_shipping_binary(self) -> None:
        archive = Path(self.temp_dir.name) / "ironclaw-target.tar.gz"
        with tarfile.open(archive, "w:gz") as package:
            member = tarfile.TarInfo("README.md")
            payload = b"not the binary"
            member.size = len(payload)
            package.addfile(member, io.BytesIO(payload))

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "found 0"):
            SMOKE.smoke_release_archive(archive, "ironclaw", self.runner)
        self.assertEqual(self.runner.calls, [])

    def test_archive_rejects_duplicate_shipping_binaries(self) -> None:
        archive = Path(self.temp_dir.name) / "ironclaw-target.tar.gz"
        with tarfile.open(archive, "w:gz") as package:
            for path in ("first/ironclaw", "second/ironclaw"):
                member = tarfile.TarInfo(path)
                payload = b"duplicate"
                member.size = len(payload)
                package.addfile(member, io.BytesIO(payload))

        with self.assertRaisesRegex(SMOKE.SmokeFailure, "found 2"):
            SMOKE.smoke_release_archive(archive, "ironclaw", self.runner)
        self.assertEqual(self.runner.calls, [])

    @unittest.skipIf(os.name == "nt", "the fake executable uses a POSIX shebang")
    def test_archive_cli_path_executes_the_extracted_binary(self) -> None:
        archive = Path(self.temp_dir.name) / "ironclaw-target.tar.gz"
        executable = b"""#!/usr/bin/env python3
import json
import os
import sys

args = sys.argv[1:]
if args == ["--version"]:
    print("ironclaw 1.0.0")
elif args == ["--help"]:
    print("Commands: serve run extension profile")
elif args == ["profile", "list", "--json"]:
    print(json.dumps({"profiles": [
        {"name": "local-dev"},
        {"name": "production"},
        {"name": "migration-dry-run"},
    ]}))
elif args == ["extension", "search", "--json"]:
    from pathlib import Path
    database = Path(os.environ["IRONCLAW_REBORN_HOME"]) / "local-dev" / "reborn-local-dev.db"
    database.parent.mkdir(parents=True)
    database.write_bytes(b"migrated libsql")
    print(json.dumps({"payload": {"extensions": [
        {"package_ref": {"id": "first-party-package"}, "runtime_kind": "first_party", "source": "host_bundled"},
        {"package_ref": {"id": "mcp-package"}, "runtime_kind": "mcp_server", "source": "host_bundled"},
        {"package_ref": {"id": "wasm-package"}, "runtime_kind": "wasm_tool", "source": "host_bundled"},
    ]}}))
elif args == ["run", "--dry-run"]:
    print("profile: " + os.environ["IRONCLAW_REBORN_PROFILE"])
else:
    raise SystemExit(9)
"""
        with tarfile.open(archive, "w:gz") as package:
            member = tarfile.TarInfo("ironclaw-target/ironclaw")
            member.size = len(executable)
            package.addfile(member, io.BytesIO(executable))

        evidence = SMOKE.smoke_release_archive(archive, "ironclaw")

        self.assertEqual(evidence, SMOKE.REQUIRED_EVIDENCE)


if __name__ == "__main__":
    unittest.main()
