#!/usr/bin/env python3
"""Contract tests for affected-area Reborn PR test planning."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/ci/reborn_pr_test_plan.py"
SPEC = importlib.util.spec_from_file_location("reborn_pr_test_plan", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
planner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = planner
SPEC.loader.exec_module(planner)


def metadata() -> dict:
    root = str(ROOT / "Cargo.toml")
    alpha = str(ROOT / "crates/alpha/Cargo.toml")
    beta = str(ROOT / "crates/beta/Cargo.toml")
    gamma = str(ROOT / "crates/gamma/Cargo.toml")
    return {
        "workspace_members": ["root", "alpha", "beta", "gamma"],
        "packages": [
            {"id": "root", "name": "ironclaw_reborn_integration_tests", "manifest_path": root},
            {"id": "alpha", "name": "alpha", "manifest_path": alpha},
            {"id": "beta", "name": "beta", "manifest_path": beta},
            {"id": "gamma", "name": "gamma", "manifest_path": gamma},
        ],
        "resolve": {
            "nodes": [
                {"id": "root", "deps": [{"pkg": "gamma"}]},
                {"id": "alpha", "deps": []},
                {"id": "beta", "deps": [{"pkg": "alpha"}]},
                {"id": "gamma", "deps": [{"pkg": "beta"}]},
            ]
        },
    }


class RebornPrTestPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.original_bucket_packages = planner._bucket_packages
        planner._bucket_packages = lambda packages: (
            [{"name": "selected", "packages": packages}] if packages else []
        )
        self.canonical = ["alpha", "beta", "gamma"]

    def tearDown(self) -> None:
        planner._bucket_packages = self.original_bucket_packages

    def plan(self, event: str, paths: list[str]) -> dict:
        return planner.build_plan(
            event=event,
            changed_paths=paths,
            metadata=metadata(),
            canonical_packages=self.canonical,
        )

    def test_merge_queue_is_always_exhaustive(self) -> None:
        plan = self.plan("merge_group", ["crates/alpha/src/lib.rs"])
        self.assertEqual(plan["mode"], "full")
        self.assertEqual(plan["coverage_mode"], "full")
        self.assertEqual(plan["root_partitions"], [0, 1, 2, 3])
        self.assertEqual(plan["integration_lanes"], [0, 1, 2, 3, "groups"])

    def test_changed_package_includes_transitive_reverse_dependents(self) -> None:
        plan = self.plan("pull_request", ["crates/alpha/src/lib.rs"])
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["changed_packages"], ["alpha"])
        self.assertEqual(plan["affected_packages"], ["alpha", "beta", "gamma"])
        self.assertEqual(
            plan["crate_buckets"],
            [{"name": "selected", "packages": ["alpha", "beta", "gamma"]}],
        )
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["coverage_mode"], "none")

    def test_high_fanout_package_keeps_consumers_in_bounded_jobs(self) -> None:
        wide = metadata()
        for index in range(5):
            package_id = f"consumer-{index}"
            package_name = f"consumer_{index}"
            wide["workspace_members"].append(package_id)
            wide["packages"].append(
                {
                    "id": package_id,
                    "name": package_name,
                    "manifest_path": str(
                        ROOT / f"crates/{package_name}/Cargo.toml"
                    ),
                }
            )
            wide["resolve"]["nodes"].append(
                {"id": package_id, "deps": [{"pkg": "alpha"}]}
            )
        canonical = ["alpha"] + [f"consumer_{index}" for index in range(5)]
        planner._bucket_packages = lambda packages: [
            {"name": package, "packages": [package]} for package in packages
        ]

        plan = planner.build_plan(
            event="pull_request",
            changed_paths=["crates/alpha/src/lib.rs"],
            metadata=wide,
            canonical_packages=canonical,
        )

        self.assertEqual(plan["affected_packages"], canonical)
        self.assertEqual(len(plan["crate_buckets"]), 3)
        self.assertEqual(
            sorted(
                package
                for bucket in plan["crate_buckets"]
                for package in bucket["packages"]
            ),
            canonical,
        )
        self.assertIn("without omitting packages", plan["reasons"][-1])

    def test_bounded_jobs_do_not_split_canonical_buckets(self) -> None:
        source = [
            {"name": "reborn-core", "packages": ["ironclaw", "runner"]},
            {"name": "composition-core", "packages": ["composition"]},
            {"name": "webui-ingress", "packages": ["attachments", "webui"]},
            {"name": "memory-skills", "packages": ["memory", "skills"]},
        ]
        bounded = planner._bound_pr_buckets(source, max_buckets=3)

        self.assertEqual(len(bounded), 3)
        for bucket in source:
            package_set = set(bucket["packages"])
            self.assertTrue(
                any(package_set <= set(candidate["packages"]) for candidate in bounded)
            )

    def test_frontend_change_runs_frontend_and_baseline_qa_replay(self) -> None:
        plan = self.plan(
            "pull_request", ["crates/ironclaw_webui/frontend/src/app.tsx"]
        )
        self.assertEqual(plan["mode"], "selected")
        self.assertTrue(plan["run_frontend"])
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["crate_buckets"], [])
        self.assertEqual(plan["integration_lanes"], [])

    def test_nested_crate_markdown_remains_package_owned(self) -> None:
        plan = self.plan("pull_request", ["crates/alpha/README.md"])
        self.assertEqual(plan["changed_packages"], ["alpha"])
        self.assertNotEqual(plan["mode"], "none")

    def test_recorded_fixture_change_runs_only_qa_replay(self) -> None:
        plan = self.plan(
            "pull_request",
            ["tests/fixtures/llm_traces/reborn_qa/example.json"],
        )
        self.assertEqual(plan["mode"], "selected")
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["crate_buckets"], [])

    def test_unrelated_workflow_change_runs_only_baseline_qa_replay(self) -> None:
        plan = self.plan("pull_request", [".github/workflows/code_style.yml"])
        self.assertEqual(plan["mode"], "none")
        self.assertTrue(plan["run_qa_replay"])

    def test_reborn_workflow_change_fails_closed_to_full_pr_plan(self) -> None:
        plan = self.plan("pull_request", [".github/workflows/reborn-tests.yml"])
        self.assertEqual(plan["mode"], "full")
        self.assertEqual(plan["root_partitions"], [0, 1, 2, 3])
        self.assertEqual(plan["integration_lanes"], [0, 1, 2, 3, "groups"])
        self.assertTrue(plan["run_frontend"])
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["coverage_mode"], "full")

    def test_empty_diff_fails_closed_to_full_pr_plan(self) -> None:
        plan = self.plan("pull_request", [])
        self.assertEqual(plan["mode"], "full")

    def test_reborn_caller_workflow_fails_closed_to_full_pr_plan(self) -> None:
        plan = self.plan("pull_request", [".github/workflows/nightly-deep-ci.yml"])
        self.assertEqual(plan["mode"], "full")

    def test_coverage_policy_change_runs_full_coverage_on_pr(self) -> None:
        plan = self.plan(
            "pull_request", ["tests/integration/coverage-floor.toml"]
        )
        self.assertEqual(plan["mode"], "full")
        self.assertEqual(plan["coverage_mode"], "full")

    def test_noncanonical_package_fails_closed_to_full_pr_plan(self) -> None:
        plan = planner.build_plan(
            event="pull_request",
            changed_paths=["crates/gamma/src/lib.rs"],
            metadata=metadata(),
            canonical_packages=["alpha", "beta"],
        )
        self.assertEqual(plan["mode"], "full")

    def test_generated_integration_suites_are_assigned_to_flat_lanes(self) -> None:
        lanes = planner._integration_test_lanes()
        self.assertIn("tests/integration/generated_gate_sequences.rs", lanes)
        self.assertIn("tests/integration/generated_restart_sequences.rs", lanes)
        self.assertIsInstance(
            lanes["tests/integration/generated_gate_sequences.rs"], int
        )
        lane_runner = (
            ROOT / "scripts/ci/reborn-coverage-lane-run.sh"
        ).read_text(encoding="utf-8")
        self.assertIn("reborn_(integration_|generated_)", lane_runner)

    def test_unmapped_crate_path_fails_closed_to_full_pr_plan(self) -> None:
        plan = self.plan("pull_request", ["crates/deleted/src/lib.rs"])
        self.assertEqual(plan["mode"], "full")

    def test_unclassified_build_input_fails_closed_to_full_pr_plan(self) -> None:
        plan = self.plan("pull_request", ["Dockerfile"])
        self.assertEqual(plan["mode"], "full")

    def test_changed_integration_binary_selects_its_exact_lane(self) -> None:
        path, lane = next(iter(planner._integration_test_lanes().items()))
        plan = self.plan("pull_request", [path])
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["integration_lanes"], [lane])

    def test_workflow_consumes_plan_and_bounds_each_rust_matrix(self) -> None:
        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("python3 scripts/ci/reborn_pr_test_plan.py", workflow)
        self.assertIn("scripts/ci/discover-reborn-package-crates.sh", workflow)
        self.assertIn("--canonical-packages", workflow)
        self.assertIn("needs.changes.outputs.crate_buckets", workflow)
        self.assertIn("needs.changes.outputs.root_partitions", workflow)
        self.assertIn("needs.changes.outputs.integration_lanes", workflow)
        self.assertIn(
            '"${feature_args[@]}" --ignore-rust-version --all-targets',
            workflow,
        )
        self.assertIn(
            "max-parallel: ${{ github.event_name == 'pull_request' && 3 || 14 }}",
            workflow,
        )
        self.assertIn(
            "max-parallel: ${{ github.event_name == 'pull_request' && 1 || 4 }}",
            workflow,
        )
        self.assertIn(
            "max-parallel: ${{ github.event_name == 'pull_request' && 1 || 5 }}",
            workflow,
        )
        self.assertIn("github.event.merge_group.base_sha", workflow)
        self.assertIn(
            "ran with result '${result}' despite planned=false",
            workflow,
        )
        self.assertIn("Full Reborn plan is not exhaustive", workflow)
        self.assertIn("Full Reborn plan omitted a required lane", workflow)

    def test_reborn_e2e_shards_preserve_all_runtime_and_webui_suites(self) -> None:
        workflow = (ROOT / ".github/workflows/reborn-e2e.yml").read_text(
            encoding="utf-8"
        )
        rust_gate = (ROOT / "scripts/reborn-e2e-rust.sh").read_text(
            encoding="utf-8"
        )

        for group in (
            "architecture-boundaries",
            "architecture-runtime",
            "runtimes",
            "substrates",
        ):
            with self.subTest(group=group):
                self.assertIn(f"          - {group}", workflow)

        self.assertEqual(
            rust_gate.count(
                "gateway_maps_deterministic_provider_response_errors_to_invalid_output"
            ),
            1,
        )
        self.assertEqual(
            rust_gate.count(
                "deterministic_provider_response_errors_use_bounded_invalid_output_recovery"
            ),
            1,
        )
        self.assertEqual(
            rust_gate.count(
                "process_projection::runtime::tests::retry_rejects_checkpoint_rejection_without_creating_a_process"
            ),
            1,
        )
        self.assertNotIn("projection::tests::nested_dispatch_stream", rust_gate)

        for suite in (
            "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py",
            "tests/e2e/scenarios/test_reborn_webui_v2_sso.py",
            "tests/e2e/scenarios/test_reborn_qa_trace_full_path.py",
            "tests/e2e/scenarios/test_reborn_qa_trace_replay.py",
            "tests/e2e/scenarios/test_reborn_blackbox_smoke.py",
            "tests/e2e/reborn_responses_e2e_tests.txt",
        ):
            with self.subTest(suite=suite):
                self.assertIn(suite, workflow)

        for process in (
            "product_build_pid",
            "evidence_setup_pid",
        ):
            with self.subTest(process=process):
                self.assertIn(f'wait "${{{process}}}" || status=1', workflow)

        self.assertIn('default_binary="${RUNNER_TEMP}/ironclaw-default"', workflow)
        self.assertIn('mv "${default_binary}" "${target_dir}/debug/ironclaw"', workflow)
        self.assertNotIn('--target-dir "${CARGO_TARGET_DIR:-target}/e2e-sso"', workflow)

        for shard in range(4):
            with self.subTest(provider_operation_shard=shard):
                self.assertIn(f'shard: "{shard}/4"', workflow)
        self.assertIn("  webui-v2-test-lanes:", workflow)
        self.assertIn("lane: fast-contracts", workflow)
        self.assertIn("lane: provider-contracts", workflow)
        self.assertNotIn("\n  blackbox-smoke:", workflow)
        self.assertIn("max-parallel: 7", workflow)
        self.assertIn("reborn-webui-v2-sso-binary-${{", workflow)
        self.assertIn("reborn-webui-v2-binary-${{", workflow)
        self.assertIn("touch target/debug/ironclaw", workflow)
        self.assertIn("touch target/e2e-sso/debug/ironclaw", workflow)
        self.assertIn(
            'job_result_ok "webui-v2-test-lanes"',
            workflow,
        )

    def test_main_coverage_cancels_superseded_commits(self) -> None:
        workflow = (ROOT / ".github/workflows/coverage.yml").read_text()
        self.assertIn("group: code-coverage-${{ github.ref }}", workflow)
        self.assertIn("cancel-in-progress: true", workflow)


if __name__ == "__main__":
    unittest.main()
