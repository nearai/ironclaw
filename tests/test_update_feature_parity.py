import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = REPO_ROOT / "scripts" / "update_feature_parity.py"


class UpdateFeatureParityTests(unittest.TestCase):
    def run_script(self, source: str, sha: str = "abc123", pr: str = "17") -> str:
        with tempfile.TemporaryDirectory() as tmpdir:
            parity_path = Path(tmpdir) / "FEATURE_PARITY.md"
            parity_path.write_text(source, encoding="utf-8")

            env = os.environ.copy()
            env["FEATURE_PARITY_PATH"] = str(parity_path)
            env["PARITY_SYNC_SHA"] = sha
            env["PARITY_PR_NUMBER"] = pr

            subprocess.run(
                [sys.executable, str(SCRIPT_PATH)],
                check=True,
                env=env,
            )

            return parity_path.read_text(encoding="utf-8")

    def test_inserts_auto_block_when_missing(self) -> None:
        original = textwrap.dedent(
            """\
            # IronClaw ↔ OpenClaw Feature Parity Matrix

            Existing content.
            """
        )
        updated = self.run_script(original, sha="deadbeef", pr="88")
        self.assertIn("<!-- parity:auto:start -->", updated)
        self.assertIn("<!-- parity:auto:end -->", updated)
        self.assertIn("deadbeef", updated)
        self.assertIn("#88", updated)

    def test_replaces_existing_auto_block(self) -> None:
        original = textwrap.dedent(
            """\
            # IronClaw ↔ OpenClaw Feature Parity Matrix

            <!-- parity:auto:start -->
            stale
            <!-- parity:auto:end -->

            Existing content.
            """
        )
        updated = self.run_script(original, sha="feedface", pr="19")
        self.assertIn("feedface", updated)
        self.assertIn("#19", updated)
        self.assertNotIn("stale", updated)

    def test_is_idempotent_for_same_inputs(self) -> None:
        original = textwrap.dedent(
            """\
            # IronClaw ↔ OpenClaw Feature Parity Matrix

            Existing content.
            """
        )
        once = self.run_script(original, sha="c0ffee", pr="7")
        twice = self.run_script(once, sha="c0ffee", pr="7")
        self.assertEqual(once, twice)


if __name__ == "__main__":
    unittest.main()
