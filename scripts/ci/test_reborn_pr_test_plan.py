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

    def plan(
        self,
        event: str,
        paths: list[str],
        *,
        lockfile_manifest_owned: bool = False,
    ) -> dict:
        return planner.build_plan(
            event=event,
            changed_paths=paths,
            metadata=metadata(),
            canonical_packages=self.canonical,
            lockfile_manifest_owned=lockfile_manifest_owned,
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

    def test_package_owned_test_change_does_not_run_reverse_dependents(self) -> None:
        plan = self.plan("pull_request", ["crates/alpha/tests/contract.rs"])
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["changed_packages"], ["alpha"])
        self.assertEqual(plan["affected_packages"], ["alpha"])
        self.assertEqual(
            plan["crate_buckets"],
            [
                {
                    "name": "selected",
                    "packages": ["alpha"],
                    "exact_targets": [
                        {"package": "alpha", "kind": "test", "name": "contract"}
                    ],
                }
            ],
        )

    def test_nested_package_test_change_keeps_all_owning_package_targets(self) -> None:
        plan = self.plan("pull_request", ["crates/alpha/tests/support/mod.rs"])
        self.assertEqual(plan["affected_packages"], ["alpha"])
        self.assertNotIn("exact_targets", plan["crate_buckets"][0])

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

    def test_frontend_change_is_owned_by_code_style_with_baseline_qa_replay(self) -> None:
        plan = self.plan(
            "pull_request", ["crates/ironclaw_webui/frontend/src/app.tsx"]
        )
        self.assertEqual(plan["mode"], "none")
        self.assertNotIn("run_frontend", plan)
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

    def test_live_qa_harness_changes_run_only_qa_replay(self) -> None:
        for path in (
            "scripts/live-canary/README.md",
            "scripts/live-canary/notify_slack.py",
            "scripts/reborn_webui_v2_live_qa/run_live_qa.py",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "selected")
                self.assertTrue(plan["run_qa_replay"])
                self.assertEqual(plan["crate_buckets"], [])
                self.assertEqual(plan["root_partitions"], [])
                self.assertEqual(plan["integration_lanes"], [])

    def test_unrelated_workflow_change_runs_only_baseline_qa_replay(self) -> None:
        plan = self.plan("pull_request", [".github/workflows/code_style.yml"])
        self.assertEqual(plan["mode"], "none")
        self.assertTrue(plan["run_qa_replay"])

    def test_reborn_workflow_change_is_owned_by_static_and_merge_queue_gates(self) -> None:
        plan = self.plan("pull_request", [".github/workflows/reborn-tests.yml"])
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["root_partitions"], [])
        self.assertEqual(plan["integration_lanes"], [])
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["coverage_mode"], "none")

    def test_empty_diff_fails_fast(self) -> None:
        with self.assertRaisesRegex(ValueError, "empty pull-request diff"):
            self.plan("pull_request", [])

    def test_reborn_caller_workflow_is_owned_by_static_and_merge_queue_gates(self) -> None:
        plan = self.plan("pull_request", [".github/workflows/nightly-deep-ci.yml"])
        self.assertEqual(plan["mode"], "none")

    def test_coverage_policy_change_is_statically_validated_on_pr(self) -> None:
        plan = self.plan(
            "pull_request", ["tests/integration/coverage-floor.toml"]
        )
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["coverage_mode"], "none")

    def test_lockfile_with_changed_crate_manifest_uses_affected_packages(self) -> None:
        plan = self.plan(
            "pull_request",
            ["Cargo.lock", "crates/alpha/Cargo.toml"],
            lockfile_manifest_owned=True,
        )
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["changed_packages"], ["alpha"])
        self.assertEqual(plan["affected_packages"], ["alpha", "beta", "gamma"])

    def test_lockfile_without_changed_crate_manifest_defers_breadth_to_queue(self) -> None:
        plan = self.plan("pull_request", ["Cargo.lock"])
        self.assertEqual(plan["mode"], "none")

    def test_unowned_lockfile_change_still_runs_changed_manifest_closure(self) -> None:
        plan = self.plan(
            "pull_request", ["Cargo.lock", "crates/alpha/Cargo.toml"]
        )
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["affected_packages"], ["alpha", "beta", "gamma"])

    def test_lockfile_ownership_accepts_only_changed_workspace_dependency_edges(self) -> None:
        base = {
            "version": 4,
            "package": [
                {"name": "alpha", "version": "0.1.0", "dependencies": ["serde"]},
                {
                    "name": "serde",
                    "version": "1.0.0",
                    "source": "registry",
                    "checksum": "same",
                },
            ],
        }
        current = {
            **base,
            "package": [
                {
                    "name": "alpha",
                    "version": "0.1.0",
                    "dependencies": ["serde", "tempfile"],
                },
                base["package"][1],
            ],
        }
        self.assertTrue(
            planner._lockfile_change_is_manifest_owned(
                current=current,
                base=base,
                changed_paths={"crates/alpha/Cargo.toml"},
                metadata=metadata(),
            )
        )

        current["package"][1] = {**base["package"][1], "checksum": "changed"}
        self.assertFalse(
            planner._lockfile_change_is_manifest_owned(
                current=current,
                base=base,
                changed_paths={"crates/alpha/Cargo.toml"},
                metadata=metadata(),
            )
        )

    def test_lockfile_ownership_rejects_package_additions_and_removals(self) -> None:
        base = {
            "version": 4,
            "package": [
                {"name": "alpha", "version": "0.1.0", "dependencies": []},
            ],
        }
        added = {
            "version": 4,
            "package": [
                {
                    "name": "alpha",
                    "version": "0.1.0",
                    "dependencies": ["tempfile"],
                },
                {
                    "name": "tempfile",
                    "version": "3.0.0",
                    "source": "registry",
                    "checksum": "x",
                },
            ],
        }
        for current, previous in ((added, base), (base, added)):
            with self.subTest(current=current):
                self.assertFalse(
                    planner._lockfile_change_is_manifest_owned(
                        current=current,
                        base=previous,
                        changed_paths={"crates/alpha/Cargo.toml"},
                        metadata=metadata(),
                    )
                )

    def test_stress_tool_is_owned_by_dedicated_workflow(self) -> None:
        plan = self.plan(
            "pull_request", ["tools/ironclaw_stress/src/main.rs"]
        )
        self.assertEqual(plan["mode"], "none")
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["integration_lanes"], [])

    def test_changed_coverage_manifest_does_not_launch_integration_lanes(self) -> None:
        plan = self.plan(
            "pull_request",
            ["tests/integration/changed-coverage-exemptions.toml"],
        )
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["integration_lanes"], [])
        self.assertEqual(plan["coverage_mode"], "none")

    def test_noncanonical_package_fails_fast(self) -> None:
        with self.assertRaisesRegex(ValueError, "outside the canonical"):
            planner.build_plan(
                event="pull_request",
                changed_paths=["crates/gamma/src/lib.rs"],
                metadata=metadata(),
                canonical_packages=["alpha", "beta"],
            )

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

    def test_unmapped_crate_path_fails_fast(self) -> None:
        with self.assertRaisesRegex(ValueError, "unmapped crate path"):
            self.plan("pull_request", ["crates/deleted/src/lib.rs"])

    def test_unclassified_build_input_fails_fast(self) -> None:
        with self.assertRaisesRegex(ValueError, "unclassified pull-request path"):
            self.plan("pull_request", ["Dockerfile"])

    def test_changed_integration_binary_selects_its_exact_lane(self) -> None:
        path, lane = next(iter(planner._integration_test_lanes().items()))
        plan = self.plan("pull_request", [path])
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["integration_lanes"], [lane])

    def test_shared_test_support_uses_representative_pr_lanes(self) -> None:
        root_plan = self.plan(
            "pull_request", ["tests/support/reborn_parity_qa/assertions.rs"]
        )
        integration_plan = self.plan(
            "pull_request", ["tests/integration/support/database.rs"]
        )
        self.assertEqual(root_plan["root_partitions"], [0])
        self.assertEqual(integration_plan["integration_lanes"], [0])

    def test_workspace_topology_change_defers_exhaustive_matrix_to_queue(self) -> None:
        plan = self.plan("pull_request", ["Cargo.toml"])
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["crate_buckets"], [])
        self.assertTrue(plan["run_qa_replay"])

    def test_workflow_consumes_plan_and_bounds_each_rust_matrix(self) -> None:
        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("python3 scripts/ci/reborn_pr_test_plan.py", workflow)
        self.assertIn("scripts/ci/discover-reborn-package-crates.sh", workflow)
        self.assertIn("--canonical-packages", workflow)
        self.assertIn('--base-sha "$BASE_SHA"', workflow)
        self.assertIn("needs.changes.outputs.crate_buckets", workflow)
        self.assertIn("needs.changes.outputs.root_partitions", workflow)
        self.assertIn("needs.changes.outputs.integration_lanes", workflow)
        self.assertIn(
            '"${feature_args[@]}" --ignore-rust-version --all-targets',
            workflow,
        )
        self.assertIn(
            'cargo llvm-cov --branch --skip-functions \\\n'
            '                "${package_args[@]}" "${feature_args[@]}"',
            workflow,
        )
        self.assertNotIn('coverage/${package}.lcov', workflow)
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
        self.assertIn(
            "github.event_name == 'merge_group' && "
            "steps.scope.outputs.should_run == 'true'",
            workflow,
        )
        self.assertIn("scripts/ci/test-critical-mutation-gate.sh", workflow)
        self.assertIn("if: github.event_name == 'merge_group'", workflow)
        self.assertIn(
            "mutation_expected=${{ github.event_name == 'merge_group' }}",
            workflow,
        )

        code_style = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "python3 scripts/ci/changed_workspace_packages.py",
            code_style,
        )
        self.assertIn('package_args+=(-p "${package}")', code_style)
        self.assertIn("needs.changes.outputs.has_clippy == 'true'", code_style)
        self.assertIn(
            "cargo clippy --all --tests --examples ${{ matrix.flags }} -- -D warnings",
            code_style,
        )
        self.assertIn(
            "${{ toJSON(matrix.bucket.exact_targets || fromJSON('[]')) }}",
            workflow,
        )
        self.assertIn(
            '"${incremental_env[@]}" cargo test \\\n'
            '                    -p "${package}" "--${kind}" "${name}"',
            workflow,
        )

    def test_pr_workflows_do_not_repeat_reborn_rust_contracts(self) -> None:
        code_style = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )
        e2e = (ROOT / ".github/workflows/reborn-e2e.yml").read_text(
            encoding="utf-8"
        )
        reborn_tests = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )
        regression = (
            ROOT / ".github/workflows/regression-test-check.yml"
        ).read_text(encoding="utf-8")

        self.assertIn(
            "needs.changes.outputs.has_reborn_cli == 'true' && "
            "github.event_name != 'pull_request'",
            code_style,
        )
        self.assertIn(
            "needs.changes.outputs.has_e2e_scope == 'true' && "
            "github.event_name != 'pull_request'",
            e2e,
        )
        self.assertIn("rust_reborn_optional=true", e2e)
        self.assertIn("--validate-manifest-only", code_style)
        self.assertIn("pnpm test", code_style)
        self.assertIn("pnpm build", code_style)
        self.assertNotIn("webui-v2-js-tests:", reborn_tests)
        self.assertIn(
            "github.event.review.state == 'commented' && 'commented' || 'enforcing'",
            regression,
        )

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

        self.assertIn('shard: "0/4 1/4"', workflow)
        self.assertIn('shard: "2/4 3/4"', workflow)
        self.assertIn('for provider_shard in "${provider_shards[@]}"', workflow)
        self.assertIn(
            'IRONCLAW_PROVIDER_OPERATION_SHARD="${provider_shard}"',
            workflow,
        )
        self.assertIn('provider_pids+=("$!")', workflow)
        self.assertIn('wait "${provider_pids[$index]}"', workflow)
        self.assertIn('> "${provider_log}" 2>&1 &', workflow)
        self.assertIn("  webui-v2-test-lanes:", workflow)
        self.assertNotIn("lane: fast-contracts", workflow)
        self.assertIn("lane: provider-contracts", workflow)
        self.assertNotIn("\n  blackbox-smoke:", workflow)
        self.assertIn("max-parallel: 4", workflow)
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
