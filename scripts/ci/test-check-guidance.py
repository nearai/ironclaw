#!/usr/bin/env python3
"""Self-test for `scripts/ci/check-guidance.py`.

A guidance gate that cannot fail is worse than no guidance gate: it converts
"nobody checked the docs" into "something checked them and they were fine".
So almost every case here is about the gate REFUSING — a dangling backticked
reference, a dead markdown link, a `paths:` glob that matches nothing (the
`.claude/rules/skills.md` failure this gate exists for), a crate missing from
its family table, a crate without a README, a family without an AGENTS.md, a
`CLAUDE.md` alias that is deleted from the index / a regular file / a symlink
to the wrong target (the 2026-08-06 audit's committed-deletion exploit), an
alias-exception row that stopped matching the tree, a `path-ok` marker being
read as covering its whole line, a suppression row that outlived its debt,
frontmatter the parser cannot trust, an unreadable guidance file, and
extraction floors that stop a broken scanner from reporting an empty pass as
clean. The legitimate-citation forms the extractor must NOT flag get their
own case, and the happy path runs against the real repository, last.

The gate reads the tracked set through `git ls-files -s`; these tests feed it
a written file instead (`--tracked-files`, where `<path> -> <target>` marks a
symlink), so a doctored tree costs a text edit rather than a fixture git
repository. Fixture runs patch the floor constants, empty the real
`KNOWN_MISSING` table, and shrink `ALIAS_REAL_FILE_EXCEPTIONS` to the root
row the fixture needs; the real-repository run patches nothing.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import os
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

SCRIPT = pathlib.Path(__file__).with_name("check-guidance.py")
REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]

_SPEC = importlib.util.spec_from_file_location("guidance_check", SCRIPT)
assert _SPEC and _SPEC.loader
GATE = importlib.util.module_from_spec(_SPEC)
# Registered before execution: `@dataclasses.dataclass` resolves a class's
# module through `sys.modules`, and on 3.12+ a hyphenated script loaded by path
# alone raises there instead of defining the dataclass.
sys.modules[_SPEC.name] = GATE
_SPEC.loader.exec_module(GATE)

# The same module instance the gate imported — patches on it are visible to
# the gate, and its inventory cache must be dropped around every fixture.
crate_tree = sys.modules["crate_tree"]


class GuidanceGateTests(unittest.TestCase):
    # -- fixture ----------------------------------------------------------
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._tmp.name)
        crate_tree.reset_inventory_cache()
        self.addCleanup(self._tmp.cleanup)
        self.addCleanup(crate_tree.reset_inventory_cache)

    def write(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def symlink(self, relative: str, target: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_symlink() or path.exists():
            path.unlink()
        os.symlink(target, path)

    def build_fixture(self) -> None:
        """A minimal, internally consistent guidance tree that passes.

        The root `CLAUDE.md` is a real file (covered by the root exception row
        `run_gate` patches in); the family `AGENTS.md` gets the
        `CLAUDE.md -> AGENTS.md` symlink alias the alias rule requires.
        """
        self.write("AGENTS.md", "Start at [the family](crates/core/AGENTS.md).\n")
        self.write("CLAUDE.md", "Read `docs/guide.md` and `crates/core/ironclaw_alpha/README.md`.\n")
        self.symlink("crates/core/CLAUDE.md", "AGENTS.md")
        self.write("docs/guide.md", "scanned like every docs/ page\n")
        # Navigation defines the published surface the scan must cover.
        self.write("docs/docs.json", '{"navigation": {"pages": ["guide"]}}\n')
        self.write(
            ".claude/rules/alpha.md",
            '---\npaths:\n  - "crates/**/*.rs"\n---\n# Alpha rule\n',
        )
        self.write(
            ".claude/skills/demo/SKILL.md",
            "---\nname: demo\ndescription: none\n---\nSee `crates/core/AGENTS.md`.\n",
        )
        self.write(
            "crates/core/AGENTS.md",
            "# core family\n\n"
            "| Crate | Charter |\n| --- | --- |\n"
            "| [`ironclaw_alpha`](./ironclaw_alpha) | records |\n"
            "| [`beta_pkg`](./ironclaw_beta) | glue |\n",
        )
        self.write(
            "crates/core/ironclaw_alpha/Cargo.toml",
            '[package]\nname = "ironclaw_alpha"\n',
        )
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry point `src/lib.rs`; charter pinned by `tests/charter.rs`.\n",
        )
        self.write("crates/core/ironclaw_alpha/src/lib.rs", "\n")
        self.write("crates/core/ironclaw_alpha/tests/charter.rs", "\n")
        self.write(
            "crates/core/ironclaw_beta/Cargo.toml", '[package]\nname = "beta_pkg"\n'
        )
        self.write(
            "crates/core/ironclaw_beta/README.md",
            "Driven by `ironclaw_alpha` (`src/lib.rs`).\n",
        )
        self.write("crates/core/ironclaw_beta/src/lib.rs", "\n")

    def tracked(self) -> list[str]:
        """The fixture's tracked set; symlinks appear as `<path> -> <target>`."""
        entries: list[str] = []
        for path in self.root.rglob("*"):
            if path.is_symlink():
                rel = path.relative_to(self.root).as_posix()
                entries.append(f"{rel} -> {os.readlink(path)}")
            elif path.is_file():
                entries.append(path.relative_to(self.root).as_posix())
        return sorted(entries)

    ROOT_ALIAS_EXCEPTION = {"CLAUDE.md": "root adapter (fixture)"}

    def run_gate(
        self,
        tracked: list[str] | None = None,
        known_missing: tuple | None = (),
        alias_exceptions: dict[str, str] | None = None,
        internal_guidance: tuple[str, ...] = (),
    ) -> tuple[int, str]:
        listing = self.root / "tracked-files.txt"
        listing.write_text(
            "\n".join(t for t in (tracked if tracked is not None else self.tracked())
                      if t != "tracked-files.txt")
            + "\n",
            encoding="utf-8",
        )
        patches = [
            mock.patch.object(GATE, "MIN_GUIDANCE_FILES", 1),
            mock.patch.object(GATE, "MIN_PATH_REFERENCES", 1),
            mock.patch.object(GATE, "MIN_RULE_GLOBS", 1),
            mock.patch.object(GATE, "MIN_ALIAS_PAIRS", 1),
            # Fixtures opt in per test; the production tuple names real-repo
            # pages the fixture tree lacks.
            mock.patch.object(GATE, "INTERNAL_GUIDANCE_PREFIXES", internal_guidance),
            mock.patch.object(
                GATE,
                "ALIAS_REAL_FILE_EXCEPTIONS",
                dict(self.ROOT_ALIAS_EXCEPTION)
                if alias_exceptions is None
                else alias_exceptions,
            ),
            mock.patch.object(crate_tree, "MIN_CRATE_DIRECTORIES", 1),
        ]
        if known_missing is not None:
            patches.append(mock.patch.object(GATE, "KNOWN_MISSING", known_missing))
        stderr, stdout = io.StringIO(), io.StringIO()
        with contextlib.ExitStack() as stack:
            for patch in patches:
                stack.enter_context(patch)
            stack.enter_context(contextlib.redirect_stderr(stderr))
            stack.enter_context(contextlib.redirect_stdout(stdout))
            crate_tree.reset_inventory_cache()
            code = GATE.main(
                ["--repo-root", str(self.root), "--tracked-files", str(listing)]
            )
        crate_tree.reset_inventory_cache()
        return code, stderr.getvalue() + stdout.getvalue()

    # -- the fixture itself must pass, or every refusal below is suspect --
    def test_fixture_passes(self) -> None:
        self.build_fixture()
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)
        self.assertIn("guidance: OK", output)

    # -- check 1: references must resolve ---------------------------------
    def test_dangling_backticked_reference_fails_naming_the_offender(self) -> None:
        self.build_fixture()
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry point `src/lib.rs`; see `crates/core/ironclaw_gamma/README.md`.\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/core/ironclaw_alpha/README.md", output)
        self.assertIn("crates/core/ironclaw_gamma/README.md", output)

    def test_dangling_markdown_link_fails(self) -> None:
        self.build_fixture()
        self.write("AGENTS.md", "Start at [the family](crates/gone/AGENTS.md).\n")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/gone/AGENTS.md", output)

    def test_crate_relative_and_context_citations_are_not_flagged(self) -> None:
        """The legitimate citation forms measured on the live tree: crate-
        relative (`tests/charter.rs` in the crate's README), crate-context
        ("`ironclaw_alpha` (`src/lib.rs`)" in beta's README), fenced blocks,
        placeholders, the `✎` glyph, the `path-ok` marker, and MCP method
        names. The fixture passing (above) covers the first two; this case
        stacks the suppressions on one document."""
        self.build_fixture()
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry point `src/lib.rs`.\n\n"
            "```text\ncrates/fenced/does_not_exist.rs\n```\n\n"
            "Template: `crates/core/<crate>/README.md`.\n"
            "✎ 2026-08-06: `crates/core/ironclaw_removed` was deleted.\n"
            "Example `crates/core/ironclaw_example/README.md` "
            "<!-- check-guidance: path-ok -->\n"
            "The MCP `tools/list` method.\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_prefix_and_module_relative_citations_resolve(self) -> None:
        self.build_fixture()
        self.write("crates/core/ironclaw_alpha/src/hosted_mcp_discovery.rs", "\n")
        self.write("crates/core/ironclaw_alpha/src/inner/tests/policy.rs", "\n")
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Vocabulary confined to `src/hosted_mcp_` modules; a policy test "
            "belongs in `tests/policy.rs`.\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_path_ok_marker_covers_one_reference_not_the_line(self) -> None:
        """The marker vouches for the reference immediately before it — a
        dangling path slipped onto an already-marked line (before or after
        the marked reference) must still fail. The 2026-08-06 audit rode a
        fresh dangling path in on exactly this line-blanket behavior."""
        self.build_fixture()
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry point `src/lib.rs`.\n"
            "See `crates/core/ironclaw_before/README.md` and the deliberate "
            "`crates/core/ironclaw_marked/README.md` "
            "<!-- check-guidance: path-ok --> "
            "plus `crates/core/ironclaw_after/README.md`.\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/core/ironclaw_before/README.md", output)
        self.assertIn("crates/core/ironclaw_after/README.md", output)
        self.assertNotIn("crates/core/ironclaw_marked/README.md", output)

    def test_suppression_row_that_outlived_its_debt_fails(self) -> None:
        self.build_fixture()
        row = GATE.Suppression(
            doc="crates/core/ironclaw_alpha/README.md",
            token="src/lib.rs",
            reason="stale — this reference resolves",
        )
        code, output = self.run_gate(known_missing=(row,))
        self.assertEqual(code, 1)
        self.assertIn("Delete the row", output)

    def test_suppressed_reference_warns_but_passes(self) -> None:
        self.build_fixture()
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry point `src/lib.rs`; create `src/myprovider.rs` to extend.\n",
        )
        row = GATE.Suppression(
            doc="crates/core/ironclaw_alpha/README.md",
            token="src/myprovider.rs",
            reason="instructional example",
        )
        code, output = self.run_gate(known_missing=(row,))
        self.assertEqual(code, 0, output)
        self.assertIn("grandfathered dangling reference", output)
        self.assertIn("src/myprovider.rs", output)

    def test_duplicate_known_missing_rows_fail(self) -> None:
        """Two rows excusing the same reference is a table bug: one debt, one
        row. The dedup guard must fail rather than silently collapsing them."""
        self.build_fixture()
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry point `src/lib.rs`; create `src/myprovider.rs` to extend.\n",
        )
        row = GATE.Suppression(
            doc="crates/core/ironclaw_alpha/README.md",
            token="src/myprovider.rs",
            reason="instructional example",
        )
        twin = GATE.Suppression(
            doc="crates/core/ironclaw_alpha/README.md",
            token="src/myprovider.rs",
            reason="same debt, second row",
        )
        code, output = self.run_gate(known_missing=(row, twin))
        self.assertEqual(code, 1)
        self.assertIn("one debt, one row", output)

    # -- check 2: rule triggers must be live -------------------------------
    def test_rule_glob_matching_nothing_fails_naming_rule_and_glob(self) -> None:
        """The `.claude/rules/skills.md` failure: a trigger naming a file
        that does not exist, so the rule never fires."""
        self.build_fixture()
        self.write(
            ".claude/rules/alpha.md",
            '---\npaths:\n  - "crates/**/*.rs"\n  - "crates/core/ironclaw_alpha/src/bundled_skills.rs"\n---\n',
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn(".claude/rules/alpha.md", output)
        self.assertIn("bundled_skills.rs", output)
        self.assertIn("never fires", output)

    def test_brace_alternation_trigger_counts_as_live(self) -> None:
        """A `{rs,toml}` trigger matching real tracked files passes check 2
        end-to-end — the false-dead-rule shape the translator fix closes."""
        self.build_fixture()
        self.write(
            ".claude/rules/alpha.md",
            '---\npaths:\n  - "crates/**/*.{rs,toml}"\n---\n',
        )
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_skill_frontmatter_paths_are_checked_too(self) -> None:
        self.build_fixture()
        self.write(
            ".claude/skills/demo/SKILL.md",
            '---\nname: demo\npaths:\n  - "crates/nowhere/**"\n---\n',
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn(".claude/skills/demo/SKILL.md", output)
        self.assertIn("crates/nowhere/**", output)

    def test_unterminated_frontmatter_refuses(self) -> None:
        self.build_fixture()
        self.write(".claude/rules/alpha.md", "---\npaths:\n  - \"crates/**\"\n")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("never closed", output)

    def test_paths_key_with_no_globs_refuses(self) -> None:
        self.build_fixture()
        self.write(".claude/rules/alpha.md", "---\npaths:\n---\n# empty\n")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("lists no globs", output)

    def test_inline_paths_value_refuses_rather_than_skipping(self) -> None:
        self.build_fixture()
        self.write(".claude/rules/alpha.md", '---\npaths: "crates/**"\n---\n')
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("block lists only", output)

    def test_glob_translation_handles_the_repo_shapes(self) -> None:
        cases = {
            ("crates/**/*.rs", "crates/core/ironclaw_alpha/src/lib.rs"): True,
            ("crates/**/*.rs", "crates/top.rs"): True,
            ("crates/**/*.rs", "tests/lib.rs"): False,
            ("**/Cargo.toml", "Cargo.toml"): True,
            ("**/Cargo.toml", "crates/core/ironclaw_alpha/Cargo.toml"): True,
            ("crates/core/ironclaw_alpha/**", "crates/core/ironclaw_alpha/src/lib.rs"): True,
            ("crates/core/ironclaw_alpha/**", "crates/core/ironclaw_alpha"): False,
            ("skills/**", "skills/demo/SKILL.md"): True,
            ("crates/core/ironclaw_alpha/src/lib.rs", "crates/core/ironclaw_alpha/src/lib.rs"): True,
            # Brace alternation: a legitimate `{rs,toml}` trigger is live, not
            # a dead literal (#7306 review).
            ("crates/**/*.{rs,toml}", "crates/core/ironclaw_alpha/src/lib.rs"): True,
            ("crates/**/*.{rs,toml}", "crates/core/ironclaw_alpha/Cargo.toml"): True,
            ("crates/**/*.{rs,toml}", "crates/core/ironclaw_alpha/README.md"): False,
            ("{crates,tests}/**", "tests/lib.rs"): True,
            ("{crates,tests}/**", "docs/lib.rs"): False,
            ("crates/{core/**,README.md}", "crates/core/ironclaw_alpha/src/lib.rs"): True,
            ("crates/{core/**,README.md}", "crates/README.md"): True,
            ("src/{a,{b,c}}.rs", "src/b.rs"): True,
            ("src/{a,{b,c}}.rs", "src/d.rs"): False,
            # An unmatched brace stays a literal character.
            ("src/un{matched.rs", "src/un{matched.rs"): True,
        }
        for (glob, path), expected in cases.items():
            with self.subTest(glob=glob, path=path):
                self.assertIs(
                    GATE.glob_to_regex(glob).match(path) is not None, expected
                )

    # -- checks 3 and 4: family tables and READMEs --------------------------
    def test_crate_missing_from_family_table_fails(self) -> None:
        self.build_fixture()
        self.write(
            "crates/core/AGENTS.md",
            "# core family\n\n| Crate | Charter |\n| --- | --- |\n"
            "| [`ironclaw_alpha`](./ironclaw_alpha) | records |\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/core/ironclaw_beta", output)
        self.assertIn("crates/core/AGENTS.md", output)
        self.assertIn("family table no longer covers", output)

    def test_incidental_mention_in_another_row_is_not_coverage(self) -> None:
        """A crate whose own row is gone must fail even when another row's
        charter prose still names it — only the identity column counts."""
        self.build_fixture()
        self.write(
            "crates/core/AGENTS.md",
            "# core family\n\n| Crate | Charter |\n| --- | --- |\n"
            "| [`ironclaw_alpha`](./ironclaw_alpha) | records; routes to "
            "`ironclaw_beta` for storage |\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/core/ironclaw_beta", output)
        self.assertIn("identity (first) column", output)

    def test_table_match_accepts_package_name_for_renamed_directories(self) -> None:
        """`beta_pkg` (the package) is in the table, `ironclaw_beta` (the
        directory) is not — the fixture passing at all pins this."""
        self.build_fixture()
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_family_without_agents_md_fails(self) -> None:
        self.build_fixture()
        self.write("crates/solo/ironclaw_gamma/Cargo.toml", '[package]\nname = "ironclaw_gamma"\n')
        self.write("crates/solo/ironclaw_gamma/README.md", "gamma\n")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/solo/AGENTS.md", output)

    def test_crate_without_readme_fails(self) -> None:
        self.build_fixture()
        tracked = [t for t in self.tracked() if t != "crates/core/ironclaw_beta/README.md"]
        code, output = self.run_gate(tracked=tracked)
        self.assertEqual(code, 1)
        self.assertIn("crates/core/ironclaw_beta", output)
        self.assertIn("no tracked README.md", output)

    # -- check 5: the CLAUDE.md alias rule ---------------------------------
    def test_alias_deleted_from_the_index_fails_naming_the_pair(self) -> None:
        """The audit's exploit: a committed symlink deletion left the gate
        green. Removing the alias from the *tracked set* (the disk copy may
        even survive) must fail, naming both halves of the pair."""
        self.build_fixture()
        tracked = [
            t for t in self.tracked() if not t.startswith("crates/core/CLAUDE.md")
        ]
        code, output = self.run_gate(tracked=tracked)
        self.assertEqual(code, 1)
        self.assertIn("crates/core/AGENTS.md has no tracked CLAUDE.md", output)
        self.assertIn("ln -s AGENTS.md crates/core/CLAUDE.md", output)

    def test_alias_as_regular_file_fails(self) -> None:
        """A copied real file drifts; only a mode-120000 symlink (or a named
        exception row) is a legal alias."""
        self.build_fixture()
        (self.root / "crates/core/CLAUDE.md").unlink()
        self.write("crates/core/CLAUDE.md", "# core family\n\ncopied bytes\n")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn(
            "crates/core/CLAUDE.md is tracked as a regular file, not a symlink",
            output,
        )
        self.assertIn("ALIAS_REAL_FILE_EXCEPTIONS", output)

    def test_alias_with_wrong_target_fails(self) -> None:
        """The target must be the bare sibling string; a link that resolves
        to *an* AGENTS.md via another route still fails. (`../../AGENTS.md`
        reaches the fixture root's real file, so the content read stays
        healthy and the refusal is the alias check's, not a read error.)"""
        self.build_fixture()
        self.symlink("crates/core/CLAUDE.md", "../../AGENTS.md")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("'../../AGENTS.md'", output)
        self.assertIn("must target the sibling `AGENTS.md`", output)

    def test_root_real_file_exception_row_is_load_bearing(self) -> None:
        """The fixture's root CLAUDE.md is a real file and passes only via
        the patched exception row; with the row gone the gate must flag it."""
        self.build_fixture()
        code, output = self.run_gate(alias_exceptions={})
        self.assertEqual(code, 1)
        self.assertIn("CLAUDE.md is tracked as a regular file", output)

    def test_alias_exception_rows_must_match_reality(self) -> None:
        self.build_fixture()
        # A row excusing an alias that is actually a symlink is stale.
        code, output = self.run_gate(
            alias_exceptions={
                **self.ROOT_ALIAS_EXCEPTION,
                "crates/core/CLAUDE.md": "pretend reason",
            }
        )
        self.assertEqual(code, 1)
        self.assertIn("tracked as a symlink, yet it is a named real-file", output)
        # A row whose directory has no AGENTS.md at all excuses nothing.
        code, output = self.run_gate(
            alias_exceptions={
                **self.ROOT_ALIAS_EXCEPTION,
                "crates/core/ironclaw_alpha/CLAUDE.md": "stale row",
            }
        )
        self.assertEqual(code, 1)
        self.assertIn("no AGENTS.md sits beside it", output)
        self.assertIn("delete it", output)

    # -- tests/ tree gets the same treatment as crates/ --------------------
    def test_tests_tree_guidance_is_discovered(self) -> None:
        """`discover_guidance`'s `tests/` branch (TESTS_PREFIX /
        TESTS_GUIDANCE_BASENAMES) has no fixture otherwise — only the real
        repository exercises it, and that check only asserts a clean exit.
        A dangling reference inside a `tests/**/AGENTS.md` must be caught the
        same way a `crates/**/AGENTS.md` one is."""
        self.build_fixture()
        self.write(
            "tests/integration/AGENTS.md",
            "Support lives at `tests/integration/support/gone.rs`.\n",
        )
        self.symlink("tests/integration/CLAUDE.md", "AGENTS.md")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("tests/integration/AGENTS.md", output)
        self.assertIn("tests/integration/support/gone.rs", output)

    def test_tests_tree_alias_is_enforced(self) -> None:
        """The alias rule walks `tests/**/AGENTS.md` too (`check_claude_aliases`
        matches `crates/` OR `TESTS_PREFIX`), but no fixture built a tests-tree
        AGENTS/CLAUDE pair — every existing alias fixture is under `crates/`.
        A regression that narrowed the alias walk back to `crates/` only would
        still pass every fixture test before this one."""
        self.build_fixture()
        self.write("tests/integration/AGENTS.md", "# integration guidance\n")
        self.symlink("tests/integration/CLAUDE.md", "AGENTS.md")
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

        # Deleting the tracked alias fails, naming the pair.
        tracked = [
            t
            for t in self.tracked()
            if not t.startswith("tests/integration/CLAUDE.md")
        ]
        code, output = self.run_gate(tracked=tracked)
        self.assertEqual(code, 1)
        self.assertIn(
            "tests/integration/AGENTS.md has no tracked CLAUDE.md", output
        )

        # Retargeting it away from the bare sibling string fails too (points
        # at a real AGENTS.md two levels up, so the read itself stays
        # healthy and the refusal is the alias check's, not a read error).
        self.symlink("tests/integration/CLAUDE.md", "../../AGENTS.md")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("must target the sibling `AGENTS.md`", output)

    # -- the gate must not fail open ---------------------------------------
    def test_unreadable_guidance_file_refuses(self) -> None:
        self.build_fixture()
        tracked = self.tracked()
        target = self.root / "crates/core/ironclaw_alpha/README.md"
        target.unlink()
        target.mkdir()  # tracked as a file, unreadable as one on disk
        code, output = self.run_gate(tracked=tracked)
        self.assertEqual(code, 1)
        self.assertIn("cannot read guidance file", output)

    def test_missing_root_guidance_refuses(self) -> None:
        self.build_fixture()
        tracked = [t for t in self.tracked() if t != "CLAUDE.md"]
        code, output = self.run_gate(tracked=tracked)
        self.assertEqual(code, 1)
        self.assertIn("CLAUDE.md is not tracked", output)

    def test_extraction_floor_refuses_an_empty_scan(self) -> None:
        self.build_fixture()
        with mock.patch.object(GATE, "MIN_PATH_REFERENCES", 10_000):
            listing = self.root / "tracked-files.txt"
            listing.write_text("\n".join(self.tracked()) + "\n", encoding="utf-8")
            stderr = io.StringIO()
            with (
                mock.patch.object(GATE, "MIN_GUIDANCE_FILES", 1),
                mock.patch.object(GATE, "INTERNAL_GUIDANCE_PREFIXES", ()),
                mock.patch.object(GATE, "KNOWN_MISSING", ()),
                mock.patch.object(crate_tree, "MIN_CRATE_DIRECTORIES", 1),
                contextlib.redirect_stderr(stderr),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                crate_tree.reset_inventory_cache()
                code = GATE.main(
                    ["--repo-root", str(self.root), "--tracked-files", str(listing)]
                )
            crate_tree.reset_inventory_cache()
        self.assertEqual(code, 1)
        self.assertIn("refusing rather than verifying almost nothing", stderr.getvalue())

    def test_unterminated_fence_refuses(self) -> None:
        self.build_fixture()
        self.write(
            "crates/core/ironclaw_alpha/README.md",
            "Entry `src/lib.rs`.\n```text\nnever closed\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("unterminated ``` fence", output)

    # -- the docs/ surface -------------------------------------------------
    def test_docs_page_with_dangling_backticked_path_fails(self) -> None:
        """A published page naming a dead repo path is the drift class that
        motivated extending the scan (#7317)."""
        self.build_fixture()
        self.write(
            "docs/api/responses.mdx",
            "See `crates/core/ironclaw_gone/src/lib.rs` for details.\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("crates/core/ironclaw_gone/src/lib.rs", output)

    def test_docs_markdown_links_are_not_references(self) -> None:
        """Mintlify link targets are site routes, not repo paths — the link
        extractor is off for docs/ so neither form below is a claim."""
        self.build_fixture()
        self.write(
            "docs/using/cli.mdx",
            "Read [the CLI guide](/using/cli) and [service](service).\n"
            "Also [a relative page](../capabilities/configuration).\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_docs_mdx_comment_marker_suppresses_one_reference(self) -> None:
        """`{/* check-guidance: path-ok */}` is the MDX form of the marker;
        it vouches for the one reference immediately before it and no other."""
        self.build_fixture()
        self.write(
            "docs/extensions/building.mdx",
            "The retired `crates/core/ironclaw_gone/src/lib.rs` "
            "{/* check-guidance: path-ok */} is an example; "
            "`crates/core/ironclaw_also_gone/` is still checked.\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertNotIn("ironclaw_gone/src/lib.rs", output)
        self.assertIn("crates/core/ironclaw_also_gone/", output)

    def test_docs_mdx_multiline_comment_hides_its_content(self) -> None:
        """Paths inside a multi-line MDX comment block (the doc-fact marker
        shape) are not references."""
        self.build_fixture()
        self.write(
            "docs/api/policy.mdx",
            "Real prose with `crates/core/ironclaw_alpha/README.md`.\n"
            "{/* doc-fact:example\n"
            "`crates/core/ironclaw_gone/src/lib.rs`\n"
            "*/}\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_docs_zh_mirror_is_discovered(self) -> None:
        """The zh/ locale tree carries the same backticked repo paths as the
        English pages and is scanned the same way."""
        self.build_fixture()
        self.write(
            "docs/zh/index.md",
            "参见 `crates/core/ironclaw_gone/src/lib.rs`。\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("docs/zh/index.md", output)

    def test_docs_historical_archives_are_excluded_but_contracts_are_not(self) -> None:
        """docs/internal/ archives are excluded; the living contract corpus
        inside is scanned."""
        self.build_fixture()
        self.write(
            "docs/internal/plans/2026-01-01-old-plan.md",
            "We will edit `crates/core/ironclaw_gone/src/lib.rs`.\n",
        )
        self.write(
            "docs/internal/reborn/target-architecture/CHECKLIST.md",
            "Milestone: `crates/core/ironclaw_gone/src/lib.rs`.\n",
        )
        self.write(
            "docs/internal/reborn/contracts/example.md",
            "Pinned by `crates/core/ironclaw_gone/tests/contract.rs`.\n",
        )
        code, output = self.run_gate(
            internal_guidance=("docs/internal/reborn/contracts/",)
        )
        self.assertEqual(code, 1)
        self.assertNotIn("2026-01-01-old-plan.md", output)
        self.assertNotIn("CHECKLIST.md", output)
        self.assertIn("docs/internal/reborn/contracts/example.md", output)
        self.assertIn("crates/core/ironclaw_gone/tests/contract.rs", output)

    def test_docs_unterminated_fence_refuses(self) -> None:
        self.build_fixture()
        self.write(
            "docs/broken.mdx",
            "Prose.\n```bash\nnever closed\n",
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("unterminated ``` fence", output)

    def test_docs_unterminated_comment_refuses(self) -> None:
        """An unterminated comment must refuse like an unterminated fence:
        silently un-scanning the rest of the file (here, a dangling
        reference after the typo'd closer) is the fail-open shape the gate's
        charter forbids."""
        self.build_fixture()
        for name, body in (
            (
                "docs/typo.mdx",
                "{/* doc-fact:example\nnever closed with */ only\n"
                "See `crates/core/ironclaw_gone/src/lib.rs`.\n",
            ),
            (
                "docs/typo2.md",
                "Prose <!-- opened and abandoned\n"
                "See `crates/core/ironclaw_gone/src/lib.rs`.\n",
            ),
        ):
            with self.subTest(name=name):
                self.write(name, body)
                code, output = self.run_gate()
                (self.root / name).unlink()
                self.assertEqual(code, 1)
                self.assertIn("unterminated", output)
                self.assertIn(name, output)

    def test_internal_spec_links_are_repo_claims(self) -> None:
        """Internal spec pages are never published, so their relative links
        are repo-path claims, not site routes."""
        self.build_fixture()
        self.write("docs/internal/reborn/contracts/kernel.md", "A real sibling.\n")
        self.write(
            "docs/internal/reborn/contracts/index.md",
            "See [kernel](kernel.md) and [renamed](renamed-away.md).\n",
        )
        code, output = self.run_gate(
            internal_guidance=("docs/internal/reborn/contracts/",)
        )
        self.assertEqual(code, 1)
        self.assertNotIn("kernel.md`", output)
        self.assertIn("renamed-away.md", output)

    def test_internal_guidance_prefix_matching_no_pages_refuses(self) -> None:
        """A stale spec prefix (the corpus moved) must refuse, not silently
        drop the pages from the scan."""
        self.build_fixture()
        code, output = self.run_gate(
            internal_guidance=("docs/internal/reborn/contracts/",)
        )
        self.assertEqual(code, 1)
        self.assertIn("matched no tracked", output)
        self.assertIn("docs/internal/reborn/contracts/", output)

    def test_nav_coverage_refuses_when_docs_discovery_breaks(self) -> None:
        """A broken docs discovery filter refuses on the first nav page the
        scan no longer covers."""
        self.build_fixture()
        with mock.patch.object(GATE, "DOCS_PREFIX", "docz/"):
            code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("navigation publishes pages the reference scan", output)
        self.assertIn("guide", output)

    def test_nav_page_without_scanned_source_refuses(self) -> None:
        """A nav entry with no scanned source file is published prose the
        gate is not checking."""
        self.build_fixture()
        self.write(
            "docs/docs.json",
            '{"navigation": {"pages": ["guide", "ghost-page"]}}\n',
        )
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("ghost-page", output)

    def test_openapi_nav_pseudo_pages_are_not_coverage_claims(self) -> None:
        """Generated endpoint entries ("GET /users") have no source file
        and are skipped."""
        self.build_fixture()
        self.write(
            "docs/docs.json",
            '{"navigation": {"pages": ["guide", "GET /users"]}}\n',
        )
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)

    def test_unreadable_docs_json_refuses(self) -> None:
        """Missing, unparseable, or page-less docs.json refuses — never a
        vacuous pass."""
        self.build_fixture()
        self.write("docs/docs.json", "{not json\n")
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("cannot read docs/docs.json", output)
        (self.root / "docs/docs.json").unlink()
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("cannot read docs/docs.json", output)
        self.write("docs/docs.json", '{"navigation": {"pages": []}}\n')
        code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("names no source-backed pages", output)

    def test_missing_tracked_files_override_refuses(self) -> None:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = GATE.main(
                [
                    "--repo-root",
                    str(self.root),
                    "--tracked-files",
                    str(self.root / "nonexistent.txt"),
                ]
            )
        self.assertEqual(code, 1)
        self.assertIn("cannot read --tracked-files", stderr.getvalue())

    # -- happy path, against the real repository ---------------------------
    def test_real_repository_guidance_is_clean(self) -> None:
        """Deliberately coupled to `git` and this checkout: no
        `--tracked-files` override, so `GATE.main` shells out to
        `git ls-files -s` against the real repository. That coupling is the
        point — it proves the production entry path (subprocess, index
        parsing, symlink blobs) on real data, which no fixture run exercises.
        Consequence: outside a git checkout (a source export, a tarball) this
        case fails with "the tracked set cannot be read" — an environment
        failure, not guidance drift. The separate `Check guidance references
        the tracked tree` CI step runs the same command as the gate; this
        case exists so the self-test suite alone still covers the git leg."""
        stderr, stdout = io.StringIO(), io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
            crate_tree.reset_inventory_cache()
            code = GATE.main(["--repo-root", str(REPO_ROOT)])
        crate_tree.reset_inventory_cache()
        output = stderr.getvalue() + stdout.getvalue()
        self.assertEqual(code, 0, output)
        self.assertIn("guidance: OK", output)


if __name__ == "__main__":
    unittest.main()
