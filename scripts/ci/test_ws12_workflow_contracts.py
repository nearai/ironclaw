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
    LIBSQL_SCRIPTED_MEMORY_JOB,
    NIGHTLY_DEEP_CI_WORKFLOW,
    PLATFORM_WORKFLOW,
    REQUIRED_MARKERS,
    STEP_HEADING,
    STRESS_WORKFLOW,
    WEBUI_FRONTEND_CRATE,
    WEBUI_NESTED_LOCKFILE_PATTERN,
    crate_directory,
    extract_job_block,
    github_glob_to_regex,
    job_body,
    load_workflows,
    step_body,
    validate_crate_name_residue,
    validate_crate_scope_filters,
    validate_e2e_scope_filters,
    validate_libsql_scripted_memory_job,
    validate_postgres_scripted_parity,
    validate_production_lint_targets,
    validate_windows_webui_install_shell,
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

    def test_windows_webui_install_step_requires_bash(self) -> None:
        workflow = self.workflows[CODE_STYLE_WORKFLOW]
        windows_job = job_body(workflow, "clippy-windows")
        self.assertIsNotNone(windows_job)
        self.assertEqual(validate_windows_webui_install_shell(workflow), [])

        sabotaged_job = (windows_job or "").replace("        shell: bash\n", "")
        self.assertNotEqual(sabotaged_job, windows_job)
        sabotaged = dict(self.workflows)
        sabotaged[CODE_STYLE_WORKFLOW] = workflow.replace(
            windows_job or "", sabotaged_job, 1
        )

        errors = validate_workflow_texts(sabotaged)
        self.assertTrue(any("must run" in error for error in errors), errors)

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
    """#6963: the crate-keyed workflow scope filters, plus has_docs.

    The three crate-keyed filters go silently green under family nesting —
    the dist-build lane skips, every WASM ABI check skips, the stress
    workflow stops triggering. The fourth pin, `has_docs`, is deliberately
    not crate-keyed: docs/ sits outside the has_code scope, so its grep is
    the only trigger for the docs publication-boundary gate and narrowing it
    skips that gate with nothing red anywhere. None of these filters can
    assert anything about itself. These are the sabotage cases that prove
    each pin binds.
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

    def test_has_docs_trigger_is_pinned(self) -> None:
        """The docs publication gate rides its own trigger: docs-only PRs have
        has_code=false, so nothing else would run the boundary job. An
        unpinned grep here is the same silent-skip class as the crate
        filters."""
        self.assertIn("has_docs", {scope.name for scope in CRATE_SCOPE_FILTERS})

    def test_narrowing_the_has_docs_trigger_fails_loudly(self) -> None:
        sabotaged = self.sabotage(CODE_STYLE_WORKFLOW, "'^(docs/|", "'^(")
        errors = validate_crate_scope_filters(sabotaged, ROOT)
        self.assertTrue(
            any("has_docs" in error for error in errors),
            errors,
        )

    def test_code_style_docs_gate_markers_are_pinned(self) -> None:
        """Losing the boundary job, either of its two script steps, or the
        roll-up guard silently unhooks the gate for docs-only PRs. (Guard
        ORDERING relative to the has_code early exit is a separate pin —
        presence markers cannot see order.)"""
        markers = REQUIRED_MARKERS.get(CODE_STYLE_WORKFLOW, ())
        for marker in (
            "docs-publication-boundary:",
            "python3 scripts/ci/test_docs_publication_boundary.py",
            "python3 scripts/ci/docs_publication_boundary.py",
            '"${{ needs.docs-publication-boundary.result }}" != "success"',
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, markers)

    def test_moving_the_docs_guard_below_the_early_exit_fails_loudly(self) -> None:
        """Marker presence cannot see order: relocating the docs-gate guard
        to after the has_code `exit 0` keeps every REQUIRED_MARKERS string in
        the file while docs-only PRs (has_code=false) exit before the guard
        ever runs — a clean pass on a fully broken gate. The ordering pin
        must catch exactly this move."""
        text = self.workflows[CODE_STYLE_WORKFLOW]
        guard = ws12_workflow_contracts.CODE_STYLE_DOCS_GUARD_MARKER
        early_exit = ws12_workflow_contracts.CODE_STYLE_HAS_CODE_EXIT_MARKER
        self.assertIn(guard, text)
        self.assertIn(early_exit, text)

        # Relocate the guard's marker line to just after the early-exit line.
        guard_line = next(
            line for line in text.splitlines() if guard in line
        )
        exit_line = next(line for line in text.splitlines() if early_exit in line)
        sabotaged = copy.deepcopy(self.workflows)
        sabotaged[CODE_STYLE_WORKFLOW] = text.replace(
            guard_line + "\n", ""
        ).replace(exit_line, exit_line + "\n" + guard_line)
        self.assertIn(guard, sabotaged[CODE_STYLE_WORKFLOW])

        errors = validate_workflow_texts(sabotaged, ROOT)
        self.assertTrue(
            any("docs" in error and "early exit" in error for error in errors),
            errors,
        )

    def test_a_commented_decoy_guard_does_not_satisfy_the_order_pin(self) -> None:
        """A raw substring search accepts inert text: commenting out the old
        guard block above the early exit while the executable guard sits
        below it (a realistic refactor leftover) must still fail — only
        executable occurrences count."""
        text = self.workflows[CODE_STYLE_WORKFLOW]
        guard = ws12_workflow_contracts.CODE_STYLE_DOCS_GUARD_MARKER
        early_exit = ws12_workflow_contracts.CODE_STYLE_HAS_CODE_EXIT_MARKER
        guard_line = next(line for line in text.splitlines() if guard in line)
        exit_line = next(line for line in text.splitlines() if early_exit in line)
        decoy = "          # old guard: " + guard_line.strip()
        sabotaged = copy.deepcopy(self.workflows)
        sabotaged[CODE_STYLE_WORKFLOW] = (
            text.replace(guard_line + "\n", "")
            .replace(exit_line, decoy + "\n" + exit_line + "\n" + guard_line)
        )
        errors = validate_workflow_texts(sabotaged, ROOT)
        self.assertTrue(
            any("docs" in error and "early exit" in error for error in errors),
            errors,
        )

    def test_checked_in_docs_guard_order_passes(self) -> None:
        self.assertEqual(
            ws12_workflow_contracts.validate_code_style_docs_guard_order(
                self.workflows[CODE_STYLE_WORKFLOW]
            ),
            [],
        )

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
            self.sabotage(
                STRESS_WORKFLOW,
                '- "crates/kernel/ironclaw_turns/**"',
                '- "crates/**"',
            ),
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


class LibsqlScriptedMemoryJobSabotageTests(unittest.TestCase):
    """#7360: the libsql-scripted-memory job's lifecycle and artifact contract.

    REQUIRED_MARKERS pins tokens file-wide, so it cannot tell which job a
    token belongs to; this class pins them inside the ONE job block. The
    checked-in workflow is GREEN under the contract. Sabotages that touch a
    #7360-fixed invariant start from `fixed_workflows()`, which normalizes
    EITHER source shape (the historical RED one: --operations 2, an EXIT
    trap that kills without waiting, the server log embedded in every
    per-script upload, final readiness curls without timeout flags; or the
    checked-in GREEN one) to one canonical compliant fixture, and then break
    exactly one invariant, so each test fails for its own reason rather than
    piggybacking on the checked-in failures. Sabotages of invariants both
    shapes already share mutate the checked-in text directly.
    """

    SERVER_LOG_LINE = "            target/ironclaw-stress/libsql-scripted-server.log"
    SERVER_LOG_STEP = """\
      - name: Upload libsql scripted server log
        if: always()
        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
        with:
          name: ironclaw-stress-libsql-scripted-server-log
          path: |
            target/ironclaw-stress/libsql-scripted-server.log
          if-no-files-found: error
