#!/usr/bin/env python3
"""Self-test for `scripts/ci/check-target-tree.py`.

A tree gate that cannot fail is worse than no tree gate: it converts "nobody
checked" into "something checked and it was fine". So almost every case here is
about the gate REFUSING — a crate in the wrong family, a crate §5 never drew, a
crate §5 drew that nobody built, a package name that stopped matching its
directory, an excluded package that quietly became a member, an excluded
package that vanished from disk, an exceptions row that outlived its delta, and
a §5 section this gate can no longer find. The happy path is checked against
the real repository, last.

The gate reads the real tree through `cargo metadata`; these tests feed it a
recorded metadata document instead (`--metadata`), so a doctored tree costs a
JSON edit rather than a fixture workspace, and the suite needs no toolchain.
"""

from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SCRIPT = pathlib.Path(__file__).with_name("check-target-tree.py")
REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]

_SPEC = importlib.util.spec_from_file_location("target_tree_check", SCRIPT)
assert _SPEC and _SPEC.loader
GATE = importlib.util.module_from_spec(_SPEC)
# Registered before execution: `@dataclasses.dataclass` resolves a class's
# module through `sys.modules`, and on 3.12+ a hyphenated script loaded by path
# alone raises there instead of defining the dataclass.
sys.modules[_SPEC.name] = GATE
_SPEC.loader.exec_module(GATE)


def _real_metadata() -> dict:
    completed = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(completed.stdout)


