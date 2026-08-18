"""Behavioral tests for immutable Ironclaw release tag creation."""

from __future__ import annotations

import importlib.util
import json
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


def _write_candidate_manifest(
    root: Path,
    crate_relative_dir: str,
    version: str,
    *,
    package_name: str = "ironclaw",
) -> None:
    """Add a package to a minimal candidate Cargo workspace fixture."""
    manifest = root / crate_relative_dir / "Cargo.toml"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(
        f'[package]\nname = "{package_name}"\nversion = "{version}"\n'
        'edition = "2021"\n',
        encoding="utf-8",
    )
    (manifest.parent / "src").mkdir(exist_ok=True)
    (manifest.parent / "src/lib.rs").write_text("", encoding="utf-8")

    members = sorted(
        manifest.parent.relative_to(root).as_posix()
        for manifest in (root / "crates").rglob("Cargo.toml")
    )
    rendered_members = ",\n".join(f'  "{member}"' for member in members)
    (root / "Cargo.toml").write_text(
        f'[workspace]\nresolver = "2"\nmembers = [\n{rendered_members}\n]\n',
        encoding="utf-8",
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

        code_style = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "python3 scripts/ci/test_cut_ironclaw_release.py", code_style
        )

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
        """WS10: the shipping package is found after the target-architecture
        family move (`crates/<family>/ironclaw_cli`, PROPOSAL §5)."""
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root, "crates/substrates/ironclaw_cli", VERSION
            )
            self.assertEqual(release._manifest_version(candidate_root), VERSION)

    def test_candidate_manifest_resolves_historical_reborn_cli_layout(self) -> None:
        """Release tooling on main must validate supported release branches
        without requiring them to adopt main's current crate directory layout."""
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root, "crates/ironclaw_reborn_cli", VERSION
            )
            self.assertEqual(release._manifest_version(candidate_root), VERSION)

    def test_candidate_manifest_resolution_uses_package_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root,
                "crates/ironclaw_cli",
                VERSION,
                package_name="not-the-shipping-package",
            )
            with self.assertRaisesRegex(
                release.ReleaseTagError,
                "exactly one candidate workspace package named 'ironclaw', found 0",
            ):
                release._manifest_version(candidate_root)

    def test_candidate_manifest_resolution_rejects_ambiguous_package(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root, "crates/ironclaw_reborn_cli", VERSION
            )
            _write_candidate_manifest(
                candidate_root, "crates/app/ironclaw_cli", VERSION
            )
            with self.assertRaisesRegex(
                release.ReleaseTagError,
                "cannot inventory Cargo workspace.*two packages named `ironclaw`",
            ):
                release._manifest_version(candidate_root)

    def test_candidate_manifest_resolution_rejects_non_workspace_package(
        self,
    ) -> None:
        """A filesystem crate excluded from Cargo's workspace cannot identify
        the package that cargo-dist will release."""
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root,
                "crates/workspace_member",
                VERSION,
                package_name="not-the-shipping-package",
            )
            unlisted = candidate_root / "crates/unlisted_ironclaw"
            (unlisted / "src").mkdir(parents=True)
            (unlisted / "Cargo.toml").write_text(
                f'[package]\nname = "ironclaw"\nversion = "{VERSION}"\n'
                'edition = "2021"\n',
                encoding="utf-8",
            )
            (unlisted / "src/lib.rs").write_text("", encoding="utf-8")

            with self.assertRaisesRegex(
                release.ReleaseTagError,
                "exactly one candidate workspace package named 'ironclaw', found 0",
            ):
                release._manifest_version(candidate_root)

    def test_candidate_manifest_resolution_rejects_malformed_toml(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            manifest = candidate_root / "crates/ironclaw_cli/Cargo.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text('[package\nname = "ironclaw"\n', encoding="utf-8")
            metadata = {
                "workspace_members": ["ironclaw-id"],
                "packages": [
                    {
                        "id": "ironclaw-id",
                        "manifest_path": str(manifest),
                    }
                ],
            }
            completed = mock.Mock(
                returncode=0,
                stdout=json.dumps(metadata),
                stderr="",
            )

            with (
                mock.patch.object(release.subprocess, "run", return_value=completed),
                self.assertRaisesRegex(
                    release.ReleaseTagError,
                    "cannot read candidate manifest.*Expected ']'",
                ),
            ):
                release._manifest_version(candidate_root)

    def test_candidate_manifest_resolution_fails_closed_when_crate_missing(
        self,
    ) -> None:
        """A candidate without the shipping package must refuse loudly, not
        silently read `manifest_version` as empty or wrong."""
        with tempfile.TemporaryDirectory() as directory:
            candidate_root = Path(directory)
            _write_candidate_manifest(
                candidate_root,
                "crates/not_ironclaw",
                VERSION,
                package_name="not-the-shipping-package",
            )
            with self.assertRaisesRegex(
                release.ReleaseTagError,
                "exactly one candidate workspace package named 'ironclaw', found 0",
            ):
                release._manifest_version(candidate_root)


class StableChangelogGateTests(unittest.TestCase):
    """A stable cut requires the candidate's docs/changelog.mdx entry; rc
    cuts stay exempt so the freeze/blocker flow is unimpeded."""

    STABLE = "1.2.0"

    def write_changelog(self, root: Path, body: str) -> None:
        changelog = root / "docs" / "changelog.mdx"
        changelog.parent.mkdir(parents=True, exist_ok=True)
        changelog.write_text(body, encoding="utf-8")

    def test_stable_cut_without_changelog_entry_refuses(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_changelog(root, '<Update description="v1.1.0">old</Update>\n')
            with self.assertRaisesRegex(
                release.ReleaseTagError, "no entry for v1.2.0"
            ):
                release.ensure_stable_changelog_entry(root, self.STABLE)

    def test_stable_cut_without_changelog_file_refuses(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(
                release.ReleaseTagError, "cannot be read"
            ):
                release.ensure_stable_changelog_entry(Path(directory), self.STABLE)

    def test_stable_cut_with_entry_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_changelog(
                root, '<Update label="2026-08-10" description="v1.2.0">…</Update>\n'
            )
            release.ensure_stable_changelog_entry(root, self.STABLE)

    def test_rc_labeled_entry_does_not_satisfy_the_stable_gate(self) -> None:
        """`description="v1.2.0-rc.1"` contains `v1.2.0` as a substring; the
        exact-attribute probe must still refuse the stable cut."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_changelog(
                root,
                '<Update label="2026-08-10" description="v1.2.0-rc.1">…</Update>\n',
            )
            with self.assertRaisesRegex(
                release.ReleaseTagError, "no entry for v1.2.0"
            ):
                release.ensure_stable_changelog_entry(root, self.STABLE)

    def test_prose_mention_does_not_satisfy_the_stable_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_changelog(
                root, "The v1.2.0 release notes are coming soon.\n"
            )
            with self.assertRaisesRegex(
                release.ReleaseTagError, "no entry for v1.2.0"
            ):
                release.ensure_stable_changelog_entry(root, self.STABLE)

    def test_lookalike_attribute_does_not_satisfy_the_stable_gate(self) -> None:
        """Only a real `<Update>` tag counts — not `data-description=` or the
        same attribute on another element."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_changelog(
                root,
                '<Update data-description="v1.2.0">…</Update>\n'
                '<Card description="v1.2.0">…</Card>\n',
            )
            with self.assertRaisesRegex(
                release.ReleaseTagError, "no entry for v1.2.0"
            ):
                release.ensure_stable_changelog_entry(root, self.STABLE)

    def test_main_gates_the_cut_before_any_tag_operation(self) -> None:
        """The gate must run inside main(), against the candidate root, before
        GitHubTags is even constructed — not only as a callable helper."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_changelog(root, '<Update description="v1.1.0">…</Update>\n')
            argv = [
                "cut_ironclaw_release.py",
                "--version", self.STABLE,
                "--commit-sha", "0" * 40,
                "--candidate-root", str(root),
                "--repository", "nearai/ironclaw",
            ]
            with mock.patch("sys.argv", argv), mock.patch.object(
                release, "GitHubTags"
            ) as tags:
                with self.assertRaisesRegex(
                    release.ReleaseTagError, "no entry for v1.2.0"
                ):
                    release.main()
            tags.assert_not_called()

    def test_prerelease_cut_is_exempt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            # No docs/ tree at all: an rc cut must not require one.
            release.ensure_stable_changelog_entry(Path(directory), "1.2.0-rc.1")

    def test_malformed_version_is_left_to_the_canonical_validator(self) -> None:
        """`ensure_release_tag` owns the invalid-version message; the
        changelog gate must not preempt it with a confusing changelog error."""
        with tempfile.TemporaryDirectory() as directory:
            release.ensure_stable_changelog_entry(Path(directory), "not-a-version")


if __name__ == "__main__":
    unittest.main()