"""
    SERVER_LOG_STEP_NAME = "Upload libsql scripted server log"
    # The canonical trap is single-quoted so the expansion is DELAYED:
    # `$server_pid` is read when the trap fires, not when it is registered.
    KILL_ONLY_TRAP = "trap 'kill \"$server_pid\" 2>/dev/null || true' EXIT"
    CANONICAL_TRAP = (
        "trap 'kill \"$server_pid\" 2>/dev/null || true; "
        "wait \"$server_pid\" 2>/dev/null || true' EXIT"
    )
    FINAL_HEALTH_PROBE = (
        "          curl -fsS http://127.0.0.1:18080/api/health >/dev/null"
    )
    FINAL_HEALTH_PROBE_FIXED = (
        "          curl -fsS --connect-timeout 5 --max-time 10 "
        "http://127.0.0.1:18080/api/health >/dev/null"
    )
    FINAL_SESSION_PROBE = (
        "          curl -fsS \\\n"
        "            -H \"Authorization: Bearer $IRONCLAW_REBORN_WEBUI_TOKEN\" \\\n"
        "            http://127.0.0.1:18080/api/webchat/v2/session >/dev/null"
    )
    FINAL_SESSION_PROBE_FIXED = (
        "          curl -fsS --connect-timeout 5 --max-time 10 \\\n"
        "            -H \"Authorization: Bearer $IRONCLAW_REBORN_WEBUI_TOKEN\" \\\n"
        "            http://127.0.0.1:18080/api/webchat/v2/session >/dev/null"
    )

    def setUp(self) -> None:
        self.workflows = load_workflows(ROOT)

    def sabotage(self, workflow: str, old: str, new: str) -> dict[str, str]:
        mutated = copy.deepcopy(self.workflows)
        replaced = mutated[workflow].replace(old, new)
        self.assertNotEqual(replaced, mutated[workflow], f"no-op sabotage: {old!r}")
        mutated[workflow] = replaced
        return mutated

    def errors_for(self, workflows: dict[str, str]) -> list[str]:
        return validate_libsql_scripted_memory_job(workflows[STRESS_WORKFLOW])

    def _fixed_workflow_text(self, text: str) -> str:
        """One workflow's text with the #7360 contract fixes applied.

        Accepts EITHER source shape — the original RED one (--operations 2,
        a kill-only EXIT trap, the server log embedded in all three
        per-script uploads) or the checked-in GREEN one (--operations 4,
        kill+wait trap, its own log step) — and normalizes it to one
        canonical compliant shape. Every transform is conditional, so
        re-applying it to an already-fixed text is a no-op (idempotent), and
        an unexpected shape fails clearly instead of silently passing.
        """
        block, detail = extract_job_block(text, LIBSQL_SCRIPTED_MEMORY_JOB)
        self.assertIsNotNone(block, detail)
        fixed = block

        if "--operations 2 \\" in fixed:
            fixed = fixed.replace("--operations 2 \\", "--operations 4 \\")

        if self.KILL_ONLY_TRAP in fixed:
            fixed = fixed.replace(self.KILL_ONLY_TRAP, self.CANONICAL_TRAP)
        elif self.CANONICAL_TRAP not in fixed:
            self.fail(
                "unexpected EXIT trap shape: expected either the kill-only "
                f"RED form or the canonical {self.CANONICAL_TRAP!r}"
            )

        embedded = fixed.count(self.SERVER_LOG_LINE + "\n")
        if embedded == 3:
            fixed = fixed.replace(self.SERVER_LOG_LINE + "\n", "")
            fixed = fixed.rstrip("\n") + "\n\n" + self.SERVER_LOG_STEP
        elif embedded == 1:
            heading = next(
                (
                    match
                    for match in STEP_HEADING.finditer(fixed)
                    if match.group("name").strip() == self.SERVER_LOG_STEP_NAME
                ),
                None,
            )
            if heading is None:
                self.fail(
                    "checked-in GREEN shape must contain the server-log "
                    "upload step"
                )
            following = STEP_HEADING.search(fixed, heading.end())
            end = following.start() if following else len(fixed)
            fixed = fixed[: heading.start()] + self.SERVER_LOG_STEP + fixed[end:]
        else:
            self.fail(
                f"unexpected server-log upload shape: {embedded} occurrences "
                f"of {self.SERVER_LOG_LINE!r} (expected 3 embedded in the "
                "per-script uploads or 1 in its own step)"
            )

        if self.FINAL_HEALTH_PROBE in fixed:
            fixed = fixed.replace(self.FINAL_HEALTH_PROBE, self.FINAL_HEALTH_PROBE_FIXED)
        if self.FINAL_SESSION_PROBE in fixed:
            fixed = fixed.replace(
                self.FINAL_SESSION_PROBE, self.FINAL_SESSION_PROBE_FIXED
            )

        self.assertIn(self.SERVER_LOG_STEP.rstrip("\n"), fixed)
        start = text.find(block)
        self.assertNotEqual(start, -1, "extracted job block must be in the text")
        return text[:start] + fixed + text[start + len(block) :]

    def fixed_workflows(self) -> dict[str, str]:
        """Checked-in text plus the #7360 contract fixes (see
        `_fixed_workflow_text`). Every sabotage starts from this fixture so
        it fails for exactly the invariant it breaks."""
        mutated = copy.deepcopy(self.workflows)
        mutated[STRESS_WORKFLOW] = self._fixed_workflow_text(
            self.workflows[STRESS_WORKFLOW]
        )
        return mutated

    def red_shape_text(self) -> str:
        """The original RED job shape, reconstructed from the checked-in text
        by undoing the #7360 fixes: --operations 2, a kill-only EXIT trap,
        the server log embedded in all three per-script uploads (no separate
        artifact), and final probes without timeout flags."""
        text = self.workflows[STRESS_WORKFLOW]
        block, detail = extract_job_block(text, LIBSQL_SCRIPTED_MEMORY_JOB)
        self.assertIsNotNone(block, detail)
        red = block
        red = red.replace("--operations 4 \\", "--operations 2 \\")
        red = red.replace(self.CANONICAL_TRAP, self.KILL_ONLY_TRAP)
        red = red.replace(self.FINAL_HEALTH_PROBE_FIXED, self.FINAL_HEALTH_PROBE)
        red = red.replace(self.FINAL_SESSION_PROBE_FIXED, self.FINAL_SESSION_PROBE)
        heading = next(
            match
            for match in STEP_HEADING.finditer(red)
            if match.group("name").strip() == self.SERVER_LOG_STEP_NAME
        )
        following = STEP_HEADING.search(red, heading.end())
        end = following.start() if following else len(red)
        red = red[: heading.start()] + red[end:]
        red = red.rstrip("\n") + "\n"
        for script in ("memory_roundtrip", "memory_grow", "memory_mixed"):
            red = red.replace(
                f"ironclaw-stress-libsql-scripted-{script}/report.txt\n",
                f"ironclaw-stress-libsql-scripted-{script}/report.txt\n"
                + self.SERVER_LOG_LINE
                + "\n",
            )
        start = text.find(block)
        self.assertNotEqual(start, -1, "extracted job block must be in the text")
        return text[:start] + red + text[start + len(block) :]

    def test_checked_in_libsql_job_meets_the_contract(self) -> None:
        """The checked-in workflow satisfies the full scoped contract —
        lifecycle, exact runner flags, and artifact upload policy."""
        self.assertEqual(self.errors_for(self.workflows), [])

    def test_fixed_workflow_meets_the_contract(self) -> None:
        """Guard the fixture: if the fixed shape stopped satisfying the
        checker, every sabotage below would fail on fixture construction
        rather than on the invariant it breaks."""
        self.assertEqual(self.errors_for(self.fixed_workflows()), [])

    def test_fixture_is_idempotent_across_red_and_green_shapes(self) -> None:
        """`_fixed_workflow_text` must normalize EITHER source shape — the
        original RED job or the checked-in GREEN one — to one canonical
        compliant fixture, and re-applying it must be a no-op. The RED shape
        must stay RED before normalization and pass after it."""
        green_text = self._fixed_workflow_text(self.workflows[STRESS_WORKFLOW])
        self.assertEqual(self._fixed_workflow_text(green_text), green_text)
        green = copy.deepcopy(self.workflows)
        green[STRESS_WORKFLOW] = green_text
        self.assertEqual(self.errors_for(green), [])
        self.assertIn("--operations 4 \\", green_text)
        self.assertIn(self.CANONICAL_TRAP, green_text)
        self.assertIn(self.SERVER_LOG_STEP.rstrip("\n"), green_text)
        self.assertIn(self.FINAL_HEALTH_PROBE_FIXED, green_text)
        self.assertIn(self.FINAL_SESSION_PROBE_FIXED, green_text)

        red_text = self.red_shape_text()
        self.assertNotEqual(red_text, self.workflows[STRESS_WORKFLOW])
        red = copy.deepcopy(self.workflows)
        red[STRESS_WORKFLOW] = red_text
        self.assertNotEqual(self.errors_for(red), [])
        fixed_red = self._fixed_workflow_text(red_text)
        self.assertEqual(self._fixed_workflow_text(fixed_red), fixed_red)
        red_fixed = copy.deepcopy(self.workflows)
        red_fixed[STRESS_WORKFLOW] = fixed_red
        self.assertEqual(self.errors_for(red_fixed), [])

    def test_operations_count_is_exactly_four(self) -> None:
        """The fix lands at exactly 4 — fewer under-samples, and 14 / 4.0
        are not a fuzzy '4'. Dropping the flag entirely must fail too."""
        fixed = self.fixed_workflows()
        for old, new in (
            ("--operations 4 \\", "--operations 3 \\"),
            ("--operations 4 \\", "--operations 14 \\"),
            ("--operations 4 \\", "--operations 4.0 \\"),
        ):
            with self.subTest(new=new):
                mutated = copy.deepcopy(fixed)
                mutated[STRESS_WORKFLOW] = mutated[STRESS_WORKFLOW].replace(old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any(
                        "--operations 4" in error
                        and "libsql-scripted-memory" in error
                        for error in errors
                    ),
                    errors,
                )

        dropped = copy.deepcopy(fixed)
        dropped[STRESS_WORKFLOW] = dropped[STRESS_WORKFLOW].replace(
            "              --operations 4 \\\n", ""
        )
        errors = self.errors_for(dropped)
        self.assertTrue(any("--operations 4" in error for error in errors), errors)

    def test_the_script_loop_is_fixed_and_complete(self) -> None:
        """The loop must enumerate exactly the three scripts in order — a
        parametrized list, a dropped scenario, or a reordered one all
        silently change what the matrix measures."""
        for old, new in (
            (
                "for script in memory_roundtrip memory_grow memory_mixed; do",
                "for script in memory_roundtrip memory_grow; do",
            ),
            (
                "for script in memory_roundtrip memory_grow memory_mixed; do",
                "for script in ${scripts}; do",
            ),
            (
                "for script in memory_roundtrip memory_grow memory_mixed; do",
                "for script in memory_roundtrip memory_mixed memory_grow; do",
            ),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("fixed sequential loop" in error for error in errors), errors
                )

    def test_the_zero_tolerance_gate_is_pinned(self) -> None:
        """--max-failure-rate must stay exactly 0 — 0.5 or 1 accepts failed
        or leaked scripted verdicts and the job still reports green."""
        for old, new in (
            ("--max-failure-rate 0 \\", "--max-failure-rate 0.5 \\"),
            ("--max-failure-rate 0 \\", "--max-failure-rate 1 \\"),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("--max-failure-rate 0" in error for error in errors), errors
                )

        dropped = self.sabotage(
            STRESS_WORKFLOW, "              --max-failure-rate 0 \\\n", ""
        )
        errors = self.errors_for(dropped)
        self.assertTrue(any("--max-failure-rate 0" in error for error in errors), errors)

    def test_runner_flags_cannot_be_satisfied_by_same_job_decoys(self) -> None:
        """Every matrix pin belongs to the guarded runner command itself.
        Relocating a flag before the loop must fail even when an exact decoy
        remains elsewhere in the same job."""
        text = self.workflows[STRESS_WORKFLOW]
        block, detail = extract_job_block(text, LIBSQL_SCRIPTED_MEMORY_JOB)
        self.assertIsNotNone(block, detail)
        start = text.find(block)
        self.assertNotEqual(start, -1)
        for required, replacement in (
            ("--operations 4", "--operations 3"),
            (
                "--api-scripted-doc-sizes 4096,32768,131072,1048576",
                "--api-scripted-doc-sizes 4096,32768",
            ),
            ("--max-failure-rate 0", "--max-failure-rate 0.5"),
            ("--api-hot-writers 2", "--api-hot-writers 3"),
            ("--mock-llm-bind 127.0.0.1:19090", "--mock-llm-bind 127.0.0.1:19091"),
            ("--api-poll-interval-ms 10000", "--api-poll-interval-ms 2000"),
            ("--api-terminal-timeout-ms 120000", "--api-terminal-timeout-ms 60000"),
            ("--max-p95-ms 120000", "--max-p95-ms 60000"),
        ):
            with self.subTest(required=required):
                loop = ws12_workflow_contracts.LIBSQL_SCRIPTED_LOOP_BODY.search(block)
                self.assertIsNotNone(loop)
                commands = ws12_workflow_contracts.extract_continued_commands(
                    loop.group("body"), "target/release/ironclaw_stress"
                )
                self.assertEqual(len(commands), 1)
                runner = commands[0]
                mutated_runner = runner.replace(required, replacement, 1)
                self.assertNotEqual(mutated_runner, runner)
                mutated_block = block.replace(runner, mutated_runner, 1)
                loop_start = mutated_block.find(
                    "for script in memory_roundtrip memory_grow memory_mixed; do"
                )
                self.assertNotEqual(loop_start, -1)
                mutated_block = (
                    mutated_block[:loop_start]
                    + f"          {required} \\\n"
                    + mutated_block[loop_start:]
                )
                mutated = text[:start] + mutated_block + text[start + len(block) :]
                errors = validate_libsql_scripted_memory_job(mutated)
                self.assertTrue(
                    any(required in error for error in errors),
                    errors,
                )


    def test_the_four_doc_sizes_are_exact(self) -> None:
        """Adding a fifth size or changing any value breaks the pinned
        small-to-large latency curve."""
        for old, new in (
            (
                "--api-scripted-doc-sizes 4096,32768,131072,1048576 \\",
                "--api-scripted-doc-sizes 4096,32768,131072,1048576,8388608 \\",
            ),
            (
                "--api-scripted-doc-sizes 4096,32768,131072,1048576 \\",
                "--api-scripted-doc-sizes 4096,32768,131072,2097152 \\",
            ),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("4096,32768,131072,1048576" in error for error in errors),
                    errors,
                )

    def test_hot_writers_is_exactly_two(self) -> None:
        """--api-hot-writers must stay exactly 2 — 2.0/20 are not fuzzy
        '2', and a dropped flag is a regression too."""
        for old, new in (
            ("              --api-hot-writers 2 \\", "              --api-hot-writers 3 \\"),
            ("              --api-hot-writers 2 \\", "              --api-hot-writers 20 \\"),
            ("              --api-hot-writers 2 \\", "              --api-hot-writers 2.0 \\"),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("--api-hot-writers 2" in error for error in errors), errors
                )

        dropped = self.sabotage(
            STRESS_WORKFLOW, "              --api-hot-writers 2 \\\n", ""
        )
        errors = self.errors_for(dropped)
        self.assertTrue(
            any("--api-hot-writers 2" in error for error in errors), errors
        )

    def test_mock_llm_bind_is_pinned_to_19090(self) -> None:
        """The mock sidecar must bind where the server's LLM base_url
        points: 127.0.0.1:19090."""
        for old, new in (
            (
                "              --mock-llm-bind 127.0.0.1:19090 \\",
                "              --mock-llm-bind 127.0.0.1:19091 \\",
            ),
            (
                "              --mock-llm-bind 127.0.0.1:19090 \\",
                "              --mock-llm-bind 127.0.0.1:18080 \\",
            ),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any(
                        "--mock-llm-bind" in error and "19090" in error
                        for error in errors
                    ),
                    errors,
                )

    def test_poll_interval_is_exactly_10000ms(self) -> None:
        """--api-poll-interval-ms must stay 10000 — the README's 2000ms was
        the regression this pin exists to catch."""
        for old, new in (
            (
                "              --api-poll-interval-ms 10000 \\",
                "              --api-poll-interval-ms 2000 \\",
            ),
            (
                "              --api-poll-interval-ms 10000 \\",
                "              --api-poll-interval-ms 5000 \\",
            ),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("--api-poll-interval-ms 10000" in error for error in errors),
                    errors,
                )

    def test_terminal_timeout_is_exactly_120000ms(self) -> None:
        """--api-terminal-timeout-ms must stay 120000 — a shorter cap turns
        slow-but-healthy scripted terminals into false failures."""
        for old, new in (
            (
                "              --api-terminal-timeout-ms 120000 \\",
                "              --api-terminal-timeout-ms 60000 \\",
            ),
            (
                "              --api-terminal-timeout-ms 120000 \\",
                "              --api-terminal-timeout-ms 30000 \\",
            ),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("--api-terminal-timeout-ms 120000" in error for error in errors),
                    errors,
                )

    def test_p95_ceiling_is_exactly_120000ms(self) -> None:
        """--max-p95-ms must stay 120000 — both a tighter and a looser
        ceiling change what the matrix enforces."""
        for old, new in (
            ("              --max-p95-ms 120000 \\", "              --max-p95-ms 60000 \\"),
            ("              --max-p95-ms 120000 \\", "              --max-p95-ms 1200000 \\"),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any("--max-p95-ms 120000" in error for error in errors), errors
                )

    def test_loop_continues_after_a_failed_invocation(self) -> None:
        """A failed script must not abort the loop under set -e: the runner
        call's `|| failed=1` tail is what lets the later scripts still run
        and upload their evidence. `failed=0` must initialize the
        accumulator (set -u would abort on an unset variable) and the step
        must `exit "$failed"` after the loop."""
        fixed = self.fixed_workflows()

        bare = copy.deepcopy(fixed)
        bare[STRESS_WORKFLOW] = bare[STRESS_WORKFLOW].replace(
            '2> "${outdir}/report.txt" || failed=1',
            '2> "${outdir}/report.txt"',
        )
        errors = self.errors_for(bare)
        self.assertTrue(
            any("must not abort the loop" in error for error in errors), errors
        )

        no_init = copy.deepcopy(fixed)
        no_init[STRESS_WORKFLOW] = no_init[STRESS_WORKFLOW].replace(
            "          failed=0\n", ""
        )
        errors = self.errors_for(no_init)
        self.assertTrue(any("failed=0" in error for error in errors), errors)

        no_exit = copy.deepcopy(fixed)
        no_exit[STRESS_WORKFLOW] = no_exit[STRESS_WORKFLOW].replace(
            '          exit "$failed"\n', ""
        )
        errors = self.errors_for(no_exit)
        self.assertTrue(any('exit "$failed"' in error for error in errors), errors)

    def test_each_script_outdir_is_created_inside_the_loop(self) -> None:
        """`mkdir -p "${outdir}"` must run per script, before the
        invocation — the per-script upload paths must exist even when the
        invocation fails and the step continues."""
        fixed = self.fixed_workflows()
        no_outdir = copy.deepcopy(fixed)
        no_outdir[STRESS_WORKFLOW] = no_outdir[STRESS_WORKFLOW].replace(
            '            mkdir -p "${outdir}"\n', ""
        )
        errors = self.errors_for(no_outdir)
        self.assertTrue(
            any("mkdir -p" in error and "outdir" in error for error in errors), errors
        )

    def test_relocating_mkdir_breaks_the_loop_structure(self) -> None:
        """`mkdir -p "${outdir}"` must stay inside the loop, after the
        outdir= assignment and before the guarded invocation. Moved outside
        the loop (before `failed=0`) it no longer runs per script — the
        upload paths are missing exactly when the invocation failed; moved
        before the assignment or after the invocation it runs at the wrong
        moment and the runner writes into a directory that does not exist
        yet (or creates it too late for the redirects)."""
        fixed = self.fixed_workflows()

        outside = copy.deepcopy(fixed)
        outside[STRESS_WORKFLOW] = outside[STRESS_WORKFLOW].replace(
            "          failed=0\n"
            "          for script in memory_roundtrip memory_grow memory_mixed; do\n"
            '            outdir="target/ironclaw-stress/'
            'ironclaw-stress-libsql-scripted-${script}"\n'
            '            mkdir -p "${outdir}"\n',
            '            mkdir -p "${outdir}"\n'
            "          failed=0\n"
            "          for script in memory_roundtrip memory_grow memory_mixed; do\n"
            '            outdir="target/ironclaw-stress/'
            'ironclaw-stress-libsql-scripted-${script}"\n',
        )
        errors = self.errors_for(outside)
        self.assertTrue(
            any(
                "mkdir -p" in error and "outdir" in error and "inside the loop" in error
                for error in errors
            ),
            errors,
        )

        before_assign = copy.deepcopy(fixed)
        before_assign[STRESS_WORKFLOW] = before_assign[STRESS_WORKFLOW].replace(
            '            outdir="target/ironclaw-stress/'
            'ironclaw-stress-libsql-scripted-${script}"\n'
            '            mkdir -p "${outdir}"\n',
            '            mkdir -p "${outdir}"\n'
            '            outdir="target/ironclaw-stress/'
            'ironclaw-stress-libsql-scripted-${script}"\n',
        )
        errors = self.errors_for(before_assign)
        self.assertTrue(
            any(
                "in order" in error and "mkdir -p" in error and "outdir" in error
                for error in errors
            ),
            errors,
        )

        after_guard = copy.deepcopy(fixed)
        text = after_guard[STRESS_WORKFLOW].replace(
            '            outdir="target/ironclaw-stress/'
            'ironclaw-stress-libsql-scripted-${script}"\n'
            '            mkdir -p "${outdir}"\n',
            '            outdir="target/ironclaw-stress/'
            'ironclaw-stress-libsql-scripted-${script}"\n',
        )
        after_guard[STRESS_WORKFLOW] = text.replace(
            '              2> "${outdir}/report.txt" || failed=1\n'
            "          done\n",
            '              2> "${outdir}/report.txt" || failed=1\n'
            '            mkdir -p "${outdir}"\n'
            "          done\n",
        )
        errors = self.errors_for(after_guard)
        self.assertTrue(
            any(
                "in order" in error and "mkdir -p" in error and "outdir" in error
                for error in errors
            ),
            errors,
        )

    def test_exit_before_done_fails_loudly(self) -> None:
        """`exit "$failed"` must come after the loop's `done` — moved inside
        the loop it fails the job right after the first failed script,
        skipping the later scripts' evidence uploads."""
        fixed = self.fixed_workflows()
        mutated = copy.deepcopy(fixed)
        mutated[STRESS_WORKFLOW] = mutated[STRESS_WORKFLOW].replace(
            '              2> "${outdir}/report.txt" || failed=1\n'
            "          done\n"
            '          exit "$failed"\n',
            '              2> "${outdir}/report.txt" || failed=1\n'
            '          exit "$failed"\n'
            "          done\n",
        )
        errors = self.errors_for(mutated)
        self.assertTrue(
            any(
                'exit "$failed"' in error and "after the loop" in error
                for error in errors
            ),
            errors,
        )

    def test_upload_steps_always_run_and_fail_on_missing_evidence(self) -> None:
        """Every upload step needs `if: always()` (a failed matrix step must
        still upload the evidence produced before it failed) and
        `if-no-files-found: error` (an upload that silently carries nothing
        hides a lost run behind a green job)."""
        fixed = self.fixed_workflows()

        no_always = copy.deepcopy(fixed)
        no_always[STRESS_WORKFLOW] = no_always[STRESS_WORKFLOW].replace(
            "      - name: Upload libsql scripted memory_roundtrip artifacts\n"
            "        if: always()\n",
            "      - name: Upload libsql scripted memory_roundtrip artifacts\n",
        )
        errors = self.errors_for(no_always)
        self.assertTrue(
            any(
                "if: always()" in error and "memory-roundtrip" in error
                for error in errors
            ),
            errors,
        )

        no_error_on_missing = copy.deepcopy(fixed)
        no_error_on_missing[STRESS_WORKFLOW] = no_error_on_missing[
            STRESS_WORKFLOW
        ].replace(
            "target/ironclaw-stress/ironclaw-stress-libsql-scripted-"
            "memory_roundtrip/report.txt\n"
            "          if-no-files-found: error\n",
            "target/ironclaw-stress/ironclaw-stress-libsql-scripted-"
            "memory_roundtrip/report.txt\n"
            "          if-no-files-found: ignore\n",
        )
        errors = self.errors_for(no_error_on_missing)
        self.assertTrue(
            any(
                "if-no-files-found: error" in error and "memory-roundtrip" in error
                for error in errors
            ),
            errors,
        )

    def test_per_script_artifacts_carry_exactly_their_three_paths(self) -> None:
        """Each per-script artifact name must map to exactly its own
        outdir's summary.jsonl, summary.json, and report.txt — a swapped
        path points one script's upload at another script's evidence, and a
        missing path makes that evidence unrecoverable after a failure."""
        fixed = self.fixed_workflows()

        swapped = copy.deepcopy(fixed)
        swapped[STRESS_WORKFLOW] = swapped[STRESS_WORKFLOW].replace(
            "target/ironclaw-stress/ironclaw-stress-libsql-scripted-"
            "memory_roundtrip/report.txt",
            "target/ironclaw-stress/ironclaw-stress-libsql-scripted-"
            "memory_grow/report.txt",
        )
        errors = self.errors_for(swapped)
        self.assertTrue(
            any(
                "exactly the three" in error and "memory-roundtrip" in error
                for error in errors
            ),
            errors,
        )

        missing = copy.deepcopy(fixed)
        missing[STRESS_WORKFLOW] = missing[STRESS_WORKFLOW].replace(
            "target/ironclaw-stress/ironclaw-stress-libsql-scripted-"
            "memory_grow/summary.json\n",
            "",
        )
        errors = self.errors_for(missing)
        self.assertTrue(
            any(
                "exactly the three" in error and "memory-grow" in error
                for error in errors
            ),
            errors,
        )

    def test_readiness_probes_are_bounded(self) -> None:
        """An unbounded probe (`while true`, or a seq bound past the job
        timeout) hangs the job on a dead server instead of failing fast."""
        for old, new in (
            ("for _ in $(seq 1 120); do", "for _ in $(seq 1 999999999); do"),
            ("for _ in $(seq 1 120); do", "while true; do"),
        ):
            with self.subTest(new=new):
                mutated = self.sabotage(STRESS_WORKFLOW, old, new)
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any(
                        "bounded" in error
                        and ("probe" in error or "retry bound" in error)
                        for error in errors
                    ),
                    errors,
                )

        no_sleep = self.sabotage(STRESS_WORKFLOW, "            sleep 1\n", "")
        errors = self.errors_for(no_sleep)
        self.assertTrue(
            any("curl the endpoint, sleep, and break" in error for error in errors),
            errors,
        )

    def test_giving_up_after_the_retries_is_a_failure(self) -> None:
        """The final unconditional curl after each bounded loop is what makes
        'bounded' fail fast: without it, 120 failed tries degrade into a
        matrix run against a dead server. Removing either final probe from
        the compliant fixture must fail."""
        fixed = self.fixed_workflows()
        for name, probe in (
            ("health", self.FINAL_HEALTH_PROBE_FIXED),
            ("webchat session", self.FINAL_SESSION_PROBE_FIXED),
        ):
            with self.subTest(probe=name):
                mutated = copy.deepcopy(fixed)
                mutated[STRESS_WORKFLOW] = mutated[STRESS_WORKFLOW].replace(
                    probe + "\n", ""
                )
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any(
                        "final unconditional curl" in error and name in error
                        for error in errors
                    ),
                    errors,
                )

    def test_final_probes_require_explicit_timeouts(self) -> None:
        """The final curls must carry explicit --connect-timeout and
        --max-time flags: without them a wedged server hangs the job past the
        bounded loop instead of failing fast. Removing the flags from either
        final probe must fail — this is exactly the checked-in gap — and the
        values are pinned, so a dead-letter 50/100 is a regression, not a
        fix."""
        fixed = self.fixed_workflows()
        for name, fixed_probe, bare_probe in (
            ("health", self.FINAL_HEALTH_PROBE_FIXED, self.FINAL_HEALTH_PROBE),
            (
                "webchat session",
                self.FINAL_SESSION_PROBE_FIXED,
                self.FINAL_SESSION_PROBE,
            ),
        ):
            with self.subTest(probe=name):
                mutated = copy.deepcopy(fixed)
                self.assertIn(fixed_probe, mutated[STRESS_WORKFLOW])
                mutated[STRESS_WORKFLOW] = mutated[STRESS_WORKFLOW].replace(
                    fixed_probe, bare_probe
                )
                errors = self.errors_for(mutated)
                self.assertTrue(
                    any(
                        "final unconditional curl" in error
                        and name in error
                        and "--connect-timeout" in error
                        for error in errors
                    ),
                    errors,
                )

    def test_cleanup_kills_and_waits_for_the_server(self) -> None:
        """Kill without wait can leave the port bound when the step ends —
        the next run then collides with a zombie server. The trap must do
        both, and must exist at all. Sabotages mutate the canonical fixture
        (the raw checked-in text still carries a kill-only trap in a
        neighbouring job, so it cannot stand in for this job's trap)."""
        fixed = self.fixed_workflows()
        no_wait = copy.deepcopy(fixed)
        no_wait[STRESS_WORKFLOW] = no_wait[STRESS_WORKFLOW].replace(
            '; wait "$server_pid" 2>/dev/null || true', ""
        )
        errors = self.errors_for(no_wait)
        self.assertTrue(
            any("kill the server AND wait" in error for error in errors), errors
        )

        no_trap = copy.deepcopy(fixed)
        no_trap[STRESS_WORKFLOW] = no_trap[STRESS_WORKFLOW].replace(
            self.CANONICAL_TRAP, "# cleanup handled by the runner\n"
        )
        errors = self.errors_for(no_trap)
        self.assertTrue(
            any("register an EXIT trap" in error for error in errors), errors
        )

    def test_server_log_has_its_own_artifact(self) -> None:
        """A failed run's log must live in ONE upload: embedded in every
        per-script upload it is split three ways and each copy is incomplete
        on its own (the RED shape this fixture normalizes away)."""
        fixed = self.fixed_workflows()
        self.assertIn(self.SERVER_LOG_STEP.rstrip("\n"), fixed[STRESS_WORKFLOW])

        removed = copy.deepcopy(fixed)
        removed[STRESS_WORKFLOW] = removed[STRESS_WORKFLOW].replace(
            self.SERVER_LOG_STEP.rstrip("\n") + "\n", ""
        )
        errors = self.errors_for(removed)
        self.assertTrue(
            any(
                "own artifact" in error
                and "ironclaw-stress-libsql-scripted-server-log" in error
                for error in errors
            ),
            errors,
        )

        renamed = copy.deepcopy(fixed)
        renamed[STRESS_WORKFLOW] = renamed[STRESS_WORKFLOW].replace(
            "name: ironclaw-stress-libsql-scripted-server-log",
            "name: ironclaw-stress-libsql-scripted-serverlog",
        )
        errors = self.errors_for(renamed)
        self.assertTrue(any("own artifact" in error for error in errors), errors)

    def test_per_script_artifacts_are_unique_and_exclude_the_server_log(self) -> None:
        """Three scripts must map to three distinct artifact identities, and
        the server log must live in its own upload, not in each of them."""
        fixed = self.fixed_workflows()

        duplicated = copy.deepcopy(fixed)
        duplicated[STRESS_WORKFLOW] = duplicated[STRESS_WORKFLOW].replace(
            "name: ironclaw-stress-libsql-scripted-memory-grow",
            "name: ironclaw-stress-libsql-scripted-memory-roundtrip",
        )
        errors = self.errors_for(duplicated)
        self.assertTrue(
            any("exactly three distinct per-script artifacts" in error for error in errors),
            errors,
        )

        leaked = copy.deepcopy(fixed)
        leaked[STRESS_WORKFLOW] = leaked[STRESS_WORKFLOW].replace(
            "ironclaw-stress-libsql-scripted-memory_roundtrip/report.txt\n",
            "ironclaw-stress-libsql-scripted-memory_roundtrip/report.txt\n"
            + self.SERVER_LOG_LINE
            + "\n",
        )
        errors = self.errors_for(leaked)
        self.assertTrue(
            any(
                "must not include" in error and "libsql-scripted-server.log" in error
                for error in errors
            ),
            errors,
        )

    def test_the_job_stays_scheduled_and_manually_triggerable(self) -> None:
        """The gate must keep both trigger paths — dropping
        workflow_dispatch strands the daily scan unreachable by hand."""
        mutated = self.sabotage(
            STRESS_WORKFLOW,
            "if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'",
            "if: github.event_name == 'schedule'",
        )
        errors = self.errors_for(mutated)
        self.assertTrue(
            any("schedule and workflow_dispatch" in error for error in errors), errors
        )

    def test_the_libsql_volume_profile_is_pinned_inside_the_job(self) -> None:
        text = self.workflows[STRESS_WORKFLOW]
        block, detail = extract_job_block(text, LIBSQL_SCRIPTED_MEMORY_JOB)
        self.assertIsNotNone(block, detail)
        start = text.find(block)
        self.assertNotEqual(start, -1)
        expected = 'profile = "hosted-single-tenant-volume"'
        replacement = 'profile = "hosted-single-tenant-provisioned"'

        inside = copy.deepcopy(self.workflows)
        inside[STRESS_WORKFLOW] = (
            f"{expected}\n"
            + text[:start]
            + block.replace(expected, replacement, 1)
            + text[start + len(block) :]
        )
        errors = self.errors_for(inside)
        self.assertTrue(any(expected in error for error in errors), errors)

        outside = copy.deepcopy(self.workflows)
        outside[STRESS_WORKFLOW] = f'{replacement}\n{text}'
        self.assertEqual(self.errors_for(outside), [])


    def postgres_errors_for(self, workflows: dict[str, str]) -> list[str]:
        return validate_postgres_scripted_parity(workflows[STRESS_WORKFLOW])

    # The runner picks each operation's document size by cycling the
    # --api-scripted-doc-sizes list (doc_size_for), so N operations exercise
    # only the first N sizes. The historical `--operations 2` ran just the
    # 4096/32768 buckets; parity requires at least one operation per
    # configured size.
    POSTGRES_SCRIPTED_FLAGS = (
        "--operations 4 \\\n"
        "            --api-scripted-tool memory_roundtrip \\"
    )

    def test_checked_in_postgres_scripted_leg_reaches_every_size(self) -> None:
        """The Postgres parity leg must run at least as many operations as
        configured doc sizes — with fewer, doc_size_for() cycles only the
        first buckets and the largest sizes are never exercised."""
        self.assertEqual(self.postgres_errors_for(self.workflows), [])
        self.assertEqual(self.postgres_errors_for(self.fixed_workflows()), [])

    def test_postgres_scripted_leg_has_zero_failure_tolerance(self) -> None:
        """One failed operation out of 32 is below five percent, so only an
        exact zero gate fails closed on leaks and lost durable writes."""
        for bad in ("0.05", "0.5"):
            with self.subTest(value=bad):
                mutated = self.sabotage(
                    STRESS_WORKFLOW,
                    "            --max-failure-rate 0 \\\n",
                    f"            --max-failure-rate {bad} \\\n",
                )
                errors = self.postgres_errors_for(mutated)
                self.assertTrue(
                    any("--max-failure-rate 0" in error for error in errors),
                    errors,
                )

        dropped = self.sabotage(
            STRESS_WORKFLOW, "            --max-failure-rate 0 \\\n", ""
        )
        errors = self.postgres_errors_for(dropped)
        self.assertTrue(
            any("--max-failure-rate 0" in error for error in errors), errors
        )

    def test_postgres_failure_gate_rejects_same_job_decoy(self) -> None:
        text = self.workflows[STRESS_WORKFLOW]
        block, detail = extract_job_block(text, "postgres-api-capacity")
        self.assertIsNotNone(block, detail)
        start = text.find(block)
        commands = [
            command
            for command in ws12_workflow_contracts.extract_continued_commands(
                block, "target/release/ironclaw_stress"
            )
            if "--api-scripted-tool memory_roundtrip" in command
        ]
        self.assertEqual(len(commands), 1)
        runner = commands[0]
        mutated_runner = runner.replace(
            "--max-failure-rate 0", "--max-failure-rate 0.05", 1
        )
        self.assertNotEqual(mutated_runner, runner)
        mutated_block = block.replace(
            runner,
            '          echo "--max-failure-rate 0"\n' + mutated_runner,
            1,
        )
        self.assertNotEqual(mutated_block, block)
        mutated = text[:start] + mutated_block + text[start + len(block) :]
        errors = validate_postgres_scripted_parity(mutated)
        self.assertTrue(
            any("--max-failure-rate 0" in error for error in errors), errors
        )


    def test_postgres_operations_below_size_count_fail_loudly(self) -> None:
        """`--operations 2`/`3` leave configured doc sizes unexercised —
        both must fail. Values at or above the size count stay legal (the
        cycling then covers every bucket), pinning the >= contract rather
        than an exact value."""
        for ops in ("2", "3"):
            with self.subTest(operations=ops):
                mutated = self.sabotage(
                    STRESS_WORKFLOW,
                    self.POSTGRES_SCRIPTED_FLAGS,
                    f"--operations {ops} \\\n"
                    "            --api-scripted-tool memory_roundtrip \\",
                )
                errors = self.postgres_errors_for(mutated)
                self.assertTrue(
                    any(
                        "postgres-api-capacity" in error
                        and "--operations" in error
                        for error in errors
                    ),
                    errors,
                )

        generous = self.sabotage(
            STRESS_WORKFLOW,
            self.POSTGRES_SCRIPTED_FLAGS,
            "--operations 14 \\\n"
            "            --api-scripted-tool memory_roundtrip \\",
        )
        self.assertEqual(self.postgres_errors_for(generous), [])

    def test_dropping_the_postgres_doc_sizes_fails_loudly(self) -> None:
        """Without the exact four-size list the operations count has nothing
        to reach — the parity leg must configure the same small-to-large
        curve as the libsql matrix."""
        mutated = self.sabotage(
            STRESS_WORKFLOW,
            "            --api-scripted-doc-sizes 4096,32768,131072,1048576 \\\n"
            "            --api-hot-writers 2 \\",
            "            --api-hot-writers 2 \\",
        )
        errors = self.postgres_errors_for(mutated)
        self.assertTrue(
            any(
                "postgres-api-capacity" in error
                and "4096,32768,131072,1048576" in error
                for error in errors
            ),
            errors,
        )

    def test_postgres_parity_failures_reach_the_top_level_contract(self) -> None:
        """The parity validator must stay wired into `validate_workflow_texts`
        — main()'s entry point is the only consumer CI actually runs."""
        mutated = self.sabotage(
            STRESS_WORKFLOW,
            self.POSTGRES_SCRIPTED_FLAGS,
            "--operations 2 \\\n"
            "            --api-scripted-tool memory_roundtrip \\",
        )
        errors = validate_workflow_texts(mutated, ROOT)
        self.assertTrue(
            any(
                "postgres-api-capacity" in error and "--operations" in error
                for error in errors
            ),
            errors,
        )

    def test_zero_or_two_matching_job_blocks_refuse(self) -> None:
        """The scoped contract must resolve exactly one job block — a deleted
        job and a duplicated job key both make the pins unassertable."""
        text = self.workflows[STRESS_WORKFLOW]
        self.assertIn("  libsql-scripted-memory:\n", text)

        deleted = text.replace("  libsql-scripted-memory:\n", "")
        errors = validate_libsql_scripted_memory_job(deleted)
        self.assertTrue(any("expected exactly one" in error for error in errors), errors)

        duplicated = text.replace(
            "  libsql-scripted-memory:\n",
            "  libsql-scripted-memory:\n  libsql-scripted-memory:\n",
            1,
        )
        errors = validate_libsql_scripted_memory_job(duplicated)
        self.assertTrue(
            any("expected exactly one" in error and "found 2" in error for error in errors),
            errors,
        )

    def test_libsql_job_failures_reach_the_top_level_contract(self) -> None:
        """The validator must stay wired into `validate_workflow_texts` —
        main()'s entry point is the only consumer CI actually runs."""
        mutated = self.sabotage(
            STRESS_WORKFLOW,
            "for script in memory_roundtrip memory_grow memory_mixed; do",
            "for script in memory_roundtrip memory_grow; do",
        )
        errors = validate_workflow_texts(mutated, ROOT)
        self.assertTrue(any("fixed sequential loop" in error for error in errors), errors)


if __name__ == "__main__":
    unittest.main()