class TargetTreeGateTests(unittest.TestCase):
    metadata: dict

    @classmethod
    def setUpClass(cls) -> None:
        cls.metadata = _real_metadata()
        cls._tmp = tempfile.TemporaryDirectory()
        cls.tmp = pathlib.Path(cls._tmp.name)

    @classmethod
    def tearDownClass(cls) -> None:
        cls._tmp.cleanup()

    # -- helpers ----------------------------------------------------------
    def run_gate(
        self,
        metadata: dict | None = None,
        proposal: pathlib.Path | None = None,
        repo_root: pathlib.Path | None = None,
    ) -> tuple[int, str]:
        document = copy.deepcopy(self.metadata) if metadata is None else metadata
        path = self.tmp / f"metadata-{self.id().rsplit('.', 1)[-1]}.json"
        path.write_text(json.dumps(document), encoding="utf-8")
        argv = ["--metadata", str(path), "--repo-root", str(repo_root or REPO_ROOT)]
        if proposal is not None:
            argv += ["--proposal", str(proposal)]
        stderr = io.StringIO()
        stdout = io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
            code = GATE.main(argv)
        return code, stderr.getvalue() + stdout.getvalue()

    def relocate(self, metadata: dict, package: str, old: str, new: str) -> dict:
        for entry in metadata["packages"]:
            if entry["name"] == package:
                entry["manifest_path"] = entry["manifest_path"].replace(old, new, 1)
                return metadata
        raise AssertionError(f"{package} is not a workspace member of the fixture")

    def written_proposal(self, body: str) -> pathlib.Path:
        path = self.tmp / f"PROPOSAL-{self.id().rsplit('.', 1)[-1]}.md"
        path.write_text(body, encoding="utf-8")
        return path

    # -- refusals ---------------------------------------------------------
    def test_crate_in_the_wrong_family_fails(self) -> None:
        """The whole point: discovery-based gates cannot see this."""
        metadata = self.relocate(
            copy.deepcopy(self.metadata),
            "ironclaw_wasm",
            "/crates/lanes/",
            "/crates/substrates/",
        )
        code, output = self.run_gate(metadata)
        self.assertEqual(code, 1)
        self.assertIn("crates/substrates/ironclaw_wasm", output)
        self.assertIn("crates/lanes/ironclaw_wasm", output)

    def test_crate_section5_never_drew_fails(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        template = next(
            entry for entry in metadata["packages"] if entry["name"] == "ironclaw_mcp"
        )
        invented = copy.deepcopy(template)
        invented["name"] = "ironclaw_invented"
        invented["manifest_path"] = str(
            pathlib.Path(metadata["workspace_root"])
            / "crates/lanes/ironclaw_invented/Cargo.toml"
        )
        metadata["packages"].append(invented)
        code, output = self.run_gate(metadata)
        self.assertEqual(code, 1)
        self.assertIn("§5 draws no home for it", output)

    def test_crate_section5_draws_but_nobody_built_fails(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        metadata["packages"] = [
            entry for entry in metadata["packages"] if entry["name"] != "ironclaw_trust"
        ]
        code, output = self.run_gate(metadata)
        self.assertEqual(code, 1)
        self.assertIn("ironclaw_trust", output)
        self.assertIn("no workspace member has that name", output)

    def test_package_name_must_match_its_directory(self) -> None:
        """§5.1's directory rule. A renamed package in an unrenamed directory
        surfaces as both halves of the same break."""
        metadata = copy.deepcopy(self.metadata)
        for entry in metadata["packages"]:
            if entry["name"] == "ironclaw_llm":
                entry["name"] = "ironclaw_language_models"
        code, output = self.run_gate(metadata)
        self.assertEqual(code, 1)
        self.assertIn("ironclaw_language_models", output)
        self.assertIn("ironclaw_llm", output)

    def test_documented_exclusion_that_became_a_member_fails(self) -> None:
        metadata = copy.deepcopy(self.metadata)
        template = next(
            entry for entry in metadata["packages"] if entry["name"] == "ironclaw_stress"
        )
        silk = copy.deepcopy(template)
        silk["name"] = "ironclaw_silk_decoder"
        silk["manifest_path"] = str(
            pathlib.Path(metadata["workspace_root"])
            / "tools/ironclaw_silk_decoder/Cargo.toml"
        )
        metadata["packages"].append(silk)
        code, output = self.run_gate(metadata)
        self.assertEqual(code, 1)
        self.assertIn("excluded from this workspace", output)

    def test_documented_exclusion_missing_from_disk_fails(self) -> None:
        """An excluded package nothing builds is the easiest crate to lose."""
        empty_root = self.tmp / "empty-root"
        empty_root.mkdir(exist_ok=True)
        code, output = self.run_gate(
            repo_root=empty_root, proposal=REPO_ROOT / GATE.PROPOSAL_RELATIVE
        )
        self.assertEqual(code, 1)
        self.assertIn("tools/ironclaw_silk_decoder/Cargo.toml", output)

    # -- the exceptions table is shrink-only -------------------------------
    def test_exception_row_for_a_package_that_is_gone_fails(self) -> None:
        row = GATE.Exception_(
            package="ironclaw_departed",
            actual="crates/ironclaw_departed",
            documented=None,
            owner="a row that already landed",
            why="stale",
        )
        with mock.patch.object(GATE, "EXCEPTIONS", (row,)):
            code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("a delta that no longer exists", output)

    def test_exception_row_for_a_crate_now_at_its_section5_path_fails(self) -> None:
        row = GATE.Exception_(
            package="ironclaw_wasm",
            actual="crates/ironclaw_wasm",
            documented="crates/lanes/ironclaw_wasm",
            owner="WS7 2/2",
            why="already landed",
        )
        with mock.patch.object(GATE, "EXCEPTIONS", GATE.EXCEPTIONS + (row,)):
            code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("Delete the row", output)

    def test_exception_row_describing_a_different_delta_fails(self) -> None:
        rows = tuple(
            GATE.Exception_(
                package=row.package,
                actual="crates/somewhere_else",
                documented=row.documented,
                owner=row.owner,
                why=row.why,
            )
            if row.package == "ironclaw_projects"
            else row
            for row in GATE.EXCEPTIONS
        )
        with mock.patch.object(GATE, "EXCEPTIONS", rows):
            code, output = self.run_gate()
        self.assertEqual(code, 1)
        self.assertIn("describes a different delta", output)

    # -- the gate must not fail open --------------------------------------
    def test_missing_section_refuses(self) -> None:
        proposal = self.written_proposal("# PROPOSAL\n\nNo tree here.\n")
        code, output = self.run_gate(proposal=proposal)
        self.assertEqual(code, 1)
        self.assertIn("has no", output)

    def test_unfenced_section_refuses(self) -> None:
        proposal = self.written_proposal(
            f"{GATE.SECTION_HEADING}\n\nprose, no fenced block\n"
        )
        code, output = self.run_gate(proposal=proposal)
        self.assertEqual(code, 1)
        self.assertIn("no ```text block", output)

    def test_truncated_tree_refuses(self) -> None:
        proposal = self.written_proposal(
            f"{GATE.SECTION_HEADING}\n\n```text\n"
            "crates/\n"
            "├── lanes/                 ▢ execution mechanisms\n"
            "│   └── ironclaw_wasm      ▣ [runtimes] WASM component lane\n"
            "```\n"
        )
        code, output = self.run_gate(proposal=proposal)
        self.assertEqual(code, 1)
        self.assertIn("floor is", output)

    def test_missing_metadata_file_refuses(self) -> None:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            with self.assertRaises(OSError):
                GATE.main(["--metadata", str(self.tmp / "nonexistent.json")])

    # -- the parser's two written naming exceptions ------------------------
    def test_cli_directory_holds_the_package_named_in_its_annotation(self) -> None:
        entries = GATE.parse_tree(GATE.read_section(REPO_ROOT / GATE.PROPOSAL_RELATIVE))
        by_directory = {entry.directory: entry.package for entry in entries}
        self.assertEqual(by_directory["crates/app/ironclaw_cli"], "ironclaw")

    def test_package_directories_take_the_crate_name_beside_the_marker(self) -> None:
        entries = GATE.parse_tree(GATE.read_section(REPO_ROOT / GATE.PROPOSAL_RELATIVE))
        by_directory = {entry.directory: entry.package for entry in entries}
        self.assertEqual(
            by_directory["crates/extensions/packages/slack"],
            "ironclaw_slack_extension",
        )
        self.assertEqual(by_directory["."], "ironclaw_integration_tests")

    def test_excluded_entries_are_parsed_as_exclusions(self) -> None:
        entries = GATE.parse_tree(GATE.read_section(REPO_ROOT / GATE.PROPOSAL_RELATIVE))
        excluded = {entry.package: entry.directory for entry in entries if entry.excluded}
        self.assertEqual(excluded, {"ironclaw_silk_decoder": "tools/ironclaw_silk_decoder"})

    # -- happy path, against the real tree ---------------------------------
    def test_real_tree_matches_section5(self) -> None:
        code, output = self.run_gate()
        self.assertEqual(code, 0, output)
        self.assertIn("target tree: OK", output)


if __name__ == "__main__":
    unittest.main()
