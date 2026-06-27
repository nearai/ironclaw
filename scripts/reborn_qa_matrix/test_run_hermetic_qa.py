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
                "REBCLI-048-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-048-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-049-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-049-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-050-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-050-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-051-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-051-TC-06",
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
                "REBCLI-059-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-059-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-060-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-060-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-061-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-061-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-062-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-062-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-063-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-063-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-064-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-064-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-065-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-065-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-065-TC-25",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-066-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-066-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-067-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-067-TC-06",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-097-TC-01",
                manifest["qa_matrix"]["represented_test_ids"],
            )
            self.assertIn(
                "REBCLI-097-TC-06",
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
                "webui_v2_chat_client_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_workspace_project_client_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_automations_client_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_project_files_api_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_project_membership_api_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_public_sso_session_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_product_auth_oauth_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_extension_oauth_setup_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_manual_token_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_product_auth_account_lifecycle_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_spa_static_serving_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_login_session_state_regression",
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
            self.assertIn(
                "webui_v2_operator_config_api_regression",
                {case["case"] for case in manifest["cases"]},
            )
            self.assertIn(
                "webui_v2_provider_login_api_regression",
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

    def test_webui_chat_client_case_dry_run_maps_chat_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_chat_client_regression",
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
                    "REBCLI-065-TC-01",
                    "REBCLI-065-TC-02",
                    "REBCLI-065-TC-03",
                    "REBCLI-065-TC-04",
                    "REBCLI-065-TC-05",
                    "REBCLI-065-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                ["webui_v2_chat_client_contracts"],
            )
            self.assertIn("node --test", commands[0]["command"])
            self.assertIn("static/js/pages/chat", commands[0]["command"])

    def test_webui_workspace_project_case_dry_run_maps_client_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_workspace_project_client_regression",
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
                    "REBCLI-066-TC-01",
                    "REBCLI-066-TC-02",
                    "REBCLI-066-TC-03",
                    "REBCLI-066-TC-04",
                    "REBCLI-066-TC-05",
                    "REBCLI-066-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                ["webui_v2_workspace_project_client_contracts"],
            )
            self.assertIn("workspace-api.test.mjs", commands[0]["command"])
            self.assertIn("projects-api.test.mjs", commands[0]["command"])

    def test_webui_automations_case_dry_run_maps_client_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_automations_client_regression",
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
                    "REBCLI-067-TC-01",
                    "REBCLI-067-TC-02",
                    "REBCLI-067-TC-03",
                    "REBCLI-067-TC-04",
                    "REBCLI-067-TC-05",
                    "REBCLI-067-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                ["webui_v2_automations_client_contracts"],
            )
            self.assertIn("static/js/lib/api.test.mjs", commands[0]["command"])
            self.assertIn("static/js/pages/automations", commands[0]["command"])

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

    def test_webui_static_serving_case_dry_run_maps_spa_static_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_spa_static_serving_regression",
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
                    "REBCLI-063-TC-01",
                    "REBCLI-063-TC-02",
                    "REBCLI-063-TC-03",
                    "REBCLI-063-TC-04",
                    "REBCLI-063-TC-05",
                    "REBCLI-063-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                [
                    "webui_v2_static_router_contracts",
                    "webui_v2_composition_static_route_contracts",
                ],
            )
            self.assertIn("ironclaw_webui_v2_static", commands[0]["command"])
            self.assertIn("router", commands[0]["command"])
            self.assertIn("--test webui_v2_serve", commands[1]["command"])
            self.assertIn("static", commands[1]["command"])

    def test_webui_login_session_state_case_dry_run_maps_auth_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_login_session_state_regression",
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
                    "REBCLI-064-TC-01",
                    "REBCLI-064-TC-02",
                    "REBCLI-064-TC-03",
                    "REBCLI-064-TC-04",
                    "REBCLI-064-TC-05",
                    "REBCLI-064-TC-06",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertEqual(
                [command["name"] for command in commands],
                [
                    "webui_v2_static_auth_js_contract",
                    "webui_v2_static_api_auth_client_contracts",
                    "webui_v2_ingress_session_auth_contracts",
                ],
            )
            self.assertIn("auth_js_carries_login_ticket_contract", commands[0]["command"])
            self.assertIn("api.test.mjs", commands[1]["command"])
            self.assertIn("ironclaw_reborn_webui_ingress", commands[2]["command"])
            self.assertIn("session", commands[2]["command"])

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

    def test_provider_login_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_provider_login_api_regression",
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
                    "REBCLI-097-TC-01",
                    "REBCLI-097-TC-02",
                    "REBCLI-097-TC-03",
                    "REBCLI-097-TC-04",
                    "REBCLI-097-TC-05",
                    "REBCLI-097-TC-06",
                ],
            )
            self.assertEqual(
                [command["name"] for command in results["results"][0]["details"]["commands"]],
                [
                    "webui_v2_llm_provider_routes",
                    "webui_v2_nearai_login_state_contracts",
                    "webui_v2_provider_login_multi_user_mount_policy",
                ],
            )
            self.assertIn(
                "--test webui_v2_handlers_contract",
                results["results"][0]["details"]["commands"][0]["command"],
            )
            self.assertIn(
                "operator_routes_are_not_mounted_for_multi_user_authenticator",
                results["results"][0]["details"]["commands"][2]["command"],
            )

    def test_operator_config_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_operator_config_api_regression",
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
                    "REBCLI-048-TC-01",
                    "REBCLI-048-TC-02",
                    "REBCLI-048-TC-03",
                    "REBCLI-048-TC-04",
                    "REBCLI-048-TC-05",
                    "REBCLI-048-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_descriptor_policy_surface",
                    "webui_v2_llm_provider_routes",
                    "webui_v2_operator_handler_contracts",
                    "webui_v2_operator_mount_policy",
                    "webui_v2_operator_llm_config_persistence",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("--test webui_v2_descriptors_contract", commands[0]["command"])
            self.assertIn("--test webui_v2_handlers_contract", commands[1]["command"])
            self.assertIn("operator_", commands[2]["command"])
            self.assertIn("operator_llm_config", commands[4]["command"])

    def test_project_files_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_project_files_api_regression",
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
                    "REBCLI-049-TC-01",
                    "REBCLI-049-TC-02",
                    "REBCLI-049-TC-03",
                    "REBCLI-049-TC-04",
                    "REBCLI-049-TC-05",
                    "REBCLI-049-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_filesystem_handler_slice",
                    "composition_project_filesystem_reader",
                    "composition_mount_filesystem_reader",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("fs_", commands[0]["command"])
            self.assertIn("project_filesystem_reader", commands[1]["command"])
            self.assertIn("mount_filesystem_reader", commands[2]["command"])

    def test_project_membership_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_project_membership_api_regression",
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
                    "REBCLI-050-TC-01",
                    "REBCLI-050-TC-02",
                    "REBCLI-050-TC-03",
                    "REBCLI-050-TC-04",
                    "REBCLI-050-TC-05",
                    "REBCLI-050-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_descriptor_policy_surface",
                    "webui_v2_project_handler_contracts",
                    "webui_v2_projects_handler_contracts",
                    "webui_v2_member_handler_contracts",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("--test webui_v2_descriptors_contract", commands[0]["command"])
            self.assertIn("project_", commands[1]["command"])
            self.assertIn("projects", commands[2]["command"])
            self.assertIn("member", commands[3]["command"])

    def test_public_sso_session_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_public_sso_session_regression",
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
                    "REBCLI-051-TC-01",
                    "REBCLI-051-TC-02",
                    "REBCLI-051-TC-03",
                    "REBCLI-051-TC-04",
                    "REBCLI-051-TC-05",
                    "REBCLI-051-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_sso_auth_route_contracts",
                    "webui_v2_google_oauth_routes",
                    "webui_v2_github_oauth_routes",
                    "webui_v2_sso_session_round_trip",
                    "webui_v2_sso_network_limits",
                    "webui_v2_sso_public_mount_policy",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("--test auth_route_contract", commands[0]["command"])
            self.assertIn("--test google_oauth_routes", commands[1]["command"])
            self.assertIn("--test github_oauth_routes", commands[2]["command"])
            self.assertIn("--test session_round_trip", commands[3]["command"])
            self.assertIn("--test network_limits_contract", commands[4]["command"])
            self.assertIn("public_route_mount_is_merged", commands[5]["command"])

    def test_product_auth_oauth_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_product_auth_oauth_regression",
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
                    "REBCLI-059-TC-01",
                    "REBCLI-059-TC-02",
                    "REBCLI-059-TC-03",
                    "REBCLI-059-TC-04",
                    "REBCLI-059-TC-05",
                    "REBCLI-059-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_product_auth_oauth_routes",
                    "webui_v2_product_auth_google_oauth_routes",
                    "webui_v2_product_auth_callback_routes",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("product_auth_oauth", commands[0]["command"])
            self.assertIn("product_auth_google_oauth", commands[1]["command"])
            self.assertIn("product_auth_callback", commands[2]["command"])

    def test_extension_oauth_setup_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_extension_oauth_setup_regression",
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
                    "REBCLI-060-TC-01",
                    "REBCLI-060-TC-02",
                    "REBCLI-060-TC-03",
                    "REBCLI-060-TC-04",
                    "REBCLI-060-TC-05",
                    "REBCLI-060-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_extension_oauth_route_contract",
                    "webui_v2_extension_oauth_start_contracts",
                    "webui_v2_extension_google_oauth_start_contracts",
                    "webui_v2_dcr_oauth_callback_contracts",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("extension_oauth", commands[0]["command"])
            self.assertIn("extension_oauth_start", commands[1]["command"])
            self.assertIn("extension_google_oauth_start", commands[2]["command"])
            self.assertIn("dcr_oauth_callback", commands[3]["command"])

    def test_manual_token_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_manual_token_regression",
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
                    "REBCLI-061-TC-01",
                    "REBCLI-061-TC-02",
                    "REBCLI-061-TC-03",
                    "REBCLI-061-TC-04",
                    "REBCLI-061-TC-05",
                    "REBCLI-061-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_manual_token_legacy_submit_routes",
                    "webui_v2_manual_token_split_routes",
                    "webui_v2_manual_token_facade_contracts",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("product_auth_manual_token", commands[0]["command"])
            self.assertIn("webui_v2_product_auth_4201", commands[1]["command"])
            self.assertIn("manual_tokens", commands[2]["command"])
            self.assertIn("manual_token_facade", commands[2]["command"])

    def test_product_auth_account_lifecycle_case_dry_run_maps_api_matrix_ids(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            exit_code = run_hermetic_qa.main(
                [
                    "--output-dir",
                    str(output_dir),
                    "--case",
                    "webui_v2_product_auth_account_lifecycle_regression",
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
                    "REBCLI-062-TC-01",
                    "REBCLI-062-TC-02",
                    "REBCLI-062-TC-03",
                    "REBCLI-062-TC-04",
                    "REBCLI-062-TC-05",
                    "REBCLI-062-TC-06",
                ],
            )
            self.assertEqual(
                [
                    command["name"]
                    for command in results["results"][0]["details"]["commands"]
                ],
                [
                    "webui_v2_product_auth_account_routes",
                    "webui_v2_product_auth_lifecycle_cleanup_routes",
                ],
            )
            commands = results["results"][0]["details"]["commands"]
            self.assertIn("webui_v2_product_auth_4201", commands[0]["command"])
            self.assertIn("account", commands[0]["command"])
            self.assertIn("webui_v2_product_auth_4201", commands[1]["command"])
            self.assertIn("lifecycle", commands[1]["command"])

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
