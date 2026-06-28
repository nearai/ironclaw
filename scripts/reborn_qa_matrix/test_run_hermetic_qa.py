#!/usr/bin/env python3
"""Unit tests for the Reborn QA matrix hermetic runner.

Run with::

    python3 scripts/reborn_qa_matrix/test_run_hermetic_qa.py
"""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

import run_hermetic_qa


class RebornQaMatrixHermeticRunnerTests(unittest.TestCase):
    def test_duration_parser_accepts_seconds_minutes_and_hours(self):
        self.assertEqual(run_hermetic_qa.parse_duration_seconds("42"), 42)
        self.assertEqual(run_hermetic_qa.parse_duration_seconds("30s"), 30)
        self.assertEqual(run_hermetic_qa.parse_duration_seconds("45m"), 2700)
        self.assertEqual(run_hermetic_qa.parse_duration_seconds("1h"), 3600)
        with self.assertRaises(ValueError):
            run_hermetic_qa.parse_duration_seconds("1.5m")

    def test_default_selection_keeps_only_matrix_owned_cases(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args([])

        selected = run_hermetic_qa._selected_case_names(args)

        self.assertIn("openai_responses_api_workflow_regression", selected)
        self.assertIn("openai_chat_completions_workflow_regression", selected)
        self.assertIn("webui_v2_chat_client_regression", selected)
        self.assertIn("webui_v2_workspace_project_client_regression", selected)
        self.assertNotIn("openai_compat_owner_crate_regression", selected)
        self.assertNotIn("support_substrate_product_workflow_regression", selected)
        self.assertNotIn("webui_v2_route_contract_regression", selected)
        self.assertNotIn(
            "webui_v2_gateway_middleware_serve_foundation_regression",
            selected,
        )

    def test_removed_existing_ci_coverage_has_no_opt_back_flag(self):
        parser = run_hermetic_qa.build_parser()

        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(["--run-existing-ci-coverage"])

    def test_ci_only_case_cannot_be_selected_for_execution(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(["--case", "support_substrate_product_workflow_regression"])

        with self.assertRaises(SystemExit):
            run_hermetic_qa._selected_case_names(args)

    def test_manifest_emits_only_active_cases(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            selected = ["openai_responses_api_workflow_regression"]

            manifest_path = run_hermetic_qa.write_case_manifest(output_dir, selected)
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

            self.assertEqual(manifest["selected_cases"], selected)
            self.assertEqual(
                manifest["qa_matrix"]["selected_represented_test_ids"],
                [
                    "REBCLI-057-TC-01",
                    "REBCLI-057-TC-02",
                    "REBCLI-057-TC-03",
                    "REBCLI-057-TC-04",
                    "REBCLI-057-TC-05",
                    "REBCLI-057-TC-06",
                    "REBCLI-058-TC-01",
                    "REBCLI-058-TC-02",
                    "REBCLI-058-TC-03",
                    "REBCLI-058-TC-04",
                    "REBCLI-058-TC-05",
                    "REBCLI-058-TC-06",
                ],
            )
            case_names = {case["case"] for case in manifest["cases"]}
            self.assertIn("openai_responses_api_workflow_regression", case_names)
            self.assertIn("webui_v2_chat_client_regression", case_names)
            self.assertNotIn("support_substrate_product_workflow_regression", case_names)
            self.assertNotIn("webui_v2_route_contract_regression", case_names)
            self.assertNotIn("openai_compat_owner_crate_regression", case_names)

    def test_manifest_does_not_represent_existing_ci_only_cases(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            selected = ["openai_responses_api_workflow_regression"]

            manifest_path = run_hermetic_qa.write_case_manifest(output_dir, selected)
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

            self.assertEqual(manifest["qa_matrix"]["existing_ci_only_test_ids"], [])
            represented_ids = set(manifest["qa_matrix"]["represented_test_ids"])
            self.assertNotIn("REBCLI-043-TC-12", represented_ids)
            cases = {case["case"] for case in manifest["cases"]}
            self.assertNotIn("support_substrate_product_workflow_regression", cases)
            self.assertEqual(
                manifest["qa_matrix"]["represented_test_id_count"],
                len(represented_ids),
            )

    def test_responses_api_dry_run_writes_results_and_logs(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "openai_responses_api_workflow_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertTrue(results["success"])
            self.assertFalse(results["run_existing_ci_coverage"])
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                [
                    "openai_responses_workflow_handlers_contract",
                    "openai_responses_streaming_handlers_contract",
                ],
            )
            for command in commands:
                self.assertTrue(Path(command["stdout_log"]).exists())
                self.assertTrue(Path(command["stderr_log"]).exists())

    def test_webui_mixed_case_prunes_ci_owned_browser_duplicate(self):
        case = run_hermetic_qa.CASES["webui_v2_workspace_project_client_regression"]

        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            [
                "reborn_cli_webui_v2_binary",
                "webui_v2_workspace_project_client_contracts",
            ],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            ["webui_v2_projects_browser_smoke", "webui_v2_workspace_browser_smoke"],
        )

    def test_previous_command_failure_skips_remaining_matrix_owned_commands(self):
        case = run_hermetic_qa.CaseSpec(
            name="synthetic_failure",
            feature="Synthetic",
            category="Synthetic",
            qa_matrix_test_ids=["REBCLI-TEST"],
            commands=[
                run_hermetic_qa.CommandSpec(
                    name="fail",
                    argv=[sys.executable, "-c", "import sys; sys.exit(7)"],
                ),
                run_hermetic_qa.CommandSpec(
                    name="skip",
                    argv=[sys.executable, "-c", "import sys; sys.exit(0)"],
                ),
            ],
        )
        with tempfile.TemporaryDirectory() as tmpdir:
            result = run_hermetic_qa.run_case(
                case,
                output_dir=Path(tmpdir),
                timeout_seconds=30,
                dry_run=False,
            )

            self.assertFalse(result["success"])
            commands = result["details"]["commands"]
            self.assertEqual(commands[0]["returncode"], 7)
            self.assertTrue(commands[1]["skipped"])


if __name__ == "__main__":
    unittest.main()
