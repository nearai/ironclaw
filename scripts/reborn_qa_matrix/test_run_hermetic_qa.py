#!/usr/bin/env python3
"""Unit tests for the Reborn QA matrix hermetic runner.

Run with::

    python3 scripts/reborn_qa_matrix/test_run_hermetic_qa.py
"""

from __future__ import annotations

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

    def test_manifest_tracks_selected_and_total_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            selected = ["openai_compat_owner_crate_regression"]
            manifest_path = run_hermetic_qa.write_case_manifest(output_dir, selected)

            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            self.assertEqual(manifest["selected_cases"], selected)
            self.assertEqual(
                manifest["qa_matrix"]["selected_represented_test_ids"],
                ["REBCLI-056-TC-07"],
            )
            self.assertIn(
                "REBCLI-056-TC-08",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-056-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-056-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-055-TC-07",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-055-TC-12",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-057-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-058-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-065-TC-25",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "openai_responses_api_workflow_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "openai_chat_completions_workflow_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_route_contract_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_static_js_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_composition_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "support_substrate_product_workflow_regression",
                {case["case"] for case in manifest["cases"]},
            )

    def test_dry_run_writes_results_and_logs_without_running_cargo(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "support_substrate_product_workflow_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertTrue(results["success"])
            self.assertTrue(results["dry_run"])
            self.assertEqual(
                results["summary"]["qa_matrix_test_ids"],
                [
                    "REBCLI-043-TC-12",
                    "REBCLI-044-TC-07",
                    "REBCLI-045-TC-10",
                    "REBCLI-047-TC-07",
                    "REBCLI-056-TC-08",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(len(commands), 2)
            for command in commands:
                self.assertTrue(Path(command["stdout_log"]).exists())
                self.assertTrue(Path(command["stderr_log"]).exists())

    def test_webui_route_contract_case_dry_run_maps_focused_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_route_contract_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                results["summary"]["qa_matrix_test_ids"],
                [
                    "REBCLI-055-TC-08",
                    "REBCLI-065-TC-23",
                    "REBCLI-065-TC-24",
                    "REBCLI-065-TC-25",
                ],
            )
            self.assertEqual(
                [command["name"] for command in results["results"][0]["details"]["commands"]],
                [
                    "webui_v2_send_multiline_contract",
                    "webui_v2_send_error_contract",
                    "webui_v2_cancel_error_contract",
                    "webui_v2_route_contracts",
                ],
            )

    def test_webui_composition_case_dry_run_maps_gateway_matrix_id(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_composition_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                results["summary"]["qa_matrix_test_ids"],
                ["REBCLI-055-TC-09"],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(commands[0]["name"], "webui_v2_composition_regression")
            self.assertIn(
                "--test webui_v2_product_auth_4201",
                commands[0]["command"],
            )

    def test_responses_api_case_dry_run_maps_create_retrieve_cancel_matrix_ids(self):
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
            self.assertEqual(
                results["summary"]["qa_matrix_test_ids"],
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
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                commands[0]["name"],
                "openai_responses_workflow_handlers_contract",
            )
            self.assertIn(
                "--test responses_workflow_handlers_contract",
                commands[0]["command"],
            )

    def test_webui_static_js_case_dry_run_maps_static_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_static_js_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                results["summary"]["qa_matrix_test_ids"],
                ["REBCLI-055-TC-07", "REBCLI-055-TC-12"],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(commands[0]["name"], "webui_v2_static_js_suite")
            self.assertIn("node --test", commands[0]["command"])

    def test_chat_completions_case_dry_run_maps_primary_chat_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "openai_chat_completions_workflow_regression",
                    "--dry-run",
                ]
            )

            self.assertEqual(exit_code, 0)
            results = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                results["summary"]["qa_matrix_test_ids"],
                [
                    "REBCLI-056-TC-01",
                    "REBCLI-056-TC-02",
                    "REBCLI-056-TC-03",
                    "REBCLI-056-TC-04",
                    "REBCLI-056-TC-05",
                    "REBCLI-056-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                commands[0]["name"],
                "openai_chat_workflow_handlers_contract",
            )
            self.assertIn(
                "--test chat_workflow_handlers_contract",
                commands[0]["command"],
            )

    def test_failed_command_stops_later_commands_in_case(self):
        case = run_hermetic_qa.CaseSpec(
            name="synthetic_failure",
            feature="Synthetic",
            category="Unit",
            qa_matrix_test_ids=["REBCLI-000-TC-00"],
            commands=[
                run_hermetic_qa.CommandSpec(
                    name="fail",
                    argv=[
                        sys.executable,
                        "-c",
                        "import sys; sys.exit(7)",
                    ],
                ),
                run_hermetic_qa.CommandSpec(
                    name="skip",
                    argv=[
                        sys.executable,
                        "-c",
                        "import sys; sys.exit(0)",
                    ],
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
