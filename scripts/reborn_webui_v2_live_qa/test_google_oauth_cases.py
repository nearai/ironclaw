import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


_SPEC = importlib.util.spec_from_file_location(
    "google_oauth_cases",
    Path(__file__).resolve().parents[2] / ".github/scripts/google_oauth_cases.py",
)
google_cases = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = google_cases
_SPEC.loader.exec_module(google_cases)


GOOGLE_CASES = "qa_2a_gmail_connect,qa_4e_github_release_email_delivery"


class GoogleOauthCasesTests(unittest.TestCase):
    def test_filter_and_mint_decisions_share_the_same_case_set(self):
        configured = set(google_cases.parse_cases(GOOGLE_CASES))
        selected = google_cases.parse_cases(
            "qa_2a_gmail_connect,qa_4b_github_connect,qa_4e_github_release_email_delivery"
        )
        self.assertTrue(google_cases.requires_google(selected, configured))
        self.assertEqual(
            google_cases.retain_without_google(selected, configured),
            ["qa_4b_github_connect"],
        )

    def test_cli_preserves_non_google_cases_after_mint_failure(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            github_env = Path(tmpdir) / "env"
            github_output = Path(tmpdir) / "output"
            argv = [
                "google_oauth_cases.py",
                "--mode",
                "suppress",
                "--cases",
                "qa_2a_gmail_connect,qa_4b_github_connect",
                "--google-cases",
                GOOGLE_CASES,
                "--status",
                "invalid_grant",
                "--github-env",
                str(github_env),
                "--github-output",
                str(github_output),
            ]
            with patch.object(sys, "argv", argv):
                self.assertEqual(google_cases.main(), 0)
            self.assertEqual(
                github_env.read_text(encoding="utf-8"), "CASES=qa_4b_github_connect\n"
            )
            self.assertEqual(github_output.read_text(encoding="utf-8"), "skip_shard=0\n")

    def test_cli_skips_an_all_google_shard_after_mint_failure(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            github_output = Path(tmpdir) / "output"
            argv = [
                "google_oauth_cases.py",
                "--mode",
                "suppress",
                "--cases",
                "qa_2a_gmail_connect,qa_4e_github_release_email_delivery",
                "--google-cases",
                GOOGLE_CASES,
                "--status",
                "network:URLError",
                "--github-output",
                str(github_output),
            ]
            with patch.object(sys, "argv", argv):
                self.assertEqual(google_cases.main(), 0)
            self.assertEqual(github_output.read_text(encoding="utf-8"), "skip_shard=1\n")


if __name__ == "__main__":
    unittest.main()
