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

        self.assertIn("openai_compat_beta_routes_regression", selected)
        self.assertIn("openai_responses_missing_cancel_shape_regression", selected)
        self.assertIn("openai_responses_external_tools_e2e_regression", selected)
        self.assertIn("openai_chat_completions_workflow_regression", selected)
        self.assertIn("openai_models_list_api_regression", selected)
        self.assertIn("webui_v2_session_thread_message_api_regression", selected)
        self.assertIn("webui_v2_streaming_run_control_api_regression", selected)
        self.assertIn("webui_v2_operator_config_api_regression", selected)
        self.assertNotIn("webui_v2_chat_client_regression", selected)
        self.assertNotIn("webui_v2_workspace_project_client_regression", selected)
        self.assertNotIn("webui_v2_settings_toolbar_search_regression", selected)
        self.assertNotIn("webui_v2_provider_login_api_regression", selected)
        self.assertNotIn("webui_v2_admin_console_usage_regression", selected)
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

        with self.assertRaisesRegex(SystemExit, "removed from the executable QA lane"):
            run_hermetic_qa._selected_case_names(args)

    def test_ci_owned_contract_cases_are_removed_from_case_registry(self):
        self.assertNotIn(
            "support_substrate_product_workflow_regression",
            run_hermetic_qa.CASES,
        )
        self.assertNotIn(
            "webui_v2_session_service_substrate_contracts",
            {
                command.name
                for case in run_hermetic_qa.CASES.values()
                for command in case.commands
            },
        )

    def test_responses_missing_cancel_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(
            ["--case", "openai_responses_missing_cancel_shape_regression"]
        )

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["openai_responses_missing_cancel_shape_regression"],
        )
        case = run_hermetic_qa.CASES[
            "openai_responses_missing_cancel_shape_regression"
        ]
        self.assertEqual(case.qa_matrix_test_ids, ["REBCLI-058-TC-02"])
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["openai_responses_missing_cancel_shape_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_default_executable_lane_has_no_cargo_test_commands(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args([])
        selected = run_hermetic_qa._selected_case_names(args)

        cargo_tests = [
            command.name
            for name in selected
            for command in run_hermetic_qa._commands_for_case(run_hermetic_qa.CASES[name])
            if command.argv[:2] == ["cargo", "test"]
        ]

        self.assertEqual(cargo_tests, [])

    def test_default_executable_lane_has_no_static_contract_commands(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args([])
        selected = run_hermetic_qa._selected_case_names(args)

        static_contract_commands = [
            command.name
            for name in selected
            for command in run_hermetic_qa._commands_for_case(run_hermetic_qa.CASES[name])
            if "node --test" in run_hermetic_qa.render_command(command)
            or command.name.endswith("_contract")
            or command.name.endswith("_contracts")
        ]

        self.assertEqual(static_contract_commands, [])

    def test_default_executable_lane_has_no_browser_smoke_commands(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args([])
        selected = run_hermetic_qa._selected_case_names(args)

        browser_commands = [
            command.name
            for name in selected
            for command in run_hermetic_qa._commands_for_case(run_hermetic_qa.CASES[name])
            if command.name.endswith("_browser_smoke")
        ]

        self.assertEqual(browser_commands, [])

    def test_session_thread_message_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(["--case", "webui_v2_session_thread_message_api_regression"])

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["webui_v2_session_thread_message_api_regression"],
        )
        case = run_hermetic_qa.CASES["webui_v2_session_thread_message_api_regression"]
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["webui_v2_session_thread_message_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_models_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(["--case", "openai_models_list_api_regression"])

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["openai_models_list_api_regression"],
        )
        case = run_hermetic_qa.CASES["openai_models_list_api_regression"]
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["openai_models_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_chat_completions_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(
            ["--case", "openai_chat_completions_workflow_regression"]
        )

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["openai_chat_completions_workflow_regression"],
        )
        case = run_hermetic_qa.CASES["openai_chat_completions_workflow_regression"]
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["openai_chat_completions_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_openai_compat_route_mount_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(["--case", "openai_compat_beta_routes_regression"])

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["openai_compat_beta_routes_regression"],
        )
        case = run_hermetic_qa.CASES["openai_compat_beta_routes_regression"]
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["openai_compat_route_mount_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_operator_config_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(["--case", "webui_v2_operator_config_api_regression"])

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["webui_v2_operator_config_api_regression"],
        )
        case = run_hermetic_qa.CASES["webui_v2_operator_config_api_regression"]
        self.assertEqual(
            case.qa_matrix_test_ids,
            [
                "REBCLI-048-TC-01",
                "REBCLI-048-TC-02",
                "REBCLI-048-TC-03",
                "REBCLI-048-TC-04",
                "REBCLI-048-TC-05",
                "REBCLI-048-TC-06",
            ],
        )
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["webui_v2_operator_config_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_streaming_run_control_case_executes_only_served_e2e_command(self):
        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(
            ["--case", "webui_v2_streaming_run_control_api_regression"]
        )

        self.assertEqual(
            run_hermetic_qa._selected_case_names(args),
            ["webui_v2_streaming_run_control_api_regression"],
        )
        case = run_hermetic_qa.CASES["webui_v2_streaming_run_control_api_regression"]
        self.assertEqual(
            case.qa_matrix_test_ids,
            [
                "REBCLI-044-TC-01",
                "REBCLI-044-TC-02",
                "REBCLI-044-TC-03",
                "REBCLI-044-TC-04",
                "REBCLI-044-TC-05",
                "REBCLI-044-TC-06",
            ],
        )
        self.assertEqual(
            [command.name for command in run_hermetic_qa._commands_for_case(case)],
            ["webui_v2_streaming_run_control_served_e2e"],
        )
        self.assertEqual(
            [command["name"] for command in run_hermetic_qa._removed_existing_ci_commands(case)],
            [],
        )

    def test_manifest_emits_only_active_cases(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            selected = ["openai_responses_external_tools_e2e_regression"]

            manifest_path = run_hermetic_qa.write_case_manifest(output_dir, selected)
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

            self.assertEqual(manifest["selected_cases"], selected)
            self.assertEqual(
                manifest["qa_matrix"]["selected_represented_test_ids"],
                [
                    "REBCLI-100-TC-01",
                    "REBCLI-100-TC-02",
                    "REBCLI-100-TC-03",
                    "REBCLI-100-TC-04",
                    "REBCLI-100-TC-05",
                    "REBCLI-100-TC-06",
                ],
            )
            case_names = {case["case"] for case in manifest["cases"]}
            self.assertIn("openai_responses_external_tools_e2e_regression", case_names)
            self.assertIn(
                "openai_responses_missing_cancel_shape_regression",
                case_names,
            )
            self.assertNotIn("webui_v2_chat_client_regression", case_names)
            self.assertNotIn("webui_v2_settings_toolbar_search_regression", case_names)
            self.assertNotIn("webui_v2_provider_login_api_regression", case_names)
            self.assertNotIn("support_substrate_product_workflow_regression", case_names)
            self.assertNotIn("webui_v2_route_contract_regression", case_names)
            self.assertNotIn("openai_compat_owner_crate_regression", case_names)

    def test_manifest_does_not_represent_existing_ci_only_cases(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            selected = ["openai_responses_external_tools_e2e_regression"]

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

    def test_responses_external_tools_e2e_dry_run_writes_results_and_logs(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "openai_responses_external_tools_e2e_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertTrue(results["success"])
            self.assertEqual(
                results["results"][0]["details"]["qa_matrix_test_ids"],
                [
                    "REBCLI-100-TC-01",
                    "REBCLI-100-TC-02",
                    "REBCLI-100-TC-03",
                    "REBCLI-100-TC-04",
                    "REBCLI-100-TC-05",
                    "REBCLI-100-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                ["openai_responses_external_tools_served_e2e"],
            )
            self.assertIn(
                "test_reborn_responses_repeated_external_tools_round_trip",
                commands[0]["command"],
            )
            self.assertIn(
                "test_reborn_responses_rejects_wrong_external_tool_call_id",
                commands[0]["command"],
            )
            self.assertTrue(Path(commands[0]["stdout_log"]).exists())
            self.assertTrue(Path(commands[0]["stderr_log"]).exists())

    def test_browser_or_static_only_cases_are_not_executable_cases(self):
        self.assertNotIn(
            "webui_v2_workspace_project_client_regression",
            run_hermetic_qa.CASES,
        )

        parser = run_hermetic_qa.build_parser()
        args = parser.parse_args(
            ["--case", "webui_v2_workspace_project_client_regression"]
        )

        with self.assertRaisesRegex(SystemExit, "unknown case"):
            run_hermetic_qa._selected_case_names(args)

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
