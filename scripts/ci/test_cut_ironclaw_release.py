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

# `release` imports crate_tree as a side effect of exec_module above (it
# inserts scripts/ci/lib onto sys.path), so this import is safe here without
# repeating the path insertion.
import crate_tree  # noqa: E402

VERSION = "1.1.0-rc.1"
SHA = "a" * 40


def _write_candidate_manifest(
    root: Path, crate_relative_dir: str, version: str
) -> None:
    """A candidate checkout fixture: the reborn-cli crate at
    `crate_relative_dir` plus enough filler crates to clear crate_tree's
    discovery floor (a realistic-enough tree, not a one-crate stub)."""
    manifest = root / crate_relative_dir / "Cargo.toml"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(
        f'[package]\nname = "ironclaw"\nversion = "{version}"\n',
        encoding="utf-8",
    )
    for index in range(crate_tree.MIN_CRATE_DIRECTORIES + 2):
        filler = root / "crates" / f"ironclaw_filler_{index}"
        filler.mkdir(parents=True, exist_ok=True)
        (filler / "Cargo.toml").write_text(
            f'[package]\nname = "ironclaw_filler_{index}"\n', encoding="utf-8"
        )


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
            created: list[tuple[str, str]] = []
            arguments = {
                "requested_version": VERSION,
                "requested_sha": SHA,
                "manifest_version": VERSION,
                "checked_out_sha": SHA,
                "get_tag_target": lambda _tag: None,
                "create_tag": lambda tag, sha, calls=created: calls.append((tag, sha)),
            }
            arguments[field] = value
            with (
                self.subTest(field=field),
                self.assertRaisesRegex(release.ReleaseTagError, message),
            ):
                release.ensure_release_tag(**arguments)
            self.assertEqual(created, [])

    def test_rejects_ambiguous_release_identity(self) -> None:
        cases = (
            {"requested_version": "v1.1.0", "requested_sha": SHA},
            {"requested_version": "01.1.0", "requested_sha": SHA},
            {"requested_version": "1.01.0", "requested_sha": SHA},
            {"requested_version": "1.1.01", "requested_sha": SHA},
            {"requested_version": "1.1.0-01", "requested_sha": SHA},
            {"requested_version": "1.1.0-rc..1", "requested_sha": SHA},
            # Docker release tags cannot contain Cargo build metadata.
            {"requested_version": "1.1.0+build.7", "requested_sha": SHA},
            {"requested_version": VERSION, "requested_sha": "abc123"},
        )
        for overrides in cases:
            created: list[tuple[str, str]] = []
            arguments = {
                "requested_version": VERSION,
                "requested_sha": SHA,
                "manifest_version": VERSION,
                "checked_out_sha": SHA,
                "get_tag_target": lambda _tag: None,
                "create_tag": lambda tag, sha, calls=created: calls.append((tag, sha)),
            }
            arguments.update(overrides)
            if "requested_version" in overrides:
                arguments["manifest_version"] = overrides["requested_version"]
            with (
                self.subTest(overrides=overrides),
                self.assertRaisesRegex(release.ReleaseTagError, "invalid|commit_sha"),
            ):
                release.ensure_release_tag(**arguments)
            self.assertEqual(created, [])

    def test_accepts_valid_semver_prerelease_identifiers(self) -> None:
        for version in ("1.1.0-0", "1.1.0-rc.1", "1.1.0-01a"):
            with self.subTest(version=version):
                created: list[tuple[str, str]] = []
                release.ensure_release_tag(
                    requested_version=version,
                    requested_sha=SHA,
                    manifest_version=version,
                    checked_out_sha=SHA,
                    get_tag_target=lambda _tag: None,
                    create_tag=lambda tag, sha, calls=created: calls.append((tag, sha)),
                )
                self.assertEqual(created, [(f"ironclaw-v{version}", SHA)])

    def test_annotated_tag_is_resolved_to_its_commit(self) -> None:
        tag_object_sha = "b" * 40
        responses = (
            mock.Mock(
                returncode=0,
                stdout=(f'{{"object":{{"type":"tag","sha":"{tag_object_sha}"}}}}'),
                stderr="",
            ),
            mock.Mock(
                returncode=0,
                stdout=(f'{{"object":{{"type":"commit","sha":"{SHA}"}}}}'),
                stderr="",
            ),
        )
        with mock.patch.object(release.subprocess, "run", side_effect=responses) as run:
            target = release.GitHubTags("nearai/ironclaw").get_target("ironclaw-v1.0.0")

        self.assertEqual(target, SHA)
        self.assertIn(f"git/tags/{tag_object_sha}", run.call_args_list[1].args[0][2])

    def test_accepts_exact_annotated_tag_resolution_limit(self) -> None:
        responses = [
            mock.Mock(
                returncode=0,
                stdout=f'{{"object":{{"type":"tag","sha":"{index:x}{"0" * 39}"}}}}',
                stderr="",
            )
            for index in range(1, release.MAX_ANNOTATED_TAG_DEPTH + 1)
        ]
        responses.append(
            mock.Mock(
                returncode=0,
                stdout=f'{{"object":{{"type":"commit","sha":"{SHA}"}}}}',
                stderr="",
            )
        )

        with mock.patch.object(release.subprocess, "run", side_effect=responses):
            target = release.GitHubTags("nearai/ironclaw").get_target("ironclaw-v1.0.0")

        self.assertEqual(target, SHA)

    def test_rejects_annotated_tag_beyond_resolution_limit(self) -> None:
        responses = [
            mock.Mock(
                returncode=0,
                stdout=f'{{"object":{{"type":"tag","sha":"{index:x}{"0" * 39}"}}}}',
                stderr="",
            )
            for index in range(1, release.MAX_ANNOTATED_TAG_DEPTH + 2)
        ]

        with (
            mock.patch.object(release.subprocess, "run", side_effect=responses) as run,
            self.assertRaisesRegex(release.ReleaseTagError, "resolution depth"),
        ):
            release.GitHubTags("nearai/ironclaw").get_target("ironclaw-v1.0.0")

        self.assertEqual(run.call_count, release.MAX_ANNOTATED_TAG_DEPTH + 1)

    def test_release_tooling_is_pinned_to_default_branch_dispatch(self) -> None:
        workflow = (ROOT / ".github/workflows/cut-ironclaw-release.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "if: github.ref == format('refs/heads/{0}', "
            "github.event.repository.default_branch)",
            workflow,
        )
        self.assertIn("ref: ${{ github.sha }}", workflow)
        self.assertIn(
            "python3 release-tools/scripts/ci/cut_ironclaw_release.py", workflow
        )
        self.assertNotIn("candidate/scripts/ci/cut_ironclaw_release.py", workflow)

    def test_candidate_metadata_comes_from_supplied_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root, "crates/ironclaw_cli", VERSION
            )
            self.assertEqual(release._manifest_version(candidate_root), VERSION)

            completed = mock.Mock(stdout=f"{SHA}\n")
            with mock.patch.object(
                release.subprocess, "run", return_value=completed
            ) as run:
                self.assertEqual(release._checked_out_sha(candidate_root), SHA)
            self.assertEqual(run.call_args.kwargs["cwd"], candidate_root)

    def test_candidate_manifest_resolves_through_crate_inventory_when_nested(
        self,
    ) -> None:
        """WS10: the candidate's ironclaw_cli manifest is found by
        crate NAME even after the target-architecture family move
        (crates/<family>/ironclaw_cli, PROPOSAL §5) — this is exactly
        the shape a release cut against a moved candidate commit hits."""
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root, "crates/substrates/ironclaw_cli", VERSION
            )
            self.assertEqual(release._manifest_version(candidate_root), VERSION)

    def test_candidate_manifest_resolution_fails_closed_when_crate_missing(
        self,
    ) -> None:
        """A candidate checkout that cannot resolve ironclaw_cli must
        refuse loudly, not silently read `manifest_version` as empty/wrong —
        the WS10 failure mode this whole module guards against."""
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            for index in range(crate_tree.MIN_CRATE_DIRECTORIES + 2):
                filler = candidate_root / "crates" / f"ironclaw_filler_{index}"
                filler.mkdir(parents=True)
                (filler / "Cargo.toml").write_text(
                    f'[package]\nname = "ironclaw_filler_{index}"\n',
                    encoding="utf-8",
                )
            with self.assertRaisesRegex(
                release.ReleaseTagError,
                "cannot resolve the ironclaw_cli crate",
            ):
                release._manifest_version(candidate_root)


if __name__ == "__main__":
    unittest.main()
