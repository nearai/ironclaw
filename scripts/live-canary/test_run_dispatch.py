#!/usr/bin/env python3
"""Unit tests for scripts/live-canary/run.sh dispatch behavior.

Run with::

    python3 -m pytest scripts/live-canary/test_run_dispatch.py -v

Or directly::

    python3 scripts/live-canary/test_run_dispatch.py
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RUN_SH = ROOT / "scripts" / "live-canary" / "run.sh"


class RunShDispatchTests(unittest.TestCase):
    PERSONA_CREDENTIAL_ENV_NAMES = (
        "LIVE_CANARY_GITHUB_TOKEN",
        "LIVE_CANARY_GOOGLE_OAUTH_TOKEN",
        "LIVE_CANARY_SLACK_BOT_TOKEN",
        "LIVE_CANARY_TELEGRAM_BOT_TOKEN",
        "LIVE_CANARY_COMPOSIO_API_KEY",
    )

    def run_dispatch(
        self, *, cases: str
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        with tempfile.TemporaryDirectory() as tmpdir:
            env = {
                **os.environ,
                "ARTIFACT_ROOT": tmpdir,
                "CASES": cases,
                "LANE": "reborn-webui-v2-live-qa",
                "PLAYWRIGHT_INSTALL": "skip",
                "PROVIDER": "reborn-webui-v2",
                "PYTHON_BIN": "echo",
                "TIMESTAMP": "dispatch-test",
            }
            result = subprocess.run(
                [str(RUN_SH)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                check=True,
            )
            summary_path = (
                Path(tmpdir)
                / "reborn-webui-v2-live-qa"
                / "reborn-webui-v2"
                / "dispatch-test"
                / "env-summary.txt"
            )
            return result, summary_path.read_text(encoding="utf-8")

    def run_persona_dispatch(self, **credential_env: str) -> str:
        with tempfile.TemporaryDirectory() as tmpdir:
            fake_bin = Path(tmpdir) / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            fake_cargo.chmod(0o755)

            artifact_root = Path(tmpdir) / "artifacts"
            env = {
                **os.environ,
                "ARTIFACT_ROOT": str(artifact_root),
                "LANE": "persona-rotating",
                "PATH": f"{fake_bin}{os.pathsep}{os.environ['PATH']}",
                "PROVIDER": "anthropic",
                "SCENARIO": "developer_full_workflow",
                "TIMESTAMP": "dispatch-test",
            }
            for env_name in self.PERSONA_CREDENTIAL_ENV_NAMES:
                env.pop(env_name, None)
            env.update(credential_env)

            subprocess.run(
                [str(RUN_SH)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                check=True,
            )
            summary_path = (
                artifact_root
                / "persona-rotating"
                / "anthropic"
                / "dispatch-test"
                / "env-summary.txt"
            )
            return summary_path.read_text(encoding="utf-8")

    def test_reborn_all_cases_dispatches_non_telegram_qa_flag(self):
        result, summary = self.run_dispatch(cases="all")

        self.assertIn("scripts/reborn_webui_v2_live_qa/run_live_qa.py", result.stdout)
        self.assertIn("--non-telegram-qa-cases", result.stdout)
        self.assertNotIn("--all-cases", result.stdout)
        self.assertNotIn("--case all", result.stdout)
        self.assertNotIn("persona_credentials_configured=", summary)
        self.assertNotIn("persona_credentials_fallback=", summary)

    def test_reborn_specific_cases_dispatch_as_repeated_case_flags(self):
        result, _summary = self.run_dispatch(
            cases="qa_3b_endpoint_status_live_chat, qa_8b_hn_keyword_live_chat"
        )

        self.assertIn("--case qa_3b_endpoint_status_live_chat", result.stdout)
        self.assertIn("--case qa_8b_hn_keyword_live_chat", result.stdout)
        self.assertNotIn("--all-cases", result.stdout)

    def test_persona_summary_reports_configured_and_fallback_credentials_without_secrets(
        self,
    ):
        summary = self.run_persona_dispatch(
            LIVE_CANARY_GITHUB_TOKEN="secret-github-value",
            LIVE_CANARY_SLACK_BOT_TOKEN="secret-slack-value",
        )
        summary_lines = summary.splitlines()

        self.assertIn(
            "persona_credentials_configured=github,slack",
            summary_lines,
        )
        self.assertIn(
            "persona_credentials_fallback=google,telegram,composio",
            summary_lines,
        )
        self.assertNotIn("secret-github-value", summary)
        self.assertNotIn("secret-slack-value", summary)

    def test_persona_summary_treats_empty_credentials_as_fallback(self):
        summary = self.run_persona_dispatch(LIVE_CANARY_GOOGLE_OAUTH_TOKEN="")
        summary_lines = summary.splitlines()

        self.assertIn("persona_credentials_configured=", summary_lines)
        self.assertIn(
            "persona_credentials_fallback=github,google,slack,telegram,composio",
            summary_lines,
        )


if __name__ == "__main__":
    unittest.main()
