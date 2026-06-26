#!/usr/bin/env python3
"""Unit tests for the Reborn WebUI v2 live QA runner helpers.

Run with::

    python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
"""

from __future__ import annotations

import argparse
import asyncio
import importlib.util
import json
import os
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import run_live_qa


class RebornWebUiV2LiveQaRunnerTests(unittest.TestCase):
    def test_generated_google_seed_creates_refreshable_product_auth_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN": "fake-access-token",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "fake-refresh-token",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "fake-client-id",
                "REBORN_WEBUI_V2_LIVE_QA_SKIP_GOOGLE_REFRESH_PROBE": "1",
            }
            with patch.dict(os.environ, env, clear=False):
                seed = run_live_qa._seed_generated_google_product_auth_if_configured(
                    home,
                    "qa-user",
                )
                preflight = run_live_qa._google_product_auth_preflight(
                    home,
                    "qa-user",
                    {"IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "fake-client-id"},
                )

            self.assertTrue(seed["seeded"])
            self.assertTrue(preflight["configured_ready"])
            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)
            account = preflight["accounts"][0]
            self.assertTrue(account["access_secret_expired"])
            self.assertTrue(account["refresh_secret_present"])
            self.assertEqual(account["refresh_probe"]["reason"], "disabled_by_env")

            db_path = home / "local-dev" / "reborn-local-dev.db"
            master_key = (
                home / "local-dev" / ".reborn-local-dev-secrets-master-key"
            ).read_text(encoding="utf-8")
            with sqlite3.connect(db_path) as db:
                rows = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE '%/secrets/google-oauth-refresh-%'"
                ).fetchall()
            self.assertEqual(len(rows), 1)
            stored = json.loads(rows[0][0])
            self.assertEqual(
                run_live_qa._decrypt_filesystem_secret(master_key, stored),
                "fake-refresh-token",
            )

    def test_generated_github_seed_creates_manual_token_product_auth_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GITHUB_TOKEN": "fake-github-token",
            }
            with patch.dict(os.environ, env, clear=False):
                seed = run_live_qa._seed_generated_github_product_auth_if_configured(
                    home,
                    "qa-user",
                )
                preflight = run_live_qa._github_auth_preflight(
                    home,
                    {},
                    requires_github_auth=True,
                )

            self.assertTrue(seed["seeded"])
            self.assertEqual(seed["token_env_source"], "AUTH_LIVE_GITHUB_TOKEN")
            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)

            db_path = home / "local-dev" / "reborn-local-dev.db"
            master_key = (
                home / "local-dev" / ".reborn-local-dev-secrets-master-key"
            ).read_text(encoding="utf-8")
            with sqlite3.connect(db_path) as db:
                account_row = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE '%product-auth/callback/accounts/%.json'"
                ).fetchone()
            self.assertIsNotNone(account_row)
            account = json.loads(account_row[0])
            self.assertEqual(account["provider"], "github")
            self.assertEqual(account["status"], "configured")
            expected_handle = (
                f"product-auth-manual-{seed['account_id']}-{seed['account_id']}"
            )
            self.assertEqual(account["access_secret"], expected_handle)

            with sqlite3.connect(db_path) as db:
                secret_row = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE ?",
                    (f"%/{account['access_secret']}.json",),
                ).fetchone()
            self.assertIsNotNone(secret_row)
            stored = json.loads(secret_row[0])
            self.assertEqual(
                run_live_qa._decrypt_filesystem_secret(master_key, stored),
                "fake-github-token",
            )

    def test_prepare_reborn_home_gates_missing_slack_without_raising(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            missing_source = root / "missing-source-home"
            args = argparse.Namespace(
                output_dir=root / "out",
                reborn_home=missing_source,
                require_slack_live=False,
            )
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
            }
            for name in (
                "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
                "IRONCLAW_REBORN_SLACK_SIGNING_SECRET_PATH",
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN_PATH",
            ):
                env[name] = ""

            with patch.dict(os.environ, env, clear=False):
                prepared = run_live_qa.prepare_reborn_home(
                    args,
                    ["qa_3a_slack_connect"],
                )

            slack = prepared.preflight["slack"]
            self.assertTrue(slack["enabled_in_config"])
            self.assertTrue(slack["requires_slack"])
            self.assertFalse(slack["env_present"])
            self.assertEqual(slack["auth_test"]["error"], "Slack env unavailable")
            self.assertEqual(slack["config_installation_id"], "local-dev-installation")
            self.assertEqual(slack["config_team_id"], "local-dev-team")
            self.assertEqual(slack["config_api_app_id"], "local-dev-app-id")

    def test_generated_slack_home_ignores_empty_ci_vars(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID": "",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID": "",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID": "",
                "IRONCLAW_REBORN_SLACK_SIGNING_SECRET": "",
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN": "",
            }

            with patch.dict(os.environ, env, clear=True):
                run_live_qa.create_generated_reborn_home(home, include_slack=True)

            config = (home / "config.toml").read_text(encoding="utf-8")
            self.assertIn('installation_id = "local-dev-installation"', config)
            self.assertIn('team_id = "local-dev-team"', config)
            self.assertIn('api_app_id = "local-dev-app-id"', config)
            self.assertNotIn('installation_id = ""', config)
            self.assertNotIn('team_id = ""', config)
            self.assertNotIn('api_app_id = ""', config)

    def test_default_suite_includes_github_connect_after_generated_auth_seed(self):
        self.assertTrue(run_live_qa.CASES["qa_4b_github_connect"].default_enabled)
        self.assertTrue(run_live_qa.CASES["qa_4b_github_connect"].requires_github_auth)
        self.assertIn("qa_4b_github_connect", run_live_qa.CASES)
        default_cases = [
            name
            for name, spec in run_live_qa.CASES.items()
            if spec.default_enabled
        ]
        self.assertIn("qa_4b_github_connect", default_cases)
        self.assertIn("webui_mobile_live_llm_chat", default_cases)
        self.assertEqual(
            run_live_qa.CASES["webui_mobile_live_llm_chat"].qa_matrix_test_ids,
            ["REBCLI-065-TC-20", "REBCLI-065-TC-21"],
        )

    def test_case_manifest_distinguishes_targeted_from_placeholder_gates(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            matrix_path = output_dir / "ironclaw_reborn_cli_qa_matrix.xlsx"
            with patch.dict(
                os.environ,
                {"REBORN_WEBUI_V2_LIVE_QA_MATRIX_PATH": str(matrix_path)},
                clear=False,
            ):
                manifest_path = run_live_qa.write_case_manifest(
                    output_dir,
                    [
                        "qa_2d_calendar_prep_live_chat",
                        "qa_2f_calendar_prep_email_delivery",
                    ],
                )
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

        self.assertNotIn("qa_sheet", manifest)
        self.assertEqual(manifest["qa_matrix"]["source"], "local_xlsx")
        self.assertEqual(manifest["qa_matrix"]["path"], str(matrix_path))
        self.assertIn(
            "REBCLI-065-TC-20",
            manifest["qa_matrix"]["represented_test_ids"],
        )
        self.assertIn(
            "REBCLI-065-TC-21",
            manifest["qa_matrix"]["represented_test_ids"],
        )
        cases = {case["case"]: case for case in manifest["cases"]}
        self.assertEqual(
            cases["webui_mobile_live_llm_chat"]["qa_matrix_test_ids"],
            ["REBCLI-065-TC-20", "REBCLI-065-TC-21"],
        )
        self.assertTrue(cases["qa_2d_calendar_prep_live_chat"]["implemented"])
        self.assertEqual(
            cases["qa_2d_calendar_prep_live_chat"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_2f_calendar_prep_email_delivery"]["implemented"])
        self.assertEqual(
            cases["qa_2f_calendar_prep_email_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_4e_github_release_email_delivery"]["implemented"])
        self.assertEqual(
            cases["qa_4e_github_release_email_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_5d_slack_strategy_doc_answer"]["implemented"])
        self.assertTrue(cases["qa_5d_slack_strategy_doc_answer"]["requires_slack_target"])
        self.assertEqual(
            cases["qa_5d_slack_strategy_doc_answer"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_6e_gmail_to_sheet_delivery"]["implemented"])
        self.assertEqual(
            cases["qa_6e_gmail_to_sheet_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_7e_slack_bug_sheet_delivery"]["implemented"])
        self.assertTrue(cases["qa_7e_slack_bug_sheet_delivery"]["requires_slack_target"])
        self.assertEqual(
            cases["qa_7e_slack_bug_sheet_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertFalse(cases["qa_1a_telegram_connect"]["implemented"])
        self.assertEqual(
            cases["qa_1a_telegram_connect"]["status"],
            "gated:requires_live_telegram",
        )

    def test_gmail_delivery_target_prefers_explicit_env(self):
        target = asyncio.run(
            run_live_qa._gmail_delivery_target_email(
                access_token="unused-token",
                extra_env={"REBORN_WEBUI_V2_LIVE_QA_EMAIL_TARGET": "qa@example.test"},
            )
        )
        self.assertEqual(target, "qa@example.test")

    def test_extract_google_spreadsheet_id_from_url_or_label(self):
        spreadsheet_id = "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890"
        self.assertEqual(
            run_live_qa._extract_google_spreadsheet_id(
                f"Created: https://docs.google.com/spreadsheets/d/{spreadsheet_id}/edit#gid=0"
            ),
            spreadsheet_id,
        )
        self.assertEqual(
            run_live_qa._extract_google_spreadsheet_id(
                f"spreadsheet id: {spreadsheet_id}"
            ),
            spreadsheet_id,
        )
        self.assertIsNone(run_live_qa._extract_google_spreadsheet_id("no sheet here"))

    def test_google_runtime_token_requires_client_secret_for_expired_copied_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN": "expired-access-token",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "refresh-token",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id",
            }
            with patch.dict(os.environ, env, clear=True):
                seed = run_live_qa._seed_generated_google_product_auth_if_configured(
                    home,
                    "qa-user",
                )
            self.assertTrue(seed["seeded"])

            with patch.dict(os.environ, {}, clear=True):
                with self.assertRaisesRegex(
                    run_live_qa.LiveQaError,
                    "client id/secret env is incomplete",
                ):
                    run_live_qa._google_runtime_access_token(
                        home,
                        "qa-user",
                        {"IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id"},
                    )

    def test_bootstrap_forwards_all_cases_flag(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir) / "out"
            home = Path(tmpdir) / "home"
            argv = [
                "run_live_qa.py",
                "--output-dir",
                str(output_dir),
                "--reborn-home",
                str(home),
                "--all-cases",
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(run_live_qa, "bootstrap_python", return_value=Path("/venv/bin/python")),
                patch.object(run_live_qa, "install_playwright"),
                patch.object(run_live_qa.subprocess, "run") as subprocess_run,
            ):
                subprocess_run.return_value.returncode = 0
                self.assertEqual(run_live_qa.main(), 0)

            forwarded = subprocess_run.call_args.args[0]
            self.assertIn("--all-cases", forwarded)
            self.assertNotIn("--case", forwarded)

    def test_delivered_gate_routes_for_run_reads_trigger_gate_records(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3.connect(db_path) as db:
                db.execute(
                    """
                    CREATE TABLE root_filesystem_entries (
                        path TEXT PRIMARY KEY,
                        contents BLOB NOT NULL,
                        updated_at TEXT NOT NULL
                    )
                    """
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents, updated_at) "
                    "VALUES (?, ?, ?)",
                    (
                        "/tenants/reborn-cli/users/qa/outbound/delivered-gate-routes/route.json",
                        json.dumps(
                            {
                                "gate_ref": "gate:approval-abc",
                                "run_id": "run-123",
                                "scope": {"thread_id": "thread-456"},
                            }
                        ),
                        "2026-06-24T00:00:00Z",
                    ),
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents, updated_at) "
                    "VALUES (?, ?, ?)",
                    (
                        "/tenants/reborn-cli/users/qa/outbound/delivered-gate-routes/other.json",
                        json.dumps(
                            {
                                "gate_ref": "gate:approval-other",
                                "run_id": "run-other",
                                "scope": {"thread_id": "thread-other"},
                            }
                        ),
                        "2026-06-24T00:00:01Z",
                    ),
                )

            routes = run_live_qa._delivered_gate_routes_for_run(home, "run-123")

            self.assertEqual(
                routes,
                [
                    {
                        "path": "/tenants/reborn-cli/users/qa/outbound/delivered-gate-routes/route.json",
                        "gate_ref": "gate:approval-abc",
                        "thread_id": "thread-456",
                        "run_id": "run-123",
                    }
                ],
            )

    def test_github_auth_preflight_detects_configured_product_auth_account(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3.connect(db_path) as db:
                db.execute(
                    """
                    CREATE TABLE root_filesystem_entries (
                        path TEXT PRIMARY KEY,
                        contents BLOB NOT NULL
                    )
                    """
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents) VALUES (?, ?)",
                    (
                        "/tenants/reborn-cli/users/qa/secrets/agents/reborn-cli-agent/"
                        "product-auth/callback/accounts/github.json",
                        json.dumps(
                            {
                                "provider": "github",
                                "status": "configured",
                                "access_secret": "product-auth-manual-github",
                            }
                        ),
                    ),
                )

            preflight = run_live_qa._github_auth_preflight(
                home,
                {},
                requires_github_auth=True,
            )

            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)

    def test_github_auth_preflight_blocks_without_configured_account(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            (home / "local-dev").mkdir(parents=True)

            preflight = run_live_qa._github_auth_preflight(
                home,
                {},
                requires_github_auth=True,
            )

            self.assertFalse(preflight["ready"])
            self.assertIn("missing GitHub live prerequisites", preflight["reason"])

    def test_google_required_env_for_runtime_block_includes_refresh_inputs(self):
        required = run_live_qa._google_required_env_for_block(
            {
                "missing_google_client_secret": True,
                "refresh_probe_failed": True,
            },
            requires_runtime_access=True,
        )

        self.assertEqual(
            required,
            [
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN",
            ],
        )

    def test_google_required_env_for_connect_block_keeps_client_id_only(self):
        required = run_live_qa._google_required_env_for_block(
            {},
            requires_runtime_access=False,
        )

        self.assertEqual(required, ["IRONCLAW_REBORN_GOOGLE_CLIENT_ID"])

    def test_slack_delivery_observed_is_status_agnostic_after_gate_resume(self):
        self.assertTrue(
            run_live_qa._slack_delivery_observed(
                {"outcome": "delivered", "run_id": "run-123"},
                {"found": True, "marker_found": True},
            )
        )
        self.assertFalse(
            run_live_qa._slack_delivery_observed(
                {"outcome": "gate_required", "run_id": "run-123"},
                {"found": True, "marker_found": True},
            )
        )
        self.assertFalse(
            run_live_qa._slack_delivery_observed(
                {"outcome": "delivered", "run_id": "run-123"},
                {"found": False, "marker_found": True},
            )
        )

    def test_run_cases_isolates_reborn_home_and_preflight_per_selected_case(self):
        async def fake_case(ctx: run_live_qa.LiveQaContext) -> run_live_qa.ProbeResult:
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live",
                success=True,
                latency_ms=1,
                details={"reborn_home": str(ctx.reborn_home)},
            )

        async def fake_start_reborn_server(
            _binary: Path,
            reborn_home: Path,
            _output_dir: Path,
            _env: dict[str, str],
        ):
            return object(), f"http://127.0.0.1/{reborn_home.name}"

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw-reborn"
            binary.touch()
            args = argparse.Namespace(
                all_cases=False,
                case=["case_a", "case_b"],
                output_dir=output_dir,
                reborn_home=root / "missing-source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "case_a": run_live_qa.CaseSpec(fake_case),
                "case_b": run_live_qa.CaseSpec(fake_case),
            }
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
            }

            with (
                patch.dict(os.environ, env, clear=False),
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fake_start_reborn_server,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            self.assertEqual(status, 0)
            case_a_home = output_dir / "reborn-home" / "case_a"
            case_b_home = output_dir / "reborn-home" / "case_b"
            self.assertTrue((case_a_home / "config.toml").exists())
            self.assertTrue((case_b_home / "config.toml").exists())
            self.assertNotEqual(case_a_home, case_b_home)

            case_a_preflight = json.loads(
                (output_dir / "preflight.case_a.json").read_text(encoding="utf-8")
            )
            case_b_preflight = json.loads(
                (output_dir / "preflight.case_b.json").read_text(encoding="utf-8")
            )
            self.assertEqual(case_a_preflight["reborn_home"], str(case_a_home))
            self.assertEqual(case_b_preflight["reborn_home"], str(case_b_home))


if __name__ == "__main__":
    unittest.main()
