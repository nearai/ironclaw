#!/usr/bin/env python3
"""Self-tests for scripts/ci/docs_publication_boundary.py."""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import docs_publication_boundary as boundary


def make_docs_tree(
    root: Path,
    nav_pages: list[object],
    mintignore: str | None,
    files: dict[str, str],
) -> Path:
    docs = root / "docs"
    docs.mkdir()
    docs_json = {
        "navigation": {
            "languages": [
                {"language": "en", "tabs": [{"tab": " ", "groups": [
                    {"group": " ", "pages": nav_pages}
                ]}]}
            ]
        }
    }
    (docs / "docs.json").write_text(json.dumps(docs_json), encoding="utf-8")
    if mintignore is not None:
        (docs / ".mintignore").write_text(mintignore, encoding="utf-8")
    for rel, content in files.items():
        path = docs / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return docs


class DocsPublicationBoundaryTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def test_page_in_nav_is_clean(self) -> None:
        docs = make_docs_tree(
            self.root, ["index"], None, {"index.mdx": "# Home"}
        )
        unfenced, missing, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])
        self.assertEqual(missing, [])

    def test_new_mintignore_entry_is_rejected(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "internal/\nsecret-notes/\n",
            {"index.mdx": "# Home", "secret-notes/plan.md": "# Internal"},
        )
        unfenced, _, unexpected = boundary.find_violations(docs)
        self.assertEqual(unexpected, ["secret-notes/"])
        # The rogue entry still fences its files — it fails the frozen-list
        # rule, not the leak rule, so the fix message points at internal/.
        self.assertEqual(unfenced, [])

    def test_mintignore_subset_of_frozen_list_is_allowed(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "internal/\nreborn/\n*.draft.mdx\n",
            {"index.mdx": "# Home"},
        )
        _, _, unexpected = boundary.find_violations(docs)
        self.assertEqual(unexpected, [])

    def test_unfenced_page_is_flagged(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "internal/\n",
            {"index.mdx": "# Home", "design/notes.md": "# Internal"},
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, ["design/notes.md"])

    def test_mintignore_directory_pattern_fences(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "internal/\n",
            {"index.mdx": "# Home", "internal/deep/notes.md": "# Internal"},
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])

    def test_mintignore_glob_pattern_fences(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "*.draft.mdx\n",
            {"index.mdx": "# Home", "guide.draft.mdx": "# WIP"},
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])

    def test_mintignore_literal_file_fences(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "reborn-binary.md\n",
            {"index.mdx": "# Home", "reborn-binary.md": "# Internal"},
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])

    def test_hidden_frontmatter_marks_deliberate_page(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            None,
            {
                "index.mdx": "# Home",
                "unlisted.mdx": "---\ntitle: Unlisted\nhidden: true\n---\n# P",
            },
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])

    def test_hidden_after_frontmatter_close_does_not_count(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            None,
            {
                "index.mdx": "# Home",
                "leak.mdx": "---\ntitle: Leak\n---\nhidden: true\n",
            },
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, ["leak.mdx"])

    def test_nav_entry_without_source_file_is_flagged(self) -> None:
        docs = make_docs_tree(
            self.root, ["index", "ghost/page"], None, {"index.mdx": "# Home"}
        )
        _, missing, _ = boundary.find_violations(docs)
        self.assertEqual(missing, ["ghost/page"])

    def test_nav_entry_matches_md_extension_too(self) -> None:
        docs = make_docs_tree(
            self.root, ["guide"], None, {"guide.md": "# Guide"}
        )
        unfenced, missing, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])
        self.assertEqual(missing, [])

    def test_nested_nav_groups_are_collected(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index", {"group": "Deep", "pages": [{"group": "Deeper", "pages": ["a/b"]}]}],
            None,
            {"index.mdx": "# Home", "a/b.mdx": "# B"},
        )
        unfenced, missing, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])
        self.assertEqual(missing, [])

    def test_builtin_ignores_apply_without_mintignore(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            None,
            {
                "index.mdx": "# Home",
                "README.md": "# Readme",
                "snippets/shared.mdx": "shared",
                ".claude/skills/x/SKILL.md": "skill",
            },
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])

    def test_negation_pattern_is_rejected_loudly(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "internal/\n!internal/public.md\n",
            {"index.mdx": "# Home"},
        )
        with self.assertRaises(boundary.MintignoreSyntaxError):
            boundary.find_violations(docs)

    def test_openapi_nav_entries_need_no_source_file(self) -> None:
        docs = make_docs_tree(
            self.root, ["index", "GET /users"], None, {"index.mdx": "# Home"}
        )
        _, missing, _ = boundary.find_violations(docs)
        self.assertEqual(missing, [])

    def test_mintignore_nested_path_glob_pattern_fences(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index"],
            "design/*.md\n",
            {"index.mdx": "# Home", "design/notes.md": "# Internal"},
        )
        unfenced, _, _ = boundary.find_violations(docs)
        self.assertEqual(unfenced, [])

    def test_main_returns_0_on_clean_tree(self) -> None:
        docs = make_docs_tree(
            self.root, ["index"], "internal/\n", {"index.mdx": "# Home"}
        )
        with mock.patch.object(boundary, "DOCS_ROOT", docs):
            with contextlib.redirect_stdout(io.StringIO()) as out:
                rc = boundary.main()
        self.assertEqual(rc, 0)
        self.assertIn("published or fenced", out.getvalue())

    def test_main_returns_1_and_prints_every_violation_class(self) -> None:
        docs = make_docs_tree(
            self.root,
            ["index", "ghost/page"],
            "internal/\nrogue/\n",
            {"index.mdx": "# Home", "design/notes.md": "# Internal"},
        )
        with mock.patch.object(boundary, "DOCS_ROOT", docs):
            with contextlib.redirect_stderr(io.StringIO()) as err:
                rc = boundary.main()
        self.assertEqual(rc, 1)
        stderr = err.getvalue()
        self.assertIn("docs/design/notes.md", stderr)
        self.assertIn("ghost/page", stderr)
        self.assertIn("rogue/", stderr)
        self.assertIn("docs/internal/", stderr)

    def test_main_reports_mintignore_syntax_error(self) -> None:
        docs = make_docs_tree(
            self.root, ["index"], "internal/\n!internal/x.md\n", {"index.mdx": "# Home"}
        )
        with mock.patch.object(boundary, "DOCS_ROOT", docs):
            with contextlib.redirect_stderr(io.StringIO()) as err:
                rc = boundary.main()
        self.assertEqual(rc, 1)
        self.assertIn("negation pattern", err.getvalue())

    def test_real_repo_docs_pass(self) -> None:
        unfenced, missing, unexpected = boundary.find_violations(boundary.DOCS_ROOT)
        self.assertEqual(unfenced, [])
        self.assertEqual(missing, [])
        self.assertEqual(unexpected, [])


if __name__ == "__main__":
    unittest.main()
