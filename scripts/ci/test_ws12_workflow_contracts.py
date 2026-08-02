#!/usr/bin/env python3
"""Sabotage tests for scheduled, merge, and release WS12 lanes."""

from __future__ import annotations

import copy
import dataclasses
import tempfile
import unittest
from pathlib import Path

import ws12_workflow_contracts
from ws12_workflow_contracts import (
    CODE_STYLE_WORKFLOW,
    CRATE_SCOPE_FILTERS,
    E2E_WORKFLOW,
    PLATFORM_WORKFLOW,
    REQUIRED_MARKERS,
    STRESS_WORKFLOW,
    github_glob_to_regex,
    load_workflows,
    validate_crate_scope_filters,
    validate_e2e_scope_filters,
    validate_workflow_texts,
)

ROOT = Path(__file__).resolve().parents[2]


class WorkflowContractSabotageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflows = load_workflows(ROOT)

    def test_checked_in_workflows_cover_every_lane(self) -> None:
        self.assertEqual(validate_workflow_texts(self.workflows), [])

    def test_removing_each_lane_marker_fails_loudly(self) -> None:
        for path, markers in REQUIRED_MARKERS.items():
            for marker in markers:
                with self.subTest(path=path, marker=marker):
                    sabotaged = copy.deepcopy(self.workflows)
                    sabotaged[path] = sabotaged[path].replace(marker, "")
                    errors = validate_workflow_texts(sabotaged)
                    self.assertTrue(
                        any(path in error and marker in error for error in errors),
                        errors,
                    )

    def test_missing_workflow_fails_loudly(self) -> None:
        sabotaged = copy.deepcopy(self.workflows)
        path = next(iter(REQUIRED_MARKERS))
        del sabotaged[path]

        self.assertIn(f"missing workflow: {path}", validate_workflow_texts(sabotaged))

    def test_unconditional_skip_fails_loudly(self) -> None:
        path = ".github/workflows/nightly-deep-ci.yml"
        conditions = (
            "if: false",
            "if:  false",
            "if: 'false'",
            "if: |\n      false",
        )
        for condition in conditions:
            with self.subTest(condition=condition):
                sabotaged = copy.deepcopy(self.workflows)
                sabotaged[path] += f"\n  disabled-lane:\n    {condition}\n"

                self.assertTrue(
                    any(
                        "unconditionally skipped" in error
                        for error in validate_workflow_texts(sabotaged)
                    )
                )

    def test_reborn_e2e_scope_filters_pass_as_checked_in(self) -> None:
        self.assertEqual(validate_e2e_scope_filters(self.workflows[E2E_WORKFLOW]), [])

    def test_flat_tree_scope_regex_fails_loudly(self) -> None:
        """Re-narrowing the scope regex to `crates/ironclaw_*` must not pass.

        This is the exact regression the WS10 rewrite exists to prevent: the
        pattern still matches every path in today's flat tree, so nothing looks
        broken until crates move and every E2E job silently stops running.
        """
        sabotaged = self.workflows[E2E_WORKFLOW].replace(
            "grep -Eq '^(crates/|", "grep -Eq '^(crates/ironclaw_[^/]+/|"
        )
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(
            any("substrates/ironclaw_events" in error for error in errors), errors
        )

    def test_flat_tree_paths_glob_fails_loudly(self) -> None:
        sabotaged = self.workflows[E2E_WORKFLOW].replace(
            '- "crates/**"', '- "crates/ironclaw_*/**"'
        )
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(any("push `paths:` filter" in error for error in errors), errors)

    def test_over_broad_scope_regex_fails_loudly(self) -> None:
        """The filter must stay a filter — matching everything is not a fix."""
        sabotaged = self.workflows[E2E_WORKFLOW].replace(
            "grep -Eq '^(crates/|", "grep -Eq '^(|"
        )
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(any("must NOT be in scope" in error for error in errors), errors)

    def test_scope_regex_survives_an_escaped_newline_continuation(self) -> None:
        """A guard split across a line continuation is still a guard.

        Regression for the one-line-only form: it reported this exact workflow
        as having no scope regex at all, failing the build over a formatting
        choice (.claude/rules/review-discipline.md — guardrails must handle
        multiline syntax).
        """
        wrapped = self.workflows[E2E_WORKFLOW].replace(
            "| grep -Eq '^(crates/|", "| grep -Eq \\\n            '^(crates/|"
        )
        self.assertNotEqual(wrapped, self.workflows[E2E_WORKFLOW])

        self.assertEqual(validate_e2e_scope_filters(wrapped), [])

    def test_missing_scope_regex_fails_loudly(self) -> None:
        sabotaged = self.workflows[E2E_WORKFLOW].replace("grep -Eq '^(crates/|", "true #")
        errors = validate_e2e_scope_filters(sabotaged)

        self.assertTrue(
            any("could not find the `changes` job scope regex" in e for e in errors),
            errors,
        )

    def test_code_style_runs_workflow_and_shard_sabotage_tests(self) -> None:
        workflow = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_ws12_suite_shards.py", workflow)
        self.assertIn("python3 scripts/ci/test_ws12_workflow_contracts.py", workflow)

    def test_workflow_dispatch_allows_expected_critical_mutation_skip(self) -> None:
        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            'job_result_ok "critical-mutation" '
            '"${{ needs.critical-mutation.result }}" '
            '"${{ github.event_name == \'workflow_dispatch\' }}" "allow"',
            workflow,
        )

    def test_code_style_fast_checks_share_one_runner_without_losing_gates(self) -> None:
        workflow = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("  fast-checks:", workflow)
        for marker in (
            "cargo fmt --all -- --check",
            "EmbarkStudios/cargo-deny-action@",
            "git ls-files -ci --exclude-standard",
            "scripts/ci/check-include-str-paths.sh",
            "scripts/ci/check-hermetic-env.sh",
            "scripts/check_no_panics.py --reborn-baseline",
            "scripts/ci/check-composition-budget.sh",
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, workflow)

        for retired_job in (
            "  clippy-matrix:",
            "  format:",
            "  deny-check:",
            "  tracked-ignored-files:",
            "  static-checks:",
            "  no-panics:",
            "  composition-budget:",
        ):
            with self.subTest(retired_job=retired_job):
                self.assertNotIn(retired_job, workflow)


