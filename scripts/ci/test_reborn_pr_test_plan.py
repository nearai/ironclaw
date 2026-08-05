#!/usr/bin/env python3
"""Contract tests for affected-area Reborn PR test planning."""

from __future__ import annotations

import importlib.util
import re
import sys
import tomllib
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/ci/reborn_pr_test_plan.py"
SPEC = importlib.util.spec_from_file_location("reborn_pr_test_plan", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
planner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = planner
SPEC.loader.exec_module(planner)

sys.path.insert(0, str(ROOT / "scripts/ci/lib"))

from crate_tree import crate_directories, owning_crate_directory  # noqa: E402

# A literal `include_str!`/`include_bytes!` target. `concat!` forms are not
# matched and do not need to be: this resolves *whether* a crate reaches into
# an asset tree, and every tree in the table has literal sites.
INCLUDE_LITERAL = re.compile(r"include_(?:str|bytes)!\s*\(\s*\"([^\"]+)\"")
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def _workspace_crate_directories() -> dict[str, Path]:
    """Package name -> crate directory, from the repo's own crate inventory.

    `crate_tree` rather than a `crates/**/Cargo.toml` glob so the workspace-
    excluded `wasm-src/` guests stay out: they declare their own `[workspace]`,
    no lane of this workspace compiles them, and counting them would make a
    guest's include of its own sibling file look like a cross-tree reach-in.
    """
    directories: dict[str, Path] = {}
    for relative in crate_directories(ROOT):
        directory = ROOT / relative
        manifest = tomllib.loads(
            (directory / "Cargo.toml").read_text(encoding="utf-8")
        )
        name = manifest.get("package", {}).get("name")
        if name:
            directories[name] = directory
    return directories


def _crates_embedding(prefix: str, crate_dirs: dict[str, Path]) -> set[str]:
    """Crates with an include of a *table-routed* file under `prefix`.

    Table-routed means "owned by no crate": the planner resolves package
    directories first, so `crates/extensions/packages/telegram/manifest.toml`
    is its own crate's file and never reaches `EMBEDDED_ASSET_OWNERS`.
    """
    embedders: set[str] = set()
    for name, directory in crate_dirs.items():
        for source in directory.rglob("*.rs"):
            if "target" in source.parts or name in embedders:
                continue
            text = source.read_text(encoding="utf-8", errors="replace")
            for literal in INCLUDE_LITERAL.findall(text):
                try:
                    target = (
                        (source.parent / literal).resolve().relative_to(ROOT).as_posix()
                    )
                except ValueError:
                    continue
                if target.startswith(prefix) and (
                    owning_crate_directory(target, ROOT) is None
                ):
                    embedders.add(name)
                    break
    return embedders


def _depends_on(package: str, target: str, crate_dirs: dict[str, Path]) -> bool:
    """True when `package` reaches `target` through workspace dependencies."""
    seen = {package}
    pending = [package]
    while pending:
        directory = crate_dirs.get(pending.pop())
        if directory is None:
            continue
        manifest = tomllib.loads(
            (directory / "Cargo.toml").read_text(encoding="utf-8")
        )
        tables = [manifest.get(table, {}) for table in DEPENDENCY_TABLES]
        for platform in manifest.get("target", {}).values():
            tables.extend(platform.get(table, {}) for table in DEPENDENCY_TABLES)
        for table in tables:
            for key, value in table.items():
                name = value.get("package", key) if isinstance(value, dict) else key
                if name == target:
                    return True
                if name in crate_dirs and name not in seen:
                    seen.add(name)
                    pending.append(name)
    return False


def metadata() -> dict:
    root = str(ROOT / "Cargo.toml")
    alpha = str(ROOT / "crates/alpha/Cargo.toml")
    beta = str(ROOT / "crates/beta/Cargo.toml")
    gamma = str(ROOT / "crates/gamma/Cargo.toml")
    return {
        "workspace_members": ["root", "alpha", "beta", "gamma"],
        "packages": [
            {"id": "root", "name": "ironclaw_integration_tests", "manifest_path": root},
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


def real_owner_metadata() -> dict:
    """A workspace named after the crates `EMBEDDED_ASSET_OWNERS` routes to.

    The rest of this file uses `metadata()`'s `alpha`/`beta`/`gamma`, which
    cannot carry the real table: the planner rejects a changed package outside
    the canonical set, so the real owners have to exist here to be routed to
    at all. Manifest paths are the real ones, so `_workspace_packages` derives
    the same package directories it does in CI — which is what makes
    `crates/extensions/packages/slack/` resolve to its own crate rather than
    to the asset table.

    `ironclaw_extension_host` depends on `ironclaw_extension_support` (an
    optional dependency plus a dev-dependency, both in its real manifest),
    which is why routing a package asset to the support crate also schedules
    the host that embeds three of those manifests itself.
    """

    def package(name: str, manifest: str, deps: tuple[str, ...] = ()) -> dict:
        return {
            "id": name,
            "name": name,
            "manifest_path": str(ROOT / manifest),
            "deps": deps,
        }

    packages = [
        package("ironclaw_integration_tests", "Cargo.toml"),
        package(
            "ironclaw_extension_support",
            "crates/extensions/ironclaw_extension_support/Cargo.toml",
        ),
        package(
            "ironclaw_extension_host",
            "crates/ironclaw_extension_host/Cargo.toml",
            ("ironclaw_extension_support",),
        ),
        package(
            "ironclaw_slack_extension",
            "crates/extensions/packages/slack/Cargo.toml",
        ),
    ]
    return {
        "workspace_members": [entry["id"] for entry in packages],
        "packages": [
            {key: value for key, value in entry.items() if key != "deps"}
            for entry in packages
        ],
        "resolve": {
            "nodes": [
                {"id": entry["id"], "deps": [{"pkg": dep} for dep in entry["deps"]]}
                for entry in packages
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

    def plan_real_owners(self, paths: list[str]) -> dict:
        """Plan a pull request through the real `EMBEDDED_ASSET_OWNERS`."""
        metadata = real_owner_metadata()
        return planner.build_plan(
            event="pull_request",
            changed_paths=paths,
            metadata=metadata,
            canonical_packages=[
                package["name"]
                for package in metadata["packages"]
                if package["name"] != "ironclaw_integration_tests"
            ],
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
        # The frontend prefix is resolved through the crate inventory
        # (scripts/ci/lib/crate_tree.py), not a hardcoded literal — deriving
        # the test input from the same resolver keeps this correct regardless
        # of whether ironclaw_webui sits flat or has already moved into a
        # family directory (PROPOSAL §5). Sibling tests below pin the
        # resolution mechanism itself (nested + fail-closed) against a mocked
        # crate_directory rather than the live tree.
        frontend_prefix = planner._webui_frontend_prefix()
        plan = self.plan("pull_request", [f"{frontend_prefix}src/app.tsx"])
        self.assertEqual(plan["mode"], "none")
        self.assertNotIn("run_frontend", plan)
        self.assertTrue(plan["run_qa_replay"])
        self.assertEqual(plan["crate_buckets"], [])
        self.assertEqual(plan["integration_lanes"], [])

    def test_frontend_prefix_resolves_through_crate_inventory_when_nested(self) -> None:
        """WS10: a family-moved ironclaw_webui still routes to Code Style.

        Mocks `crate_directory` (rather than relying on the live repo's
        current crate layout, which the target-architecture restructure is
        actively changing) to pin that the frontend-prefix resolution follows
        the crate wherever it lives.
        """
        planner._webui_frontend_prefix.cache_clear()
        try:
            with mock.patch.object(
                planner,
                "crate_directory",
                return_value="crates/substrates/ironclaw_webui",
            ) as resolver:
                plan = self.plan(
                    "pull_request",
                    ["crates/substrates/ironclaw_webui/frontend/src/app.tsx"],
                )
            resolver.assert_called_once_with("ironclaw_webui", planner.ROOT)
            self.assertEqual(plan["mode"], "none")
            self.assertEqual(plan["crate_buckets"], [])
            self.assertEqual(plan["integration_lanes"], [])
        finally:
            planner._webui_frontend_prefix.cache_clear()

    def test_frontend_prefix_resolution_failure_fails_closed(self) -> None:
        """An unresolvable ironclaw_webui crate must raise, never fall back
        to the literal — a silent fallback is exactly the WS10 failure mode
        (a moved crate makes the prefix match nothing and the planner reports
        "no Reborn test surface changed" for a real WebUI diff)."""
        planner._webui_frontend_prefix.cache_clear()
        try:
            with mock.patch.object(
                planner,
                "crate_directory",
                side_effect=planner.CrateTreeError("boom"),
            ):
                with self.assertRaisesRegex(
                    RuntimeError, "cannot resolve the ironclaw_webui crate"
                ):
                    self.plan(
                        "pull_request",
                        ["crates/ironclaw_webui/frontend/src/app.tsx"],
                    )
        finally:
            planner._webui_frontend_prefix.cache_clear()

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

    def test_reborn_e2e_scenario_change_is_owned_by_e2e_workflow(self) -> None:
        plan = self.plan(
            "pull_request",
            ["tests/e2e/scenarios/test_reborn_webui_v2_legacy_extensions.py"],
        )
        # E2E scenarios live in the dedicated reborn-e2e.yml workflow, not
        # the crate-bucket / root-partition / integration-lane plan emitted
        # here. A scenario-only change must not fail closed as an unmapped
        # path, and must not schedule crate buckets or integration lanes.
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["crate_buckets"], [])
        self.assertEqual(plan["integration_lanes"], [])
        self.assertEqual(plan["root_partitions"], [])
        self.assertTrue(plan["run_qa_replay"])
        self.assertTrue(
            any("Reborn E2E workflow owns" in reason for reason in plan["reasons"]),
            plan["reasons"],
        )

    def test_shared_e2e_harness_is_owned_by_e2e_workflow(self) -> None:
        plan = self.plan("pull_request", ["tests/e2e/reborn_webui_harness.py"])
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["integration_lanes"], [])

    def test_reborn_e2e_and_crate_changes_keep_both_owners(self) -> None:
        plan = self.plan(
            "pull_request",
            [
                "tests/e2e/scenarios/test_reborn_webui_v2_legacy_extensions.py",
                "crates/alpha/src/lib.rs",
            ],
        )
        # The E2E scenario path is skipped (owned by reborn-e2e.yml) while
        # the changed crate is still scheduled in the affected crate buckets.
        self.assertEqual(plan["mode"], "selected")
        self.assertEqual(plan["changed_packages"], ["alpha"])
        self.assertTrue(plan["crate_buckets"])
        self.assertTrue(plan["run_qa_replay"])
        self.assertTrue(
            any("Reborn E2E workflow owns" in reason for reason in plan["reasons"]),
            plan["reasons"],
        )

    def test_live_qa_harness_changes_run_only_qa_replay(self) -> None:
        for path in (
            "scripts/live-canary/README.md",
            "scripts/live-canary/notify_slack.py",
            "scripts/reborn_webui_v2_live_qa/run_live_qa.py",
            # WS10 (2026-08-04): the QA surface-inventory auditor and its
            # self-test are the same class of offline QA tooling, and were
            # raising `unmapped test or CI path` until they were classified.
            "scripts/reborn_qa_matrix/audit_surface_inventory.py",
            "scripts/reborn_qa_matrix/test_audit_surface_inventory.py",
            # 2026-08-05: `scripts/live_canary/` (UNDERSCORE) is a second real
            # directory beside `scripts/live-canary/` (hyphen) above, and
            # classifying the hyphen tree never classified this one. It is the
            # canary's importable Python package; a crate rename reaches it
            # through a `RUST_LOG` string, and the fail-closed arm rejected the
            # whole rename PR on that one line.
            "scripts/live_canary/common.py",
            "scripts/live_canary/auth_runtime.py",
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

    def test_user_sandbox_worker_change_selects_real_docker_lane(self) -> None:
        for path in (
            "Dockerfile.sandbox-worker",
            "docker/reborn/entrypoint.sh",
            "crates/ironclaw_sandbox/src/sandbox_process.rs",
            "crates/ironclaw_composition/src/builtin_capability_policy.rs",
            "crates/ironclaw_composition/src/deployment.rs",
            "crates/ironclaw_composition/src/factory/production_backend_assembly.rs",
            "crates/ironclaw_composition/src/factory/runtime_lane_assembly.rs",
            "crates/ironclaw_composition/src/input.rs",
            "crates/ironclaw_host_runtime/src/first_party_tools/mod.rs",
            "crates/ironclaw_host_runtime/src/invocation_services.rs",
            "crates/ironclaw_host_runtime/src/process_port.rs",
            "crates/ironclaw_host_runtime/src/services.rs",
            "crates/ironclaw_host_runtime/src/services/builder.rs",
            "crates/ironclaw_runtime_policy/src/planner.rs",
            "crates/ironclaw_runtime_policy/src/resolver.rs",
            "crates/ironclaw_sandbox/tests/user_sandbox_docker_live.rs",
            "tests/integration/reborn_sandbox_shell_turn.rs",
            "tests/e2e_trace_runtime_policy_serde.rs",
            "tests/fixtures/llm_traces/runtime_policy/hosted_dev_no_shell.json",
            "tests/integration/support/builder.rs",
            "tests/integration/support/capability_backend.rs",
            "tests/integration/support/docker_gate.rs",
            "tests/integration/support/harness/mod.rs",
            "tests/integration/support/harness/options.rs",
            "tests/integration/support/harness/profiles/sandbox_shell.rs",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertTrue(plan["run_sandbox_docker"])

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

    def test_out_of_workspace_harness_is_owned_by_dedicated_workflow(self) -> None:
        """`harness/**` is a standalone cargo project, not a workspace member.

        `harness/latency/runner` carries its own `Cargo.lock` and is excluded
        from the workspace, so no `Tests (Reborn)` lane builds it. Without a
        rule the planner fails closed on it, the same shape as the `.claude/`
        and container-input gaps this file already pins.
        """
        plan = self.plan("pull_request", ["harness/latency/runner/src/main.rs"])
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["crate_buckets"], [])
        self.assertEqual(plan["integration_lanes"], [])

    def test_repo_root_metadata_class_is_owned_by_other_lanes(self) -> None:
        """The repo-root metadata class de-escalates instead of failing closed.

        Classified as a *class* rather than one file per red run: a
        rename-shaped diff touches root files no feature PR normally touches,
        the fail-closed arm rejects the first one, and the next only surfaces
        after that one is fixed. Each entry was checked against `crates/**` and
        `tests/**` for a reader before being listed; none has one.

        Paired assertion, as in the `.claude/` regression: accepted AND
        selecting nothing, so a later "classification" that quietly escalates
        these to a full matrix fails here too.
        """
        for path in (
            "clippy.toml",
            "deny.toml",
            "release-plz.toml",
            ".gitattributes",
            ".coderabbit.yaml",
            ".mcp.json",
            ".node-version",
            ".nvmrc",
            ".sqlfluff",
            "Dockerfile.process-sandbox",
            "docker-compose.yml",
            "railway.toml",
            "codecov.yml",
            "ironclaw.bash",
            "ironclaw.fish",
            "ironclaw.zsh",
            "ironclaw.png",
            "LICENSE-APACHE",
            "LICENSE-MIT",
            "scripts/check_no_panics.py",
            "scripts/dev_metrics.py",
            "scripts/pre-commit-safety.sh",
            "scripts/test-mutation-audit.sh",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)
                self.assertEqual(plan["root_partitions"], [], path)
                self.assertEqual(plan["integration_lanes"], [], path)
                # The plan must say *why*, so a future reader sees the
                # decision rather than a silent "nothing to run".
                self.assertTrue(
                    any(
                        reason.startswith("static CI or workspace-policy checks own")
                        and path in reason
                        for reason in plan["reasons"]
                    ),
                    plan["reasons"],
                )

    def test_e2e_paths_are_owned_by_dedicated_workflows(self) -> None:
        for path in (
            "tests/e2e/helpers.py",
            "tests/e2e/reborn_webui_harness.py",
            "tests/e2e/reborn_coverage_tests.txt",
            "tests/e2e/scenarios/test_reborn_webui_v2_projects_api.py",
            "tests/e2e/scenarios/test_reborn_webui_v2_smoke.py",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none")
                self.assertTrue(plan["run_qa_replay"])
                self.assertEqual(plan["integration_lanes"], [])
                self.assertEqual(plan["coverage_mode"], "none")
                self.assertTrue(
                    any(
                        reason.startswith("dedicated Reborn E2E workflow owns")
                        for reason in plan["reasons"]
                    )
                )

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
        self.assertIn(
            'cargo test -p ironclaw_integration_tests "${test_args[@]}" '
            "\\\n      --ignore-rust-version",
            lane_runner,
        )

    def test_selected_integration_lane_keeps_msrv_override(self) -> None:
        lane_runner = (
            ROOT / "scripts/ci/reborn-coverage-lane-run.sh"
        ).read_text(encoding="utf-8")
        self.assertIn(
            'cargo test -p ironclaw_integration_tests "${test_args[@]}" '
            "\\\n      --ignore-rust-version -- --nocapture",
            lane_runner,
        )

    def test_unmapped_crate_path_widens_instead_of_refusing(self) -> None:
        """A crate path with no owning package widens to the exhaustive plan.

        This used to raise. It made every crate deletion or rename unplannable
        — `git diff` reports the removed crate's old paths, and CI feeds the
        planner that diff — which blocked the PR rather than protecting it.
        Widening cannot under-select, so it is the safe resolution; genuinely
        malformed input is still rejected by the unclassified-path branch.
        """
        plan = self.plan("pull_request", ["crates/deleted/src/lib.rs"])
        self.assertEqual(plan["mode"], "full")
        self.assertIn("deletion or rename", plan["reasons"][0])

    def test_unclassified_build_input_fails_fast(self) -> None:
        # Was `Dockerfile` until that gained a decision (see
        # `test_container_and_hook_inputs_are_owned_by_static_gates`). The arm
        # itself must stay fail-closed, so this keeps a genuinely undecided
        # repo-root build input pointed at it.
        with self.assertRaisesRegex(ValueError, "unclassified pull-request path"):
            self.plan("pull_request", ["Makefile"])

    def test_agent_guidance_is_classified_and_selects_no_rust_lane(self) -> None:
        """`.claude/**` is prose, like `docs/**`.

        Regression for the gap #7064 hit: the planner had no rule for
        `.claude/`, so its fail-closed arm rejected any PR editing a skill, a
        command, or a rule — failing the whole `Tests (Reborn)` roll-up on a
        documentation-only change. The assertion is deliberately paired: the
        path must be *accepted* AND must select no Rust lane, so a future
        "classification" that quietly turns guidance edits into a full matrix
        fails here too.
        """
        for path in (
            ".claude/commands/trace.md",
            ".claude/rules/testing.md",
            ".claude/skills/reborn-feature/SKILL.md",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)
                self.assertEqual(plan["root_partitions"], [], path)
                self.assertEqual(plan["integration_lanes"], [], path)

    def test_codebase_memory_artifacts_select_no_rust_lane(self) -> None:
        """Shared agent graph data has no Reborn product or test surface."""
        for path in (
            ".codebase-memory/.gitattributes",
            ".codebase-memory/graph.db.zst",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)
                self.assertEqual(plan["root_partitions"], [], path)
                self.assertEqual(plan["integration_lanes"], [], path)

                paired = self.plan(
                    "pull_request", [path, "crates/alpha/src/lib.rs"]
                )
                self.assertEqual(paired["mode"], "selected", path)
                self.assertNotEqual(paired["crate_buckets"], [], path)

    def test_repo_wide_test_guidance_selects_no_rust_lane(self) -> None:
        for path in ("tests/CLAUDE.md", "tests/integration/CLAUDE.md"):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none")
                self.assertEqual(plan["crate_buckets"], [])
                self.assertEqual(plan["root_partitions"], [])
                self.assertEqual(plan["integration_lanes"], [])

    def test_repo_root_example_env_is_classified_and_selects_no_rust_lane(self) -> None:
        """`.env.example` is documentation, like a repo-root `*.md`.

        Regression for the gap PR #7117 hit: root `*.md` was classified but
        its non-`.md` sibling was not, so correcting an env-var comment failed
        the whole `Tests (Reborn)` roll-up. Nothing reads the file — no crate,
        test, or workflow — only doc comments name it.

        Paired assertions, same reason as the `.claude/` test above: the path
        must be *accepted* AND select no Rust lane, so a future
        "classification" that turns a comment fix into a full matrix fails
        here too.
        """
        plan = self.plan("pull_request", [".env.example"])
        self.assertEqual(plan["mode"], "none")
        self.assertEqual(plan["crate_buckets"], [])
        self.assertEqual(plan["root_partitions"], [])
        self.assertEqual(plan["integration_lanes"], [])

        # The ignore is per-path, not per-PR: a real change riding along still
        # selects its lane.
        paired = self.plan(
            "pull_request", [".env.example", "crates/alpha/src/lib.rs"]
        )
        self.assertEqual(paired["mode"], "selected")
        self.assertNotEqual(paired["crate_buckets"], [])

        # And the fail-closed arm still catches a genuinely unknown root file.
        with self.assertRaisesRegex(ValueError, "unclassified pull-request path"):
            self.plan("pull_request", [".env.local"])

    def test_decided_repo_root_paths_are_owned_by_other_workflows(self) -> None:
        """Repo-root files another workflow owns outright.

        The `unmapped test or CI path` arm deliberately refuses `scripts/**`
        outside `scripts/ci/` so each file gets a decision rather than a
        blanket prefix. Each of these has one, recorded beside the constant:
        the panic baseline belongs to Code Style, the E2E selector script
        belongs to the `Reborn E2E` workflow's own scope detector,
        `check-version-bumps.sh` is invoked only by `platform-and-compat.yml`,
        `run-reborn-webui.sh` is a local launcher no workflow references, and
        `.gitignore` is read by Code Style's `Reject tracked files that match
        .gitignore` guard (#6965 — unclassified until 2026-08-04, which failed
        the whole `Tests (Reborn)` roll-up on any PR that added an ignore
        rule). None selects a lane in *this* planner — but the sibling that has no
        decision must still refuse, which the second half asserts.

        The last two were added by WS10 (2026-08-04) after editing
        `check-version-bumps.sh` failed `Detect Reborn test scope` outright and
        skipped every downstream Reborn lane — the same fail-closed-with-no-rule
        shape as the `.claude/` gap the CHECKLIST row already records.
        """
        for path in (
            "scripts/no_panics_reborn_baseline.txt",
            "scripts/reborn-e2e-rust.sh",
            "scripts/check-version-bumps.sh",
            "scripts/run-reborn-webui.sh",
            "scripts/codebase-graph.sh",
            ".gitignore",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)
                self.assertEqual(plan["root_partitions"], [], path)
                self.assertEqual(plan["integration_lanes"], [], path)
                # The plan must say *why*, so a future reader sees the
                # decision rather than a silent "nothing to run".
                self.assertTrue(
                    any(
                        reason.startswith("static CI or workspace-policy checks own")
                        and path in reason
                        for reason in plan["reasons"]
                    ),
                    plan["reasons"],
                )

                # The decision is per-path, not per-PR: a real change riding
                # along still selects its lane.
                paired = self.plan("pull_request", [path, "crates/alpha/src/lib.rs"])
                self.assertEqual(paired["mode"], "selected", path)
                self.assertNotEqual(paired["crate_buckets"], [], path)

        with self.assertRaisesRegex(ValueError, "unmapped test or CI path"):
            self.plan("pull_request", ["scripts/some-undecided-helper.sh"])

        # The two WASM build/ABI scripts are decided the same way: named in
        # `platform-and-compat.yml`'s `has_direct_wasm_abi_risk` classifier,
        # which scopes *and* runs them.
        for path in (
            "scripts/build-wasm-extensions.sh",
            "scripts/check-version-bumps.sh",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)

    def test_container_and_hook_inputs_are_owned_by_static_gates(self) -> None:
        """`Dockerfile`, `.dockerignore` and `.githooks/**` select no Rust lane.

        Regression for the #7087 gap, the same class #7064 fixed for
        `.claude/`: the planner had no rule for the container build inputs or
        the git hooks, so its fail-closed arm rejected any PR that touched
        them — which made a `Dockerfile` edit unmergeable even when the edit
        was required (Wave 3's `wit/` move had to drop a `COPY wit/ wit/` that
        no longer resolved, #7084). The image build belongs to
        `platform-and-compat.yml`'s `has_docker_risk` lane and the hooks to
        Code Style; no Reborn Rust lane builds an image or runs a hook.

        Paired assertion, as in the `.claude/` regression: accepted AND
        selecting nothing, so a later "classification" that quietly escalates
        these to a full matrix fails here too.
        """
        for path in (
            "Dockerfile",
            ".dockerignore",
            ".githooks/pre-commit",
            ".githooks/commit-msg",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)
                self.assertEqual(plan["root_partitions"], [], path)
                self.assertEqual(plan["integration_lanes"], [], path)

    def test_embedded_package_assets_schedule_the_crate_that_compiles_them(
        self,
    ) -> None:
        """Asset trees outside every crate root route to their embedding crate.

        The other half of the #7087 gap. `crates/extensions/packages/<pkg>/`
        and `test-tools/<tool>/` hold no `Cargo.toml`, so cargo's package
        directories cannot see them and the `crates/` arm failed closed.

        Classifying them as *ignored* would have been wrong in the dangerous
        direction: a `wasm/*.wasm` under `crates/extensions/packages/` is a
        shipped artifact that `ironclaw_extension_support` embeds with
        `include_bytes!`, so ignoring it converts a loud failure into a silent
        under-schedule of a change to production output. Hence the assertion
        below is that the path *selects a lane*, not merely that it is
        accepted — the inverse of the `.claude/` prose test.

        Driven through the REAL `EMBEDDED_ASSET_OWNERS`, against a workspace
        whose packages carry the real owners' names and real manifest paths.
        The first cut substituted `alpha`/`beta` owners so it could reuse the
        synthetic workspace, which exercised the prefix strings but left the
        prefix→owner *pairing* — the table's entire semantic content —
        asserted nowhere: swapping the two owners passed. Review caught it
        (#7084). `test_embedded_asset_owner_mapping_is_not_stale` supplies the
        other half, deriving the same pairing from the real `include_*!` sites
        so agreeing with a wrong constant is not enough.
        """
        for path, owner in (
            (
                "crates/extensions/packages/github/wasm/github_tool.wasm",
                "ironclaw_extension_support",
            ),
            (
                "crates/extensions/packages/github/wasm-src/src/lib.rs",
                "ironclaw_extension_support",
            ),
            (
                "crates/extensions/packages/gmail/manifest.toml",
                "ironclaw_extension_support",
            ),
            ("test-tools/market-data/manifest.toml", "ironclaw_extension_host"),
            (
                "test-tools/hacker-news/wasm-src/src/lib.rs",
                "ironclaw_extension_host",
            ),
        ):
            with self.subTest(path=path):
                plan = self.plan_real_owners([path])
                self.assertEqual(plan["mode"], "selected", path)
                self.assertEqual(plan["changed_packages"], [owner], path)
                # Routed as a *production* change, so the crates that
                # consume the embedded artifact run too.
                self.assertIn(owner, plan["affected_packages"], path)
                self.assertNotEqual(plan["crate_buckets"], [], path)

        # `ironclaw_extension_host` embeds package manifests too, and is
        # covered because it depends on the routed owner — the property
        # `test_embedded_asset_owner_mapping_is_not_stale` derives from the
        # tree rather than assuming.
        plan = self.plan_real_owners(["crates/extensions/packages/gmail/manifest.toml"])
        self.assertIn("ironclaw_extension_host", plan["affected_packages"])

        # A package that *is* a workspace crate still resolves to itself
        # rather than falling through to the asset table, even though its
        # path sits under a table prefix.
        plan = self.plan_real_owners(["crates/extensions/packages/slack/src/lib.rs"])
        self.assertEqual(plan["changed_packages"], ["ironclaw_slack_extension"])

        # The arm never goes quiet for an asset tree with no owner. This
        # raised `unmapped crate path` until #7065 replaced that fallback
        # with the exhaustive plan; `full` is a superset of any narrowing, so
        # the property pinned here — an unattributable asset path can never
        # *under*-schedule — is now carried by the mode rather than a raise.
        # It must never resolve to `none`, which is what "prose" would mean.
        unowned = self.plan_real_owners(["crates/extensions/nowhere/thing.bin"])
        self.assertEqual(unowned["mode"], "full")

    def test_package_prompt_markdown_routes_to_its_compiler_not_to_prose(self) -> None:
        """A shipped `.md` asset is owned by the asset table, not prose.

        Regression for the ordering defect found in review of #7141. The
        Markdown prose carve-out ran *before* `EMBEDDED_ASSET_OWNERS`, and a
        prompt is a `.md` file that no package *directory* owns — so a change
        to `packages/*/prompts/**.md`, which the asset table explicitly claims
        ("manifests, prompts, schemas and built `wasm/*.wasm`") and which
        `ironclaw_extension_support` compiles in, planned `mode=none` and
        selected no lane at all. Its sibling `manifest.toml` in the same
        package selected two. That is the exact "silent under-schedule of a
        change to production output" the comment above the table forbids.

        Both halves are pinned, because fixing this by making *all* crate-tree
        `.md` route somewhere would be the opposite error.
        """
        prompt = "crates/extensions/packages/github/prompts/github/create_issue.md"
        plan = self.plan_real_owners([prompt])
        self.assertEqual(plan["mode"], "selected", prompt)
        self.assertIn("ironclaw_extension_support", plan["affected_packages"])
        self.assertIn(
            f"asset compiled into ironclaw_extension_support changed: {prompt}",
            plan["reasons"],
        )

        # The prompt and the manifest beside it must agree — the defect was
        # that they disagreed.
        manifest = self.plan_real_owners(
            ["crates/extensions/packages/github/manifest.toml"]
        )
        self.assertEqual(manifest["mode"], plan["mode"])

        # The `test-tools/` prompts are assets on the same rule, routed to the
        # crate that embeds that tree.
        fixture_prompt = "test-tools/hacker-news/prompts/hacker-news/top_stories.md"
        fixture = self.plan_real_owners([fixture_prompt])
        self.assertEqual(fixture["mode"], "selected", fixture_prompt)
        self.assertIn("ironclaw_extension_host", fixture["affected_packages"])

        # And the carve-out still carves. Markdown that is *not* a prompt stays
        # prose even inside an asset tree — this is why the rule is keyed on the
        # `prompts/` segment and not on the asset prefixes, which also cover
        # documentation.
        for prose in (
            "crates/AGENTS.md",
            "crates/extensions/AGENTS.md",
            "test-tools/README.md",
        ):
            with self.subTest(prose=prose):
                quiet = self.plan_real_owners([prose])
                self.assertEqual(quiet["mode"], "none", prose)
                self.assertEqual(quiet["crate_buckets"], [], prose)

    def test_markdown_owned_by_no_crate_is_prose(self) -> None:
        """`crates/AGENTS.md` and `test-tools/README.md` select no lane.

        Nothing compiles a markdown file, so a doc that belongs to no crate is
        prose in the same class as `docs/` and `.claude/`. The rule is keyed
        on "no package owns this path", not on a literal, so it keeps holding
        for a future `crates/<family>/AGENTS.md` after the WS7 family move.

        The paired case is `test_nested_crate_markdown_remains_package_owned`:
        a doc *inside* a crate still resolves to that crate and keeps
        selecting its lane. This must not widen into that.
        """
        for path in ("crates/AGENTS.md", "test-tools/README.md"):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                self.assertEqual(plan["mode"], "none", path)
                self.assertEqual(plan["crate_buckets"], [], path)

    def test_embedded_asset_owner_mapping_is_not_stale(self) -> None:
        """Every prefix routes to a crate that really compiles the tree in.

        The mapping is a hand-written bridge across a boundary cargo cannot
        see, so it is exactly the kind of path-keyed constant CHECKLIST WS10
        found silently rotting: if an asset tree or its owning crate moves,
        the planner would resume failing closed (loud) or, worse, route to a
        crate that no longer exists. Fail here first instead.

        Existence is necessary and nowhere near sufficient, which is what
        review caught on #7084: "the prefix is a directory and the owner is a
        crate" also holds for a *wrong* owner, so the pairing has to come out
        of the tree. It does, from the `include_str!`/`include_bytes!` sites
        themselves. Routing a path to a package schedules that package and
        everything that depends on it, so the invariant the planner needs is:

          every crate that compiles a table-routed file under `prefix` into
          itself is either the routed owner or a dependent of it.

        Both halves of that bite. `crates/extensions/packages/` is embedded by
        four crates, not one — `ironclaw_extension_host`,
        `ironclaw_extension_manager` and `ironclaw_composition` reach
        into it alongside `ironclaw_extension_support` — and they are covered
        only because each depends on the support crate. Drop that edge and a
        shipped-artifact change stops scheduling a crate that embeds it, which
        is the silent under-schedule this table exists to prevent.
        """
        crate_dirs = _workspace_crate_directories()
        self.assertGreater(len(crate_dirs), 20, "crate inventory looks truncated")

        self.assertNotEqual(planner.EMBEDDED_ASSET_OWNERS, ())
        for prefix, owner in planner.EMBEDDED_ASSET_OWNERS:
            with self.subTest(prefix=prefix):
                self.assertTrue(
                    (ROOT / prefix).is_dir(),
                    f"asset tree {prefix} no longer exists",
                )
                self.assertIn(
                    owner,
                    crate_dirs,
                    f"{prefix} routes to {owner}, which is no longer a crate",
                )
                embedders = _crates_embedding(prefix, crate_dirs)
                self.assertIn(
                    owner,
                    embedders,
                    f"{prefix} routes to {owner}, which embeds nothing from it; "
                    f"the crates that do are {sorted(embedders)}",
                )
                for embedder in sorted(embedders - {owner}):
                    self.assertTrue(
                        _depends_on(embedder, owner, crate_dirs),
                        f"{embedder} compiles files from {prefix} but does not "
                        f"depend on {owner}, so routing there never schedules it",
                    )

    def test_agent_guidance_does_not_mask_a_real_lane_in_the_same_pr(self) -> None:
        """Classifying `.claude/` must not swallow its neighbours.

        A guidance edit riding along with a crate change still selects that
        crate's lane — the ignore is per-path, not per-PR.
        """
        plan = self.plan(
            "pull_request",
            [".claude/commands/trace.md", "crates/alpha/src/lib.rs"],
        )
        self.assertEqual(plan["mode"], "selected")
        self.assertNotEqual(plan["crate_buckets"], [])

    def test_agent_guidance_paths_are_prose_and_select_nothing(self) -> None:
        """`.claude/**` is guidance, like `docs/**`.

        Regression fixture: the planner used to fail closed on every
        `.claude/` path, so any PR that updated a rule, skill, or command
        alongside its code could not be planned at all.
        """
        for path in (
            ".claude/commands/triage-prs.md",
            ".claude/rules/safety-and-sandbox.md",
            ".claude/skills/reborn-feature/SKILL.md",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                docs = self.plan("pull_request", ["docs/reborn/README.md"])
                self.assertEqual(plan["mode"], "none")
                self.assertEqual(plan, docs)

    def test_generated_wiki_paths_are_prose_and_select_nothing(self) -> None:
        """`openwiki/**` is generated prose, like `docs/**`.

        Regression fixture for the same class as `.claude/` above: the planner
        had no rule for the auto-generated wiki, so its fail-closed arm rejected
        any PR that touched it. A crate rename touches it by construction — the
        wiki's prose names crate directories — so the whole of WS6 was
        unplannable while the gap stood.

        Paired against `docs/`: the assertion is that the two are the *same*
        plan, so a later change that quietly escalates the wiki to a lane fails
        here too.
        """
        for path in (
            "openwiki/quickstart.md",
            "openwiki/architecture/crates.md",
            "openwiki/development/workflows.md",
        ):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                docs = self.plan("pull_request", ["docs/reborn/README.md"])
                self.assertEqual(plan["mode"], "none")
                self.assertEqual(plan, docs)

        # The decision is per-path, not per-PR: a real change riding along
        # still selects its lane.
        paired = self.plan(
            "pull_request", ["openwiki/quickstart.md", "crates/alpha/src/lib.rs"]
        )
        self.assertEqual(paired["mode"], "selected")
        self.assertNotEqual(paired["crate_buckets"], [])

    def test_crate_tree_prose_outside_any_package_selects_nothing(self) -> None:
        """`crates/AGENTS.md` and friends belong to no package.

        Regression fixture: the planner fail-closed with "unmapped crate path"
        on the family-level guidance files, which the restructure edits in the
        same PR as the crates they describe.
        """
        docs = self.plan("pull_request", ["docs/reborn/README.md"])
        for path in ("crates/AGENTS.md", "crates/README.md", "crates/Architecture.md"):
            with self.subTest(path=path):
                plan = self.plan("pull_request", [path])
                # `reasons` is human-facing narration; the selection must match.
                self.assertEqual(
                    {k: v for k, v in plan.items() if k != "reasons"},
                    {k: v for k, v in docs.items() if k != "reasons"},
                )

    def test_crate_prose_stays_narrow_while_crate_code_widens(self) -> None:
        """Negative probe: the prose carve-out must not swallow crate code.

        `crates/AGENTS.md` selects nothing; a source file under the same
        unmapped directory widens to `full`. If the Markdown check were
        wrongly applied, the second case would also select nothing.
        """
        self.assertEqual(self.plan("pull_request", ["crates/AGENTS.md"])["mode"], "none")
        self.assertEqual(
            self.plan("pull_request", ["crates/not_a_package/src/lib.rs"])["mode"],
            "full",
        )

    def test_unclassified_paths_outside_guidance_still_fail_closed(self) -> None:
        """Negative probe: widening IGNORED_PREFIXES must not open the gate.

        The build-input probe was `Dockerfile` until #7084 gave that path a
        decision (see `test_container_and_hook_inputs_are_owned_by_static_gates`);
        `Makefile` is the genuinely undecided repo-root build input that keeps
        this arm exercised, matching `test_unclassified_build_input_fails_fast`.
        """
        for path in ("Makefile", "claude/rules/x.md", ".claudeignore"):
            with self.subTest(path=path):
                with self.assertRaisesRegex(
                    ValueError, "unclassified pull-request path"
                ):
                    self.plan("pull_request", [path])


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

    def test_hosted_mcp_support_selects_its_owning_integration_lane(self) -> None:
        owner = planner.INTEGRATION_SUPPORT_OWNERS[
            "tests/support/hosted_mcp_registration_server.rs"
        ]
        expected_lane = planner._integration_test_lanes()[owner]

        plan = self.plan(
            "pull_request", ["tests/support/hosted_mcp_registration_server.rs"]
        )

        self.assertEqual(plan["integration_lanes"], [expected_lane])

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
        self.assertIn("needs.changes.outputs.run_sandbox_docker", workflow)
        self.assertIn("--test user_sandbox_docker_live", workflow)
        self.assertIn("--test reborn_integration_sandbox_shell_turn", workflow)
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
