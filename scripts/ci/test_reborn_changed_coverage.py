#!/usr/bin/env python3
"""Workflow-contract tests for thresholded changed-line and branch coverage."""

from __future__ import annotations

import ast
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

import reborn_changed_coverage as gate

ROOT = Path(__file__).resolve().parents[2]


class ChangedCoverageWorkflowTests(unittest.TestCase):
    def test_committed_manifest_passes_standalone_validation(self):
        result = subprocess.run(
            [
                sys.executable,
                "scripts/ci/reborn_changed_coverage.py",
                "--manifest",
                "tests/integration/changed-coverage-exemptions.toml",
                "--validate-manifest-only",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("changed coverage manifest valid", result.stdout)

    def test_gate_sabotage_harness_passes(self):
        result = subprocess.run(
            ["bash", "scripts/ci/test-reborn-changed-coverage.sh"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        success = re.search(
            r"all ([1-9][0-9]*) changed-coverage self-tests passed",
            result.stdout,
        )
        self.assertIsNotNone(success, result.stdout)

    def test_committed_policy_restores_original_line_floor(self):
        with (ROOT / "tests/integration/changed-coverage-exemptions.toml").open(
            "rb"
        ) as handle:
            policy = tomllib.load(handle)["policy"]

        self.assertEqual(policy["line_percent"], 90.0)
        self.assertEqual(policy["branch_percent"], 0.0)

    def test_reborn_workflow_runs_gate_and_preserves_machine_report(self):
        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("python3 scripts/ci/test_reborn_changed_coverage.py", workflow)
        self.assertIn("python3 scripts/ci/reborn_changed_coverage.py", workflow)
        self.assertIn("tests/integration/changed-coverage-exemptions.toml", workflow)
        self.assertIn("github.event.pull_request.base.sha", workflow)
        self.assertIn("github.event.pull_request.head.sha", workflow)
        self.assertIn('--head "$HEAD_SHA"', workflow)
        self.assertNotIn('--head "$(git rev-parse HEAD)"', workflow)
        self.assertIn("github.event.merge_group.base_sha", workflow)
        self.assertIn("reborn-changed-coverage.json", workflow)
        gate_step = workflow.split(
            "- name: Gate changed Reborn lines and branches", maxsplit=1
        )[1].split("\n      - name:", maxsplit=1)[0]
        self.assertIn('--head "$HEAD_SHA"', gate_step)
        self.assertNotIn(
            "--threshold",
            gate_step,
            "thresholds come from the reviewed manifest, not a workflow override",
        )
        self.assertRegex(
            workflow,
            re.compile(
                r"- name: Post sticky coverage comment\n"
                r"\s+if: .*always\(\).*github\.event_name == 'pull_request'",
            ),
        )

    def test_workflow_wires_the_base_coverage_lookup_it_needs(self):
        """The subtraction is inert unless CI can actually reach the artifact.

        `--fetch-base-coverage` fails closed, so a missing `actions: read` or a
        missing token degrades to the strict denominator *silently as far as
        the check colour goes* — green, just never subtracting. That is exactly
        the shape (#6963, #6946) that must be pinned in a test rather than
        trusted.
        """

        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )
        job = workflow.split("\n  coverage-report:", maxsplit=1)[1].split(
            "\n  critical-mutation:", maxsplit=1
        )[0]
        gate_step = job.split(
            "- name: Gate changed Reborn lines and branches", maxsplit=1
        )[1].split("\n      - name:", maxsplit=1)[0]

        self.assertIn("--fetch-base-coverage", gate_step)
        self.assertIn("GH_TOKEN:", gate_step)
        permissions = job.split("permissions:", maxsplit=1)[1].split("steps:")[0]
        self.assertIn("actions: read", permissions)

    def test_base_coverage_workflow_constant_names_a_real_workflow(self):
        """A renamed workflow file must trip a test, not go quietly dark.

        The run lookup is scoped to this file name. If the workflow is renamed
        and the constant is not, every lookup 404s, every run falls back to the
        strict denominator, and nothing ever goes red to say so.
        """

        self.assertTrue(
            (ROOT / ".github/workflows" / gate.BASE_COVERAGE_WORKFLOW).is_file(),
            f"{gate.BASE_COVERAGE_WORKFLOW} does not exist; the base-coverage "
            "run lookup would 404 on every PR and silently stop subtracting",
        )

    def test_base_coverage_artifact_constant_matches_what_ci_uploads(self):
        """The artifact name and member must match the upload step verbatim."""

        workflow = (ROOT / ".github/workflows/reborn-tests.yml").read_text(
            encoding="utf-8"
        )
        upload = workflow.split("- name: Upload merged coverage report", maxsplit=1)[
            1
        ].split("\n  critical-mutation:", maxsplit=1)[0]

        self.assertIn(f"name: {gate.BASE_COVERAGE_ARTIFACT}", upload)
        self.assertIn(gate.BASE_COVERAGE_MEMBER, upload)


class PreimageMappingTests(unittest.TestCase):
    """A changed line may only inherit from the line it actually replaced."""

    def _parse(self, diff: str) -> gate.DiffChanges:
        return gate.parse_diff(diff, gate.ProductionPaths(ROOT))

    PATH = "crates/ironclaw_assistant/src/lib.rs"

    def test_modified_lines_pair_one_to_one(self):
        changes = self._parse(
            f"diff --git a/{self.PATH} b/{self.PATH}\n"
            f"--- a/{self.PATH}\n"
            f"+++ b/{self.PATH}\n"
            "@@ -10,2 +10,2 @@\n"
            "-old first\n"
            "-old second\n"
            "+new first\n"
            "+new second\n"
        )

        self.assertEqual(changes.added, {self.PATH: {10, 11}})
        self.assertEqual(
            changes.preimage,
            {(self.PATH, 10): (self.PATH, 10), (self.PATH, 11): (self.PATH, 11)},
        )

    def test_a_grown_region_inherits_only_as_far_as_it_replaced(self):
        """One line re-wrapped into three: the surplus tail is genuinely new."""

        changes = self._parse(
            f"diff --git a/{self.PATH} b/{self.PATH}\n"
            f"--- a/{self.PATH}\n"
            f"+++ b/{self.PATH}\n"
            "@@ -10 +10,3 @@\n"
            "-one long line\n"
            "+wrapped(\n"
            "+    argument,\n"
            "+)\n"
        )

        self.assertEqual(changes.preimage, {(self.PATH, 10): (self.PATH, 10)})
        self.assertNotIn((self.PATH, 11), changes.preimage)
        self.assertNotIn((self.PATH, 12), changes.preimage)

    def test_a_pure_addition_has_no_preimage(self):
        changes = self._parse(
            f"diff --git a/{self.PATH} b/{self.PATH}\n"
            "--- /dev/null\n"
            f"+++ b/{self.PATH}\n"
            "@@ -0,0 +1,2 @@\n"
            "+pub fn added() {}\n"
            "+pub fn also_added() {}\n"
        )

        self.assertEqual(changes.added, {self.PATH: {1, 2}})
        self.assertEqual(changes.preimage, {})

    def test_a_rename_resolves_the_preimage_to_the_old_path(self):
        old = "crates/ironclaw_assistant/src/old_home.rs"
        changes = self._parse(
            f"diff --git a/{old} b/{self.PATH}\n"
            "similarity index 92%\n"
            f"rename from {old}\n"
            f"rename to {self.PATH}\n"
            f"--- a/{old}\n"
            f"+++ b/{self.PATH}\n"
            "@@ -7 +7 @@\n"
            "-old body\n"
            "+new body\n"
        )

        self.assertEqual(changes.preimage, {(self.PATH, 7): (old, 7)})

    def test_hunk_content_that_looks_like_a_header_is_body_not_header(self):
        """`--- a/…` inside a hunk is a removed line, not a new file header."""

        changes = self._parse(
            f"diff --git a/{self.PATH} b/{self.PATH}\n"
            f"--- a/{self.PATH}\n"
            f"+++ b/{self.PATH}\n"
            "@@ -3 +3 @@\n"
            "--- a/decoy\n"
            "+++ b/decoy\n"
        )

        self.assertEqual(changes.added, {self.PATH: {3}})
        self.assertEqual(changes.preimage, {(self.PATH, 3): (self.PATH, 3)})


class BaseCoverageDecisionTests(unittest.TestCase):
    """Only a positively observed uncovered pre-image may leave the gate."""

    PATH = "crates/ironclaw_assistant/src/lib.rs"

    def _base(self, hits: dict[int, int]) -> gate.Coverage:
        return gate.Coverage(lines={self.PATH: dict(hits)}, branches={})

    def test_uncovered_at_base_is_excluded(self):
        origin = gate.preexisting_uncovered_origin(
            {(self.PATH, 10): (self.PATH, 8)}, self._base({8: 0}), self.PATH, 10
        )

        self.assertEqual(origin, (self.PATH, 8))

    def test_covered_at_base_still_gates(self):
        origin = gate.preexisting_uncovered_origin(
            {(self.PATH, 10): (self.PATH, 8)}, self._base({8: 1}), self.PATH, 10
        )

        self.assertIsNone(origin)

    def test_a_preimage_with_no_da_record_still_gates(self):
        """Not instrumented at base is not the same as uncovered at base."""

        origin = gate.preexisting_uncovered_origin(
            {(self.PATH, 10): (self.PATH, 8)}, self._base({9: 0}), self.PATH, 10
        )

        self.assertIsNone(origin)

    def test_no_preimage_still_gates(self):
        origin = gate.preexisting_uncovered_origin(
            {}, self._base({10: 0}), self.PATH, 10
        )

        self.assertIsNone(origin)

    def test_the_render_cap_never_shrinks_the_machine_record(self):
        """Only the summary rendering is capped; the JSON keeps every line.

        A cap on the audit record itself would be the rubber stamp this report
        exists to prevent, so the limit is applied at the print site alone.
        """

        source = (ROOT / "scripts/ci/reborn_changed_coverage.py").read_text(
            encoding="utf-8"
        )
        tree = ast.parse(source)
        users = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Name) and node.id == "PREEXISTING_PRINT_LIMIT"
        ]

        self.assertTrue(users, "the render cap must actually be used")
        self.assertIn(
            '"preexisting_uncovered": preexisting_uncovered',
            source,
            "the machine report must serialise the complete, uncapped list",
        )

    def test_an_lcov_with_no_da_records_is_refused_as_base_coverage(self):
        with tempfile.TemporaryDirectory() as temp:
            empty = Path(temp) / "base.lcov"
            empty.write_text("TN:\nSF:/nowhere.rs\nend_of_record\n", encoding="utf-8")

            with self.assertRaises(gate.BaseCoverageUnavailable):
                gate.load_base_coverage(empty, ROOT, "fixture")


class UninstrumentableScaffoldingTests(unittest.TestCase):
    def test_enum_bodies_classify_whole_including_struct_variants(self) -> None:
        source = (
            "use thiserror::Error;\n"
            "\n"
            "#[derive(Debug, Error)]\n"
            "pub enum InboundTurnError {\n"
            "    #[error(\"busy\")]\n"
            "    ThreadBusy,\n"
            "    /// Doc on a data variant.\n"
            "    #[error(\"turn submission failed: {error}\")]\n"
            "    TurnSubmissionFailed { error: String },\n"
            "    Multi {\n"
            "        reason: String,\n"
            "        code: u16,\n"
            "    },\n"
            "    Discriminantish = 4,\n"
            "}\n"
            "\n"
            "pub fn executable_after_enum() -> u16 {\n"
            "    7\n"
            "}\n"
        )
        lines = gate.mechanically_uninstrumentable_lines(source)
        # Every line of the enum item (4-16) classifies, including the
        # struct-shaped variant declaration that used to defeat the escape.
        for enum_line in range(4, 17):
            self.assertIn(enum_line, lines, f"enum body line {enum_line} must classify")
        # The function after the enum stays instrumentable — the classifier
        # must exit the enum at its closing brace, not swallow the file. (Its
        # bare closing brace on line 19 classifies via the pre-existing
        # symbol-only rule, which is correct and not this feature's doing.)
        self.assertNotIn(17, lines)
        self.assertNotIn(18, lines)

    def test_enum_with_where_clause_header_spanning_lines(self) -> None:
        source = (
            "pub enum Wrapped<T>\n"
            "where\n"
            "    T: Clone,\n"
            "{\n"
            "    Some(T),\n"
            "    None,\n"
            "}\n"
            "fn after() {}\n"
        )
        lines = gate.mechanically_uninstrumentable_lines(source)
        for enum_line in range(1, 8):
            self.assertIn(enum_line, lines, f"line {enum_line} must classify")
        self.assertNotIn(8, lines)

    """Lines LLVM cannot emit a coverage region for.

    Both cases below are move-PR regressions: a file whose ONLY changed lines
    are scaffolding must not read as "absent from coverage" or "contributed no
    instrumented lines", because one unclassified line defeats the escape and
    the gate then fails closed on a pure relocation.
    """

    def test_inner_attributes_are_scaffolding(self) -> None:
        """`#![forbid(unsafe_code)]` — every crate root carries one.

        The classifier matched outer `#[...]` but not the inner `#![...]`
        form, so a moved `lib.rs` of pure `mod`/`pub use` declarations still
        had exactly one unclassified line and stayed in the failure bucket.
        """
        source = (
            "//! docs\n"
            "\n"
            "#![forbid(unsafe_code)]\n"
            "#![allow(\n"
            "    clippy::all\n"
            ")]\n"
            "\n"
            "mod payload;\n"
            "pub use payload::{A, B};\n"
        )
        self.assertEqual(
            gate.mechanically_uninstrumentable_lines(source),
            {1, 2, 3, 4, 5, 6, 7, 8, 9},
        )

    def test_const_and_static_items_are_scaffolding(self) -> None:
        """A `const` initializer is compile-time; it has no coverage region.

        Repointing an asset path is the entire content of a package move, and
        it lands exclusively on `const X: &str = include_str!(...)` lines.
        """
        source = (
            'const MANIFEST: &str = include_str!("../../../packages/x/manifest.toml");\n'
            'pub(super) const WASM: &[u8] = include_bytes!("../wasm/x.wasm");\n'
            "static TABLE: &[u8] = &[\n"
            "    1, 2, 3,\n"
            "];\n"
            "fn live() -> u8 {\n"
            "    1\n"
            "}\n"
        )
        uninstrumentable = gate.mechanically_uninstrumentable_lines(source)
        self.assertEqual(uninstrumentable & {1, 2, 3, 4, 5}, {1, 2, 3, 4, 5})
        self.assertNotIn(7, uninstrumentable, "a real function body stays measurable")


class ChangedCoverageDiscoveryTests(unittest.TestCase):
    """The gate must resolve production paths from the crate inventory.

    A `crates/ironclaw_*` pattern matches nothing once crates move into family
    directories, and this gate's "nothing matched" outcome is a silent pass
    ("no Reborn production lines added", exit 0). These pin the discovery so
    that shape cannot come back (docs/reborn/target-architecture/CHECKLIST.md
    WS10, #6963).
    """

    def test_no_string_literal_reintroduces_the_flat_crate_keying(self) -> None:
        """No *operative* string may key on `crates/ironclaw_*` again.

        Checked over the AST rather than the raw text so that prose explaining
        the retired pattern — in this gate's own module docstring, and in the
        comments that say why it was retired — does not read as a violation.
        """
        source = (ROOT / "scripts/ci/reborn_changed_coverage.py").read_text(
            encoding="utf-8"
        )
        tree = ast.parse(source)
        docstrings = {
            id(node.body[0].value)
            for node in ast.walk(tree)
            if isinstance(
                node, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)
            )
            and node.body
            and isinstance(node.body[0], ast.Expr)
            and isinstance(node.body[0].value, ast.Constant)
            and isinstance(node.body[0].value.value, str)
        }

        offenders = [
            node.value
            for node in ast.walk(tree)
            if isinstance(node, ast.Constant)
            and isinstance(node.value, str)
            and id(node) not in docstrings
            and ("crates/ironclaw_*" in node.value or "crates/ironclaw_[" in node.value)
        ]

        self.assertEqual(
            offenders,
            [],
            "the flat-tree keying is what goes silently dark under family "
            "directories; discovery must come from the crate inventory",
        )

    def test_diff_pathspec_covers_the_whole_crates_root(self) -> None:
        """The pathspec must not be built from the *current* inventory.

        A per-crate `<crate>/src/**` pathspec cannot name the SOURCE of a
        rename whose crate directory moved, so `-M` has nothing to pair and a
        pure `git mv` reads as thousands of added lines. Classification still
        happens per path in `parse_diff`, so widening costs no precision.
        """
        production = gate.ProductionPaths(ROOT)
        self.assertEqual(production.diff_pathspecs(), [gate.CRATES_ROOT])

    def test_a_crate_directory_move_adds_no_production_lines(self) -> None:
        """The regression this widening exists for.

        Moves a crate wholesale — the WS2 package-colocation shape — and
        asserts the gate sees no added production lines. With a per-crate
        pathspec this returned every surviving line of the moved file.
        """
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)

            def git(*args: str) -> None:
                subprocess.run(
                    ["git", *args], cwd=root, check=True, capture_output=True
                )

            git("init", "-q", "-b", "main")
            git("config", "user.email", "t@example.com")
            git("config", "user.name", "t")
            # Clear the discovery floor.
            for index in range(25):
                crate = root / "crates" / f"ironclaw_filler{index}"
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "ironclaw_filler{index}"\n', encoding="utf-8"
                )
                (crate / "src" / "lib.rs").write_text("pub fn f() {}\n", encoding="utf-8")
            moved = root / "crates" / "ironclaw_mover"
            (moved / "src").mkdir(parents=True)
            (moved / "Cargo.toml").write_text(
                '[package]\nname = "ironclaw_mover"\n', encoding="utf-8"
            )
            body = "".join(f"pub fn f{n}() {{ let _ = {n}; }}\n" for n in range(200))
            (moved / "src" / "lib.rs").write_text(body, encoding="utf-8")
            git("add", "-A")
            git("commit", "-qm", "base")
            base = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                cwd=root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()

            destination = root / "crates" / "extensions" / "packages" / "mover"
            destination.parent.mkdir(parents=True, exist_ok=True)
            git("mv", "crates/ironclaw_mover", str(destination.relative_to(root)))
            git("commit", "-qm", "move the crate")

            production = gate.ProductionPaths(root)
            changes = gate.parse_diff(
                gate.git_diff(root, base, "HEAD", production), production
            )
            self.assertEqual(
                {path: sorted(lines) for path, lines in changes.added.items()},
                {},
                "a pure crate-directory move must contribute no added lines",
            )

    def test_production_classification_survives_family_nesting(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for index in range(25):
                crate = root / "crates/substrates" / f"ironclaw_nested{index}"
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "ironclaw_nested{index}"\n', encoding="utf-8"
                )
            production = gate.ProductionPaths(root)

            self.assertTrue(
                production.is_production(
                    "crates/substrates/ironclaw_nested0/src/lib.rs"
                )
            )
            # Attributable but not the crate's own `src/` — excluded, as the
            # old regex excluded `crates/ironclaw_safety/fuzz/src/main.rs`.
            self.assertFalse(
                production.is_production(
                    "crates/substrates/ironclaw_nested0/fuzz/src/main.rs"
                )
            )

    def test_unattributable_crate_path_is_refused_not_ignored(self) -> None:
        production = gate.ProductionPaths(ROOT)

        production.reject_unattributable("crates/AGENTS.md")
        production.reject_unattributable("src/main.rs")
        with self.assertRaises(gate.GateError):
            production.reject_unattributable("crates/not_a_crate/src/lib.rs")

    def test_missing_crate_tree_is_a_gate_error(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(gate.GateError):
                gate.ProductionPaths(Path(temp))


if __name__ == "__main__":
    unittest.main()
