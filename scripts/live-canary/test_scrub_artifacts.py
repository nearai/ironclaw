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


class ScrubArtifactsTests(unittest.TestCase):
    def run_scrub(self, artifact_dir: Path, *, strict: bool) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["STRICT_ARTIFACT_SCRUB"] = "true" if strict else "false"
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

    def test_strict_scrub_prunes_only_managed_bundled_skill_snapshots(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            skills_root = (
                root
                / "lane"
                / "reborn-home"
                / "case-a"
                / "local-dev"
                / "system"
                / "skills"
            )
            bundled = skills_root / "local-test"
            bundled.mkdir(parents=True)
            (bundled / ".ironclaw-reborn-bundled.json").write_text(
                json.dumps(
                    {
                        "owner": "ironclaw_reborn_composition_bundled_skill",
                        "format": 1,
                        "content_hash": "fixture-hash",
                    }
                ),
                encoding="utf-8",
            )
            (bundled / "SKILL.md").write_text(
                "docker run -e NEARAI_API_KEY=dummy ironclaw-test\n",
                encoding="utf-8",
            )
            unmanaged = skills_root / "operator-skill"
            unmanaged.mkdir()
            (unmanaged / "SKILL.md").write_text(
                "operator-owned diagnostic context\n",
                encoding="utf-8",
            )

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertFalse(bundled.exists())
            self.assertTrue((unmanaged / "SKILL.md").exists())

    def test_strict_scrub_still_rejects_secret_outside_bundled_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            bundled = (
                root
                / "lane"
                / "reborn-home"
                / "case-a"
                / "local-dev"
                / "system"
                / "skills"
                / "local-test"
            )
            bundled.mkdir(parents=True)
            (bundled / ".ironclaw-reborn-bundled.json").write_text(
                '{"owner": "ironclaw_reborn_composition_bundled_skill"}',
                encoding="utf-8",
            )
            (bundled / "SKILL.md").write_text(
                "NEARAI_API_KEY=dummy\n",
                encoding="utf-8",
            )
            unsafe = root / "operator-state.txt"
            unsafe.write_text("api_key: live-secret-value\n", encoding="utf-8")

            result = self.run_scrub(root, strict=True)

            self.assertEqual(result.returncode, 1, result.stdout)
            self.assertFalse(bundled.exists())
            self.assertFalse(unsafe.exists())

    def test_non_strict_scrub_is_report_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            artifact = root / "raw.html"
            artifact.write_text("api_key: secret-value\n", encoding="utf-8")

            result = self.run_scrub(root, strict=False)

            self.assertEqual(result.returncode, 0, result.stdout)
            self.assertTrue(artifact.exists())
            matches = (root / "scrub-matches.txt").read_text(encoding="utf-8")
            self.assertIn("<REDACTED>", matches)


if __name__ == "__main__":
    unittest.main()
