#!/usr/bin/env python3
"""Tests for live canary artifact scrubbing."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "live-canary" / "scrub-artifacts.sh"
NEARAI_MANIFEST_TEMPLATE = (
    ROOT / "scripts" / "live-canary" / "fixtures" / "nearai-runtime-manifest.toml"
)


class ScrubArtifactsTests(unittest.TestCase):
    def run_scrub(
        self,
        artifact_dir: Path,
        *,
        strict: bool,
        bundled_skills_root: Path | None = None,
        first_party_extensions_root: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["STRICT_ARTIFACT_SCRUB"] = "true" if strict else "false"
        if bundled_skills_root is not None:
            env["LIVE_CANARY_BUNDLED_SKILLS_ROOT"] = str(bundled_skills_root)
        if first_party_extensions_root is not None:
            env["LIVE_CANARY_FIRST_PARTY_EXTENSIONS_ROOT"] = str(
                first_party_extensions_root
            )
        runner_temp = artifact_dir.parent / f"{artifact_dir.name}-runner-temp"
        runner_temp.mkdir(parents=True, exist_ok=True)
        env["RUNNER_TEMP"] = str(runner_temp)
        return subprocess.run(
            [str(SCRIPT), str(artifact_dir)],
            cwd=ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )

    @staticmethod
    def bundle_hash(name: str, source_dir: Path) -> str:
        value = 0xCBF29CE484222325

        def update(data: bytes) -> None:
            nonlocal value
            for byte in data:
                value ^= byte
                value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF

        update(name.encode())
        source_files = sorted(path for path in source_dir.rglob("*") if path.is_file())
        for source_file in source_files:
            update(source_file.relative_to(source_dir).as_posix().encode())
            update(b"\0")
            update(source_file.read_bytes())
            update(b"\0")
        return f"{value:016x}"

    def write_bundled_skill_fixture(
        self,
        artifact_dir: Path,
        *,
        source_body: str,
        staged_body: str | None = None,
        marker: dict[str, object] | str | None = None,
    ) -> tuple[Path, Path]:
        trusted_root = artifact_dir.parent / "trusted-skills"
        trusted_skill = trusted_root / "local-test"
        trusted_skill.mkdir(parents=True)
        (trusted_skill / "SKILL.md").write_text(source_body, encoding="utf-8")

        staged_skill = (
            artifact_dir
            / "lane"
            / "reborn-home"
            / "case-a"
            / "local-dev"
            / "system"
            / "skills"
            / "local-test"
        )
        staged_skill.mkdir(parents=True)
        (staged_skill / "SKILL.md").write_text(
            source_body if staged_body is None else staged_body,
            encoding="utf-8",
        )
        marker_payload = marker or {
            "owner": "ironclaw_reborn_composition_bundled_skill",
            "format": 1,
            "content_hash": self.bundle_hash("local-test", trusted_skill),
        }
        marker_text = (
            marker_payload
            if isinstance(marker_payload, str)
            else json.dumps(marker_payload)
        )
        (staged_skill / ".ironclaw-reborn-bundled.json").write_text(
            marker_text,
            encoding="utf-8",
        )
        return trusted_root, staged_skill

    @staticmethod
    def write_extension_manifest_fixture(
        artifact_dir: Path,
        *,
        source_body: str,
        staged_body: str | None = None,
        extension_id: str = "gmail",
    ) -> tuple[Path, Path]:
        trusted_root = artifact_dir.parent / "trusted-extensions"
        trusted_extension = trusted_root / extension_id
        trusted_extension.mkdir(parents=True)
        (trusted_extension / "manifest.toml").write_text(source_body, encoding="utf-8")

        staged_manifest = (
            artifact_dir
            / "lane"
            / "reborn-home"
            / "case-a"
            / "local-dev"
            / "system"
            / "extensions"
            / extension_id
            / "manifest.toml"
        )
        staged_manifest.parent.mkdir(parents=True)
        staged_manifest.write_text(
            source_body if staged_body is None else staged_body,
            encoding="utf-8",
        )
        return trusted_root, staged_manifest

    def test_strict_scrub_redacts_diagnostics_and_preserves_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            product_log = root / "ironclaw-reborn-serve.stderr.log"
            browser_events = root / "browser-events.jsonl"
            product_log.write_text(
                "authorization: bearer super-secret-token\n"
                "access_token: ya29.abcdefghijklmnopqrstuvwxyz\n",
                encoding="utf-8",
            )
            browser_events.write_text('{"access_token":"browser-secret"}\n', encoding="utf-8")

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertTrue(product_log.exists())
            self.assertTrue(browser_events.exists())
            self.assertIn("<REDACTED>", product_log.read_text(encoding="utf-8"))
            self.assertIn('"access_token":"<REDACTED>"', browser_events.read_text(encoding="utf-8"))
            self.assertIn("Strict scrub redacted diagnostic artifacts", result.stdout)

    def test_strict_scrub_redacts_and_preserves_llm_trace_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            trace = root / "llm-traces" / "slack_message.json"
            trace.parent.mkdir()
            trace.write_text(
                '{"steps":[{"response":{"content":"Bearer live-secret-token"}}]}\n',
                encoding="utf-8",
            )

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertTrue(trace.exists())
            scrubbed = json.loads(trace.read_text(encoding="utf-8"))
            self.assertEqual(
                scrubbed["steps"][0]["response"]["content"],
                "Bearer <REDACTED>",
            )
            self.assertIn("Strict scrub redacted diagnostic artifacts", result.stdout)

    def test_strict_scrub_matches_relative_llm_trace_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            trace = root / "llm-traces" / "case_a.json"
            trace.parent.mkdir()
            trace.write_text(
                '{"steps":[{"response":{"content":"Bearer relative-secret"}}]}\n',
                encoding="utf-8",
            )
            runner_temp = root / "runner-temp"
            runner_temp.mkdir()
            env = os.environ.copy()
            env["STRICT_ARTIFACT_SCRUB"] = "true"
            env["RUNNER_TEMP"] = str(runner_temp)

            result = subprocess.run(
                [str(SCRIPT), "llm-traces"],
                cwd=root,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stdout)
            scrubbed = json.loads(trace.read_text(encoding="utf-8"))
            self.assertEqual(
                scrubbed["steps"][0]["response"]["content"],
                "Bearer <REDACTED>",
            )

    def test_strict_scrub_fails_if_redacted_artifact_still_matches_secret_shape(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            product_log = root / "ironclaw-reborn-serve.stderr.log"
            product_log.write_text("aws_access_key = akia1234567890abcdef\n", encoding="utf-8")

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertFalse(product_log.exists())

    def test_strict_scrub_deletes_unsafe_artifacts_and_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            unsafe = root / "raw.html"
            unsafe.write_text("api_key: secret-value\n", encoding="utf-8")

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertFalse(unsafe.exists())

    def test_strict_scrub_prunes_only_verified_bundled_skill_snapshots(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, bundled = self.write_bundled_skill_fixture(
                root,
                source_body="docker run -e NEARAI_API_KEY=dummy ironclaw-test\n",
            )
            skills_root = bundled.parent
            unmanaged = skills_root / "operator-skill"
            unmanaged.mkdir()
            (unmanaged / "SKILL.md").write_text(
                "operator-owned diagnostic context\n",
                encoding="utf-8",
            )

            result = self.run_scrub(
                root,
                strict=True,
                bundled_skills_root=trusted_root,
            )

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertFalse(bundled.exists())
            self.assertTrue((unmanaged / "SKILL.md").exists())

    def test_strict_scrub_still_scans_unmanaged_system_skill(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            unmanaged = (
                root
                / "lane"
                / "reborn-home"
                / "case-a"
                / "local-dev"
                / "system"
                / "skills"
                / "operator-skill"
                / "SKILL.md"
            )
            unmanaged.parent.mkdir(parents=True)
            unmanaged.write_text("api_key: live-secret-value\n", encoding="utf-8")

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertFalse(unmanaged.exists())

    def test_strict_scrub_rejects_malformed_bundled_marker_content(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, bundled = self.write_bundled_skill_fixture(
                root,
                source_body="api_key: live-secret-value\n",
                marker='{"owner":"ironclaw_reborn_composition_bundled_skill"',
            )

            result = self.run_scrub(
                root,
                strict=True,
                bundled_skills_root=trusted_root,
            )

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertTrue(bundled.exists())
            self.assertFalse((bundled / "SKILL.md").exists())

    def test_strict_scrub_rejects_spoofed_bundled_skill_content(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, bundled = self.write_bundled_skill_fixture(
                root,
                source_body="safe source-controlled instructions\n",
                staged_body="api_key: live-secret-value\n",
            )

            result = self.run_scrub(
                root,
                strict=True,
                bundled_skills_root=trusted_root,
            )

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertTrue(bundled.exists())
            self.assertFalse((bundled / "SKILL.md").exists())

    def test_strict_scrub_still_rejects_secret_outside_bundled_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, bundled = self.write_bundled_skill_fixture(
                root,
                source_body="docker run -e NEARAI_API_KEY=dummy ironclaw-test\n",
            )
            unsafe = root / "operator-state.txt"
            unsafe.write_text("api_key: live-secret-value\n", encoding="utf-8")

            result = self.run_scrub(
                root,
                strict=True,
                bundled_skills_root=trusted_root,
            )

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertFalse(bundled.exists())
            self.assertFalse(unsafe.exists())

    def test_strict_scrub_prunes_verified_first_party_extension_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, manifest = self.write_extension_manifest_fixture(
                root,
                source_body=(
                    'secret = true\naccess_token = "/access_token"\n'
                    'refresh_token = "/refresh_token"\n'
                ),
            )

            result = self.run_scrub(
                root,
                strict=True,
                first_party_extensions_root=trusted_root,
            )

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertFalse(manifest.exists())

    def test_strict_scrub_rejects_modified_first_party_extension_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, manifest = self.write_extension_manifest_fixture(
                root,
                source_body='secret = true\naccess_token = "/access_token"\n',
                staged_body=(
                    'secret = true\naccess_token = "/access_token"\n'
                    "api_key: live-secret-value\n"
                ),
            )

            result = self.run_scrub(
                root,
                strict=True,
                first_party_extensions_root=trusted_root,
            )

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertFalse(manifest.exists())

    def test_strict_scrub_prunes_verified_nearai_runtime_manifest(self) -> None:
        for endpoint in (
            "https://cloud-api.near.ai/mcp",
            "https://private.near.ai/mcp",
        ):
            with self.subTest(endpoint=endpoint):
                with tempfile.TemporaryDirectory() as tmpdir:
                    root = Path(tmpdir) / "artifacts"
                    root.mkdir()
                    runtime_manifest = NEARAI_MANIFEST_TEMPLATE.read_text(
                        encoding="utf-8"
                    ).replace(
                        "__LIVE_CANARY_NEARAI_MCP_SERVER__",
                        endpoint,
                    )
                    _, manifest = self.write_extension_manifest_fixture(
                        root,
                        extension_id="nearai",
                        source_body="not used for the dynamic nearai manifest\n",
                        staged_body=runtime_manifest,
                    )

                    result = self.run_scrub(root, strict=True)

                    self.assertEqual(result.returncode, 0, result.stdout)
                    self.assertFalse(manifest.exists())

    def test_non_strict_scrub_is_report_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir) / "artifacts"
            root.mkdir()
            trusted_root, bundled = self.write_bundled_skill_fixture(
                root,
                source_body="docker run -e NEARAI_API_KEY=dummy ironclaw-test\n",
            )
            extensions_root, extension_manifest = self.write_extension_manifest_fixture(
                root,
                source_body='secret = true\naccess_token = "/access_token"\n',
            )
            artifact = root / "raw.html"
            artifact.write_text("api_key: secret-value\n", encoding="utf-8")

            result = self.run_scrub(
                root,
                strict=False,
                bundled_skills_root=trusted_root,
                first_party_extensions_root=extensions_root,
            )

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertTrue((bundled / "SKILL.md").exists())
            self.assertTrue(extension_manifest.exists())
            self.assertTrue(artifact.exists())
            matches = (root / "scrub-matches.txt").read_text(encoding="utf-8")
            self.assertIn("<REDACTED>", matches)


if __name__ == "__main__":
    unittest.main()