class CrateScopeFilterSabotageTests(unittest.TestCase):
    """#6963: the three remaining crate-keyed workflow scope filters.

    Each one goes silently green under family nesting — the dist-build lane
    skips, every WASM ABI check skips, the stress workflow stops triggering —
    and none of them can assert anything about itself. These are the sabotage
    cases that prove the pin binds.
    """

    def setUp(self) -> None:
        self.workflows = load_workflows(ROOT)

    def sabotage(self, workflow: str, old: str, new: str) -> dict[str, str]:
        mutated = copy.deepcopy(self.workflows)
        replaced = mutated[workflow].replace(old, new)
        self.assertNotEqual(replaced, mutated[workflow], f"no-op sabotage: {old!r}")
        mutated[workflow] = replaced
        return mutated

    def test_checked_in_scope_filters_pass(self) -> None:
        self.assertEqual(validate_crate_scope_filters(self.workflows, ROOT), [])

    def test_every_filter_declares_probes(self) -> None:
        """A pin with no probes asserts nothing — the defect being fixed."""
        for scope in CRATE_SCOPE_FILTERS:
            with self.subTest(filter=scope.name):
                self.assertTrue(
                    scope.crates or scope.crate_globs or scope.in_scope,
                    "filter declares no in-scope probe",
                )
                self.assertTrue(scope.out_of_scope, "filter declares no negative probe")

    def test_renarrowing_each_filter_to_the_flat_tree_fails_loudly(self) -> None:
        """The exact regression this rewrite exists to prevent.

        `crates/<name>/` still matches every path in today's flat tree, so
        nothing looks broken until crates move and the lane silently stops.
        """
        for workflow, flat, nested_probe in (
            (
                CODE_STYLE_WORKFLOW,
                "crates/([^/]+/)*ironclaw_runner/",
                "crates/substrates/ironclaw_runner/src/lib.rs",
            ),
            (
                PLATFORM_WORKFLOW,
                "crates/([^/]+/)*ironclaw_common/",
                "crates/substrates/ironclaw_common/src/lib.rs",
            ),
        ):
            with self.subTest(workflow=workflow):
                flattened = flat.replace("([^/]+/)*", "")
                errors = validate_crate_scope_filters(
                    self.sabotage(workflow, flat, flattened), ROOT
                )
                self.assertTrue(
                    any(nested_probe in error for error in errors), errors
                )

    def test_dropping_the_nested_stress_glob_fails_loudly(self) -> None:
        errors = validate_crate_scope_filters(
            self.sabotage(
                STRESS_WORKFLOW, '      - "crates/*/ironclaw_turns/**"\n', ""
            ),
            ROOT,
        )

        self.assertTrue(
            any(
                "crates/substrates/ironclaw_turns/src/lib.rs" in error
                for error in errors
            ),
            errors,
        )

    def test_a_filter_naming_a_deleted_crate_fails_loudly(self) -> None:
        """`ironclaw_wasm_product_adapters` sat in the WASM filter for releases
        after the crate was deleted, matching nothing. Naming a crate the
        inventory cannot resolve is now an error rather than dead weight."""
        stale = dataclasses.replace(
            next(f for f in CRATE_SCOPE_FILTERS if f.name == "has_direct_wasm_abi_risk"),
            crates=(("ironclaw_wasm_product_adapters", "src/lib.rs"),),
        )
        workflows = self.sabotage(
            PLATFORM_WORKFLOW,
            "^(wit/|",
            "^(wit/|crates/([^/]+/)*ironclaw_wasm_product_adapters/|",
        )
        with self.patched_filters((stale,)):
            errors = validate_crate_scope_filters(workflows, ROOT)

        self.assertTrue(
            any(
                "ironclaw_wasm_product_adapters" in error
                and "cannot resolve" in error
                for error in errors
            ),
            errors,
        )

    def test_dropping_a_governed_crate_name_fails_loudly(self) -> None:
        errors = validate_crate_scope_filters(
            self.sabotage(
                CODE_STYLE_WORKFLOW, "crates/([^/]+/)*ironclaw_reborn_config/|", ""
            ),
            ROOT,
        )

        self.assertTrue(
            any("ironclaw_reborn_config" in error for error in errors), errors
        )

    def test_over_broadening_a_filter_fails_loudly(self) -> None:
        """Matching everything is not a fix — the dist-build and stress lanes
        are deliberately scoped and must stay scoped."""
        for workflows in (
            self.sabotage(CODE_STYLE_WORKFLOW, "^(crates/([^/]+/)*ironclaw_runner/", "^(crates/"),
            self.sabotage(STRESS_WORKFLOW, '- "crates/ironclaw_turns/**"', '- "crates/**"'),
        ):
            errors = validate_crate_scope_filters(workflows, ROOT)
            self.assertTrue(
                any("must NOT be in scope" in error for error in errors), errors
            )

    def test_deleting_a_filter_entirely_fails_loudly(self) -> None:
        for workflows, needle in (
            (
                self.sabotage(
                    CODE_STYLE_WORKFLOW,
                    "grep -Eq '^(crates/([^/]+/)*ironclaw_runner/",
                    "true #",
                ),
                "expected exactly one scope regex",
            ),
            (self.sabotage(STRESS_WORKFLOW, "    paths:\n", ""), "no `paths:` trigger filter"),
        ):
            errors = validate_crate_scope_filters(workflows, ROOT)
            self.assertTrue(any(needle in error for error in errors), errors)

    def test_a_second_paths_block_is_an_ambiguity_not_a_silent_pick(self) -> None:
        """`extract_scope_regex` refuses zero-or-many; the globs half took the
        first `paths:` block unconditionally, so a workflow that grew a second
        filter would have had the wrong one pinned with nothing to say so."""
        text = self.workflows[STRESS_WORKFLOW]
        start = text.index("    paths:\n")
        end = text.index("\n\n", start)
        duplicated = (
            text[:end] + "\n  push:\n" + text[start:end] + text[end:]
        )
        errors = validate_crate_scope_filters(
            {**self.workflows, STRESS_WORKFLOW: duplicated}, ROOT
        )

        self.assertTrue(
            any("found 2 `paths:` trigger filters" in error for error in errors), errors
        )

    def test_a_discovered_file_probe_that_discovers_nothing_fails_loudly(self) -> None:
        """The first-party manifest probe is derived from real files. If those
        files move (WS2 colocates extension packages), the probe must fail
        rather than quietly pin the filter against an empty set."""
        moved = dataclasses.replace(
            next(f for f in CRATE_SCOPE_FILTERS if f.name == "has_direct_wasm_abi_risk"),
            crate_globs=(("ironclaw_first_party_extensions", "assets/*/gone.toml"),),
        )
        with self.patched_filters((moved,)):
            errors = validate_crate_scope_filters(self.workflows, ROOT)

        self.assertTrue(
            any("discovered no files" in error for error in errors), errors
        )

    def test_a_broken_crate_inventory_fails_loudly(self) -> None:
        """No crate tree at all must refuse, never pass with zero probes."""
        with tempfile.TemporaryDirectory() as empty:
            errors = validate_crate_scope_filters(self.workflows, Path(empty))

        self.assertTrue(
            any("crate inventory unavailable" in error for error in errors), errors
        )

    def test_scope_filter_failures_reach_the_top_level_contract(self) -> None:
        """The pin is only worth anything if `main()`'s entry point sees it."""
        flattened = self.sabotage(
            CODE_STYLE_WORKFLOW,
            "crates/([^/]+/)*ironclaw_runner/",
            "crates/ironclaw_runner/",
        )

        self.assertTrue(
            any(
                "crates/substrates/ironclaw_runner" in error
                for error in validate_workflow_texts(flattened, ROOT)
            )
        )

    def test_github_glob_semantics(self) -> None:
        """`*` does not cross `/`; `**` does. The stress filter's two-form
        enumeration depends on exactly this."""
        single = github_glob_to_regex("crates/*/ironclaw_turns/**")
        self.assertTrue(single.match("crates/substrates/ironclaw_turns/src/lib.rs"))
        self.assertFalse(single.match("crates/ironclaw_turns/src/lib.rs"))
        self.assertFalse(single.match("crates/a/b/ironclaw_turns/src/lib.rs"))

        double = github_glob_to_regex("crates/ironclaw_turns/**")
        self.assertTrue(double.match("crates/ironclaw_turns/src/deep/lib.rs"))
        self.assertFalse(double.match("crates/substrates/ironclaw_turns/src/lib.rs"))

        exact = github_glob_to_regex("Cargo.toml")
        self.assertTrue(exact.match("Cargo.toml"))
        self.assertFalse(exact.match("crates/x/Cargo.toml"))
        self.assertFalse(exact.match("CargoXtoml"))

    def patched_filters(self, filters: tuple[object, ...]):
        test = self

        class _Patch:
            def __enter__(self) -> None:
                self.saved = ws12_workflow_contracts.CRATE_SCOPE_FILTERS
                ws12_workflow_contracts.CRATE_SCOPE_FILTERS = filters

            def __exit__(self, *_: object) -> None:
                ws12_workflow_contracts.CRATE_SCOPE_FILTERS = self.saved

        test.addCleanup(
            setattr,
            ws12_workflow_contracts,
            "CRATE_SCOPE_FILTERS",
            ws12_workflow_contracts.CRATE_SCOPE_FILTERS,
        )
        return _Patch()


if __name__ == "__main__":
    unittest.main()
