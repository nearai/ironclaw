"""Behavioral tests for immutable Ironclaw release tag creation."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/ci/cut_ironclaw_release.py"
SPEC = importlib.util.spec_from_file_location("cut_ironclaw_release", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
release = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = release
SPEC.loader.exec_module(release)

VERSION = "1.1.0-rc.1"
SHA = "a" * 40


class ReleaseTagTests(unittest.TestCase):
    def run_release(
        self, *, targets: list[str | None], create_error: bool = False
    ) -> tuple[str, list[tuple[str, str]]]:
        lookups = iter(targets)
        created: list[tuple[str, str]] = []

        def create(tag: str, commit_sha: str) -> None:
            created.append((tag, commit_sha))
            if create_error:
                raise release.ReleaseTagError("create failed")

        message = release.ensure_release_tag(
            requested_version=VERSION,
            requested_sha=SHA,
            manifest_version=VERSION,
            checked_out_sha=SHA,
            get_tag_target=lambda _tag: next(lookups),
            create_tag=create,
        )
        return message, created

    def test_creates_tag_for_exact_version_and_commit(self) -> None:
        message, created = self.run_release(targets=[None])
        self.assertEqual(created, [(f"ironclaw-v{VERSION}", SHA)])
        self.assertIn("created", message)

    def test_existing_tag_is_idempotent_only_at_approved_commit(self) -> None:
        message, created = self.run_release(targets=[SHA])
        self.assertEqual(created, [])
        self.assertIn("already points", message)

        with self.assertRaisesRegex(release.ReleaseTagError, "already points"):
            self.run_release(targets=["b" * 40])

    def test_create_race_is_safe_only_at_approved_commit(self) -> None:
        message, created = self.run_release(targets=[None, SHA], create_error=True)
        self.assertEqual(created, [(f"ironclaw-v{VERSION}", SHA)])
        self.assertIn("concurrently created", message)

        with self.assertRaisesRegex(release.ReleaseTagError, "create failed"):
            self.run_release(targets=[None, "b" * 40], create_error=True)

    def test_rejects_mismatched_checkout_and_manifest(self) -> None:
        for field, value, message in (
            ("checked_out_sha", "b" * 40, "checked out"),
            ("manifest_version", "1.1.0-rc.2", "declares version"),
        ):
            arguments = {
                "requested_version": VERSION,
                "requested_sha": SHA,
                "manifest_version": VERSION,
                "checked_out_sha": SHA,
                "get_tag_target": lambda _tag: None,
                "create_tag": lambda _tag, _sha: None,
            }
            arguments[field] = value
            with (
                self.subTest(field=field),
                self.assertRaisesRegex(release.ReleaseTagError, message),
            ):
                release.ensure_release_tag(**arguments)

    def test_rejects_ambiguous_release_identity(self) -> None:
        cases = (
            {"requested_version": "v1.1.0", "requested_sha": SHA},
            {"requested_version": VERSION, "requested_sha": "abc123"},
        )
        for overrides in cases:
            arguments = {
                "requested_version": VERSION,
                "requested_sha": SHA,
                "manifest_version": VERSION,
                "checked_out_sha": SHA,
                "get_tag_target": lambda _tag: None,
                "create_tag": lambda _tag, _sha: None,
            }
            arguments.update(overrides)
            with (
                self.subTest(overrides=overrides),
                self.assertRaises(release.ReleaseTagError),
            ):
                release.ensure_release_tag(**arguments)

    def test_candidate_metadata_comes_from_supplied_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            manifest = candidate_root / "crates/ironclaw_reborn_cli/Cargo.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text(
                f'[package]\nname = "ironclaw"\nversion = "{VERSION}"\n',
                encoding="utf-8",
            )
            self.assertEqual(release._manifest_version(candidate_root), VERSION)

            completed = mock.Mock(stdout=f"{SHA}\n")
            with mock.patch.object(
                release.subprocess, "run", return_value=completed
            ) as run:
                self.assertEqual(release._checked_out_sha(candidate_root), SHA)
            self.assertEqual(run.call_args.kwargs["cwd"], candidate_root)


if __name__ == "__main__":
    unittest.main()
