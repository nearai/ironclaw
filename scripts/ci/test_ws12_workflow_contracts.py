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
    CRATE_NAME_RESIDUE,
    CRATE_SCOPE_FILTERS,
    DOCKER_WORKFLOW,
    E2E_WORKFLOW,
    NIGHTLY_DEEP_CI_WORKFLOW,
    PLATFORM_WORKFLOW,
    REQUIRED_MARKERS,
    STRESS_WORKFLOW,
    WEBUI_FRONTEND_CRATE,
    WEBUI_NESTED_LOCKFILE_PATTERN,
    crate_directory,
    github_glob_to_regex,
    load_workflows,
    step_body,
    validate_crate_name_residue,
    validate_crate_scope_filters,
    validate_e2e_scope_filters,
    validate_production_lint_targets,
    validate_webui_frontend_sites,
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
            any("substrates/ironclaw_event_log" in error for error in errors), errors
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

    def test_production_lint_passes_as_checked_in_without_reading_its_neighbour(
        self,
    ) -> None:
        """Passing here is itself the proof that the scan stays in its step.

        `Check all-target lints` sits directly below and legitimately passes
        the flags this contract forbids, so a scan that overran its own step
        would fail on the checked-in workflow. The first assertion keeps that
        proof honest: if the neighbour ever stops passing `--tests`, this test
        would still pass while having stopped testing anything.
        """
        neighbour = step_body(
            self.workflows[CODE_STYLE_WORKFLOW], "Check all-target lints"
        )
        self.assertIsNotNone(neighbour)
        self.assertIn("--tests", neighbour or "")

        self.assertEqual(
            validate_production_lint_targets(self.workflows[CODE_STYLE_WORKFLOW]), []
        )

    def test_target_filters_on_the_production_lint_fail_loudly(self) -> None:
        """Every explicit target selector, in bare and value-bearing form.

        Regression for PR #6965: the lane ran `cargo clippy -p <pkg> --lib
        --bins`, and the first PR whose only changed package was the bin-only
        `ironclaw` died on `no library targets found in package` — exit 101,
        no lint ever run. `--bins` alone is the quieter half of the same bug:
        on a lib-only package cargo warns "no targets matched; this is a
        no-op" and the lane reports green having linted nothing. The rest swap
        the package's default production targets for a hand-picked set.

        Exact-count, so `--bins` is never also reported as `--bin` — a
        substring match would do exactly that.
        """
        for injected, flag in (
            ("--lib", "--lib"),
            ("--bins", "--bins"),
            ("--bin ironclaw", "--bin"),
            ("--all-targets", "--all-targets"),
            ("--tests", "--tests"),
            ("--test smoke", "--test"),
            ("--examples", "--examples"),
            ("--example demo", "--example"),
            ("--benches", "--benches"),
            ("--bench=throughput", "--bench"),
        ):
            with self.subTest(injected=injected):
                sabotaged = self.workflows[CODE_STYLE_WORKFLOW].replace(
                    'cargo clippy "${package_args[@]}" \\',
                    f'cargo clippy "${{package_args[@]}}" {injected} \\',
                )
                self.assertNotEqual(sabotaged, self.workflows[CODE_STYLE_WORKFLOW])

                errors = validate_production_lint_targets(sabotaged)
                self.assertEqual(len(errors), 1, errors)
                self.assertIn(f"must not pass {flag} ", errors[0])

    def test_widening_the_clippy_matrix_flags_fails_loudly(self) -> None:
        """`${{ matrix.flags }}` is the lane's other flag channel."""
        sabotaged = self.workflows[CODE_STYLE_WORKFLOW].replace(
            '"flags":"--all-features"', '"flags":"--all-features --all-targets"'
        )
        self.assertNotEqual(sabotaged, self.workflows[CODE_STYLE_WORKFLOW])

        errors = validate_production_lint_targets(sabotaged)
        self.assertTrue(
            any("clippy_matrix flags" in e and "--all-targets" in e for e in errors),
            errors,
        )

        # …and only that matrix. Some other matrix in this workflow may
        # legitimately carry `--tests`; reading it as widening *this* lane
        # would be a false failure on an unrelated change.
        unrelated = self.workflows[CODE_STYLE_WORKFLOW] + (
            '\n          echo \'doc_matrix=[{"name":"docs","flags":"--tests"}]\''
            ' >> "$GITHUB_OUTPUT"\n'
        )
        self.assertEqual(validate_production_lint_targets(unrelated), [])

    def test_losing_the_step_or_its_command_fails_loudly(self) -> None:
        """A contract that cannot see the command must say so, not pass."""
        renamed = self.workflows[CODE_STYLE_WORKFLOW].replace(
            "name: Check production-target lints", "name: Check nothing at all"
        )
        self.assertTrue(
            any(
                "could not find the 'Check production-target lints' step" in e
                for e in validate_production_lint_targets(renamed)
            )
        )

        moved = self.workflows[CODE_STYLE_WORKFLOW].replace(
            'cargo clippy "${package_args[@]}" \\\n            ${{ matrix.flags }} -- -D warnings',
            "bash scripts/ci/production-clippy.sh",
        )
        self.assertNotEqual(moved, self.workflows[CODE_STYLE_WORKFLOW])
        self.assertTrue(
            any(
                "no longer runs `cargo clippy`" in e
                for e in validate_production_lint_targets(moved)
            )
        )

    def test_masking_the_production_lint_exit_status_fails_loudly(self) -> None:
        """A lane that runs clippy and ignores it is the silent-green case.

        Distinct from a disguised command, which this contract deliberately
        does not chase: each of these is a plausible edit made on purpose and
        for a stated reason — unblock the queue, quiet a flaky lane — and each
        leaves the lint running and its verdict discarded.
        """
        for mask, injected in (
            ("|| true", '${{ matrix.flags }} -- -D warnings || true'),
            ("|| :", '${{ matrix.flags }} -- -D warnings || :'),
            ("set +e", 'set +e\n          ${{ matrix.flags }} -- -D warnings'),
        ):
            with self.subTest(mask=mask):
                sabotaged = self.workflows[CODE_STYLE_WORKFLOW].replace(
                    "${{ matrix.flags }} -- -D warnings", injected
                )
                self.assertNotEqual(sabotaged, self.workflows[CODE_STYLE_WORKFLOW])

                errors = validate_production_lint_targets(sabotaged)
                self.assertTrue(
                    any(f"must not mask the lint's exit status with `{mask}`" in e
                        for e in errors),
                    errors,
                )

        # The YAML-level equivalent: the command runs, fails, and the job
        # reports success anyway.
        tolerated = self.workflows[CODE_STYLE_WORKFLOW].replace(
            "      - name: Check production-target lints\n",
            "      - name: Check production-target lints\n        continue-on-error: true\n",
        )
        self.assertNotEqual(tolerated, self.workflows[CODE_STYLE_WORKFLOW])
        self.assertTrue(
            any(
                "continue-on-error" in e
                for e in validate_production_lint_targets(tolerated)
            ),
            validate_production_lint_targets(tolerated),
        )

    def test_production_lint_failures_reach_the_top_level_contract(self) -> None:
        """The validator must stay wired into `validate_workflow_texts`.

        Every assertion above calls it directly, so without this the guard
        could be unhooked from the entry point CI runs and stay green.
        """
        sabotaged = dict(self.workflows)
        sabotaged[CODE_STYLE_WORKFLOW] = self.workflows[CODE_STYLE_WORKFLOW].replace(
            'cargo clippy "${package_args[@]}" \\',
            'cargo clippy "${package_args[@]}" --lib \\',
        )
        self.assertNotEqual(
            sabotaged[CODE_STYLE_WORKFLOW], self.workflows[CODE_STYLE_WORKFLOW]
        )

        errors = validate_workflow_texts(sabotaged)
        self.assertTrue(any("must not pass --lib" in e for e in errors), errors)

    def test_code_style_runs_workflow_and_shard_sabotage_tests(self) -> None:
        workflow = (ROOT / ".github/workflows/code_style.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_ws12_suite_shards.py", workflow)
        self.assertIn("python3 scripts/ci/test_ws12_workflow_contracts.py", workflow)

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
                "crates/([^/]+/)*ironclaw_turn_runner/",
                "crates/substrates/ironclaw_turn_runner/src/lib.rs",
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
            "^(crates/([^/]+/)*ironclaw_common/|",
            "^(crates/([^/]+/)*ironclaw_wasm_product_adapters/"
            "|crates/([^/]+/)*ironclaw_common/|",
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
                CODE_STYLE_WORKFLOW, "crates/([^/]+/)*ironclaw_config/|", ""
            ),
            ROOT,
        )

        self.assertTrue(
            any("ironclaw_config" in error for error in errors), errors
        )

    def test_over_broadening_a_filter_fails_loudly(self) -> None:
        """Matching everything is not a fix — the dist-build and stress lanes
        are deliberately scoped and must stay scoped."""
        for workflows in (
            self.sabotage(CODE_STYLE_WORKFLOW, "^(crates/([^/]+/)*ironclaw_turn_runner/", "^(crates/"),
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
                    "grep -Eq '^(crates/([^/]+/)*ironclaw_turn_runner/",
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
            crate_globs=(("ironclaw_extension_support", "assets/*/gone.toml"),),
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
            "crates/([^/]+/)*ironclaw_turn_runner/",
            "crates/ironclaw_turn_runner/",
        )

        self.assertTrue(
            any(
                "crates/substrates/ironclaw_turn_runner" in error
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


class WebuiFrontendSiteSabotageTests(unittest.TestCase):
    """#7155 WS10: the 28 `crates/ironclaw_webui/frontend` sites.

    `cache-dependency-path` is a static YAML value, so its fix twins the flat
    lockfile line with a nested wildcard sibling; every `cd` and
    `working-directory:` site resolves dynamically through
    scripts/ci/crate-dir.sh and must carry no literal trace of the flat path.
    These are the sabotage cases that prove the pin catches both regressions.
    """

    def setUp(self) -> None:
        self.workflows = load_workflows(ROOT)
        # Resolved dynamically, never hardcoded as `crates/ironclaw_webui`:
        # this repo's crate tree is mid-restructure and the concurrent WS10
        # physical-move work observably flips `ironclaw_webui` between its
        # flat and family-nested location while this suite runs. A sabotage
        # string built from a stale assumption about the current location
        # would stop matching real workflow text the moment the tree moves
        # again — exactly the fragility this whole pin exists to eliminate.
        self.webui_dir = crate_directory(WEBUI_FRONTEND_CRATE, ROOT)

    def sabotage(self, workflow: str, old: str, new: str, count: int = -1) -> dict[str, str]:
        mutated = copy.deepcopy(self.workflows)
        replaced = mutated[workflow].replace(old, new, count) if count >= 0 else mutated[workflow].replace(old, new)
        self.assertNotEqual(replaced, mutated[workflow], f"no-op sabotage: {old!r}")
        mutated[workflow] = replaced
        return mutated

    def test_checked_in_webui_sites_pass(self) -> None:
        self.assertEqual(validate_webui_frontend_sites(self.workflows, ROOT), [])

    def test_every_site_was_actually_converted(self) -> None:
        """Sanity floor: the checked-in tree must contain the expected number
        of sanctioned cache-dependency-path pairings (12) — a pin that passes
        vacuously because nobody scanned anything is the defect being fixed."""
        flat_lockfile = f"{self.webui_dir}/frontend/pnpm-lock.yaml"
        pairs = 0
        for text in self.workflows.values():
            lines = text.splitlines()
            for index, line in enumerate(lines):
                if line.strip() != flat_lockfile:
                    continue
                following = next(
                    (c.strip() for c in lines[index + 1 :] if c.strip()), ""
                )
                if following == WEBUI_NESTED_LOCKFILE_PATTERN:
                    pairs += 1
        self.assertEqual(pairs, 12, "expected exactly 12 cache-dependency-path sites")

    def test_reintroducing_a_bare_cd_site_fails_loudly(self) -> None:
        """The exact pre-#7155 regression: a `cd` back to the flat literal."""
        sabotaged = self.sabotage(
            ".github/workflows/coverage.yml",
            'set -euo pipefail\n          webui_dir="$(bash scripts/ci/crate-dir.sh '
            'ironclaw_webui)"\n          cd "${webui_dir}/frontend"',
            f"cd {self.webui_dir}/frontend",
            count=1,
        )
        errors = validate_webui_frontend_sites(sabotaged, ROOT)

        self.assertTrue(
            any(
                "coverage.yml" in error and "hardcodes" in error and "frontend'" in error
                for error in errors
            ),
            errors,
        )

    def test_reintroducing_a_bare_working_directory_site_fails_loudly(self) -> None:
        sabotaged = self.sabotage(
            CODE_STYLE_WORKFLOW,
            "working-directory: ${{ env.WEBUI_FRONTEND_DIR }}",
            f"working-directory: {self.webui_dir}/frontend",
            count=1,
        )
        errors = validate_webui_frontend_sites(sabotaged, ROOT)

        self.assertTrue(
            any(CODE_STYLE_WORKFLOW in error and "hardcodes" in error for error in errors),
            errors,
        )

    def test_dropping_the_nested_cache_glob_sibling_fails_loudly(self) -> None:
        sabotaged = self.sabotage(
            ".github/workflows/reborn-playwright.yml",
            f"            {self.webui_dir}/frontend/pnpm-lock.yaml\n"
            f"            {WEBUI_NESTED_LOCKFILE_PATTERN}\n",
            f"            {self.webui_dir}/frontend/pnpm-lock.yaml\n",
        )
        errors = validate_webui_frontend_sites(sabotaged, ROOT)

        self.assertTrue(
            any(
                "reborn-playwright.yml" in error and "not twinned" in error
                for error in errors
            ),
            errors,
        )

    def test_webui_crate_unresolvable_fails_loudly(self) -> None:
        """The exact `ironclaw_webui` name must keep resolving — if renamed or
        deleted from the inventory, this must refuse rather than pass with
        nothing measured."""
        with tempfile.TemporaryDirectory() as empty:
            errors = validate_webui_frontend_sites(self.workflows, Path(empty))

        self.assertTrue(
            any(
                "crate inventory cannot resolve" in error and WEBUI_FRONTEND_CRATE in error
                for error in errors
            ),
            errors,
        )

    def test_empty_workflow_set_is_an_empty_probe_not_a_pass(self) -> None:
        """A probe that discovers nothing must fail closed, matching the
        `crate_globs` / `CRATE_SCOPE_FILTERS` fail-closed floor."""
        errors = validate_webui_frontend_sites({}, ROOT)

        self.assertTrue(
            any("cache-dependency-path probe set is empty" in error for error in errors),
            errors,
        )

    def test_flat_lockfile_missing_on_disk_fails_loudly(self) -> None:
        """The real-file probe: if the lockfile this pin is measured against
        stops existing, the pin must say so rather than silently matching
        nothing (mirrors CRATE_SCOPE_FILTERS' `crate_globs` discovery floor)."""
        with tempfile.TemporaryDirectory() as empty_str:
            empty = Path(empty_str)
            (empty / "crates" / WEBUI_FRONTEND_CRATE).mkdir(parents=True)
            (empty / "crates" / WEBUI_FRONTEND_CRATE / "Cargo.toml").write_text(
                "[package]\nname = \"ironclaw_webui\"\n"
            )
            # crate_directory() refuses under crate_tree.MIN_CRATE_DIRECTORIES
            # (20) real crates — a deliberate fail-closed floor, not something
            # this fixture should route around. Real filler directories, not
            # symlinks: crate_directories() finds manifests via `rglob`, which
            # does not descend into symlinked directories, so a symlink farm
            # would silently under-count and trip the very floor this fixture
            # exists to clear.
            for n in range(25):
                filler = empty / "crates" / f"ironclaw_filler_{n}"
                filler.mkdir(parents=True)
                (filler / "Cargo.toml").write_text(f'[package]\nname = "ironclaw_filler_{n}"\n')

            errors = validate_webui_frontend_sites(self.workflows, empty)

        self.assertTrue(
            any("does not exist on disk" in error for error in errors), errors
        )


class CrateNameResidueSabotageTests(unittest.TestCase):
    """#7155 WS10 B1/B2: docker.yml and nightly-deep-ci.yml resolve their
    governed crate's PATH dynamically now, but still spell the crate NAME as a
    bare token. This is the pin that catches that token going stale — a
    rename or deletion the workflow text never followed.
    """

    def setUp(self) -> None:
        self.workflows = load_workflows(ROOT)

    def sabotage(self, workflow: str, old: str, new: str) -> dict[str, str]:
        mutated = copy.deepcopy(self.workflows)
        replaced = mutated[workflow].replace(old, new)
        self.assertNotEqual(replaced, mutated[workflow], f"no-op sabotage: {old!r}")
        mutated[workflow] = replaced
        return mutated

    def test_checked_in_residue_passes(self) -> None:
        self.assertEqual(validate_crate_name_residue(self.workflows, ROOT), [])

    def test_every_governed_workflow_declares_probes(self) -> None:
        self.assertEqual(
            {workflow for workflow, _ in CRATE_NAME_RESIDUE},
            {DOCKER_WORKFLOW, NIGHTLY_DEEP_CI_WORKFLOW},
        )

    def test_dropping_the_docker_crate_name_fails_loudly(self) -> None:
        sabotaged = self.sabotage(
            DOCKER_WORKFLOW, "ironclaw_cli", "ironclaw_renamed_cli"
        )
        errors = validate_crate_name_residue(sabotaged, ROOT)

        self.assertTrue(
            any(
                DOCKER_WORKFLOW in error
                and "ironclaw_cli" in error
                and "no longer names" in error
                for error in errors
            ),
            errors,
        )

    def test_dropping_the_nightly_crate_name_fails_loudly(self) -> None:
        sabotaged = self.sabotage(
            NIGHTLY_DEEP_CI_WORKFLOW, "ironclaw_capabilities", "ironclaw_dispatch_only"
        )
        errors = validate_crate_name_residue(sabotaged, ROOT)

        self.assertTrue(
            any(
                NIGHTLY_DEEP_CI_WORKFLOW in error
                and "ironclaw_capabilities" in error
                and "no longer names" in error
                for error in errors
            ),
            errors,
        )

    def test_a_residue_crate_the_inventory_cannot_resolve_fails_loudly(self) -> None:
        """A name that still appears as a token but the inventory can no
        longer resolve (renamed elsewhere, deleted) must refuse."""
        stale = ((DOCKER_WORKFLOW, "ironclaw_reborn_cli_renamed"),)
        # Make the workflow text contain the stale name too, so the "no
        # longer names" branch does not fire first and mask this one.
        workflows = self.sabotage(
            DOCKER_WORKFLOW, "ironclaw_cli", "ironclaw_reborn_cli_renamed"
        )
        with self.patched_residue(stale):
            errors = validate_crate_name_residue(workflows, ROOT)

        self.assertTrue(
            any(
                "ironclaw_reborn_cli_renamed" in error and "cannot resolve" in error
                for error in errors
            ),
            errors,
        )

    def test_missing_workflow_fails_loudly(self) -> None:
        mutated = copy.deepcopy(self.workflows)
        del mutated[NIGHTLY_DEEP_CI_WORKFLOW]

        errors = validate_crate_name_residue(mutated, ROOT)

        self.assertTrue(
            any(
                NIGHTLY_DEEP_CI_WORKFLOW in error and "not loaded" in error
                for error in errors
            ),
            errors,
        )

    def test_residue_failures_reach_the_top_level_contract(self) -> None:
        sabotaged = self.sabotage(
            NIGHTLY_DEEP_CI_WORKFLOW, "ironclaw_capabilities", "ironclaw_dispatch_only"
        )

        self.assertTrue(
            any(
                "no longer names crate 'ironclaw_capabilities'" in error
                for error in validate_workflow_texts(sabotaged, ROOT)
            )
        )

    def patched_residue(self, filters: tuple[tuple[str, str], ...]):
        test = self

        class _Patch:
            def __enter__(self) -> None:
                self.saved = ws12_workflow_contracts.CRATE_NAME_RESIDUE
                ws12_workflow_contracts.CRATE_NAME_RESIDUE = filters

            def __exit__(self, *_: object) -> None:
                ws12_workflow_contracts.CRATE_NAME_RESIDUE = self.saved

        test.addCleanup(
            setattr,
            ws12_workflow_contracts,
            "CRATE_NAME_RESIDUE",
            ws12_workflow_contracts.CRATE_NAME_RESIDUE,
        )
        return _Patch()


if __name__ == "__main__":
    unittest.main()
