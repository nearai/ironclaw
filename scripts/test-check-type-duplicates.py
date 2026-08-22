#!/usr/bin/env python3
"""Self-test for `scripts/check-type-duplicates.py`.

The collector walks two glob shapes: `crates/<family>/<crate>/src` (the
post-reorg nested layout) and `crates/extensions/packages/<pkg>/src` (the
extension-package layout, which does not nest under a family directory the
same way). Neither had a regression test — a future glob/layout change could
silently narrow the scan back to a shallow `crates/*/src` layout and nothing
would fail, despite the tool's docstring claiming the reorganized workspace
is covered. These tests build a small on-disk crate tree covering both
shapes and assert `collect()` finds types from each, and that the semantic
(field-shape) duplicate match still fires across differently-named types.

`collect()` walks relative `Path('crates')`/`Path('crates/extensions/...')`,
so it is cwd-dependent; tests `chdir` into a temporary tree rather than
patching the module (the same real-tree-vs-fixture split
`scripts/ci/test-check-guidance.py` uses).
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

SCRIPT = pathlib.Path(__file__).with_name("check-type-duplicates.py")

_SPEC = importlib.util.spec_from_file_location("check_type_duplicates", SCRIPT)
assert _SPEC and _SPEC.loader
DUP = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = DUP
_SPEC.loader.exec_module(DUP)


class CollectTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self._cwd = os.getcwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, self._cwd)

    def write(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def test_nested_family_crate_is_discovered(self) -> None:
        """`crates/<family>/<crate>/src` — the post-reorg layout — must be
        walked, not just a shallow `crates/*/src`."""
        self.write(
            "crates/domains/ironclaw_alpha/src/types.rs",
            "pub struct Widget {\n"
            "    id: String,\n"
            "    name: String,\n"
            "    size: u32,\n"
            "}\n",
        )
        types = DUP.collect(min_items=3)
        self.assertEqual(len(types), 1)
        crate, kind, name, items, path = types[0]
        self.assertEqual(crate, "alpha")
        self.assertEqual(kind, "struct")
        self.assertEqual(name, "Widget")
        self.assertEqual(
            items,
            frozenset({("id", "String"), ("name", "String"), ("size", "u32")}),
        )

    def test_extension_package_crate_is_discovered(self) -> None:
        """`crates/extensions/packages/<pkg>/src` does not nest under a
        family directory the same way as `crates/<family>/<crate>` — a
        glob written only for the family shape would miss it."""
        self.write(
            "crates/extensions/packages/acme/src/config.rs",
            "pub struct Gadget {\n"
            "    id: String,\n"
            "    name: String,\n"
            "    size: u32,\n"
            "}\n",
        )
        types = DUP.collect(min_items=3)
        self.assertEqual(len(types), 1)
        crate, kind, name, items, path = types[0]
        self.assertEqual(crate, "acme")
        self.assertEqual(kind, "struct")
        self.assertEqual(name, "Gadget")

    def test_semantic_duplicate_across_both_shapes_is_flagged(self) -> None:
        """The field-shape match (not name matching) is the tool's whole
        point: a family crate and an extension-package crate defining
        differently-named structs with the same (field, type) set must be
        reported as a candidate pair.

        This drives the production `main()` candidate-selection and report
        path (the itertools.combinations/Jaccard loop and its printed
        output), not a local reimplementation of Jaccard over `collect()`'s
        return value — a regression in `main()`'s own pairing or reporting
        logic would not be caught by re-deriving the score independently.
        """
        self.write(
            "crates/domains/ironclaw_alpha/src/types.rs",
            "pub struct Widget {\n"
            "    id: String,\n"
            "    name: String,\n"
            "    size: u32,\n"
            "}\n",
        )
        self.write(
            "crates/extensions/packages/acme/src/config.rs",
            "pub struct Gadget {\n"
            "    id: String,\n"
            "    name: String,\n"
            "    size: u32,\n"
            "}\n",
        )
        types = DUP.collect(min_items=3)
        self.assertEqual(len(types), 2)
        a, b = types
        i1, i2 = a[3], b[3]
        union = len(i1 | i2)
        jaccard = len(i1 & i2) / union if union else 0
        self.assertEqual(jaccard, 1.0)

        argv = sys.argv
        sys.argv = ["check-type-duplicates.py", "--min-items", "3"]
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                DUP.main()
        finally:
            sys.argv = argv
        report = out.getvalue()
        self.assertIn("struct alpha::Widget  <->  acme::Gadget", report)
        self.assertIn("1 candidate pair(s) from 2 types", report)

    def test_below_min_items_is_not_collected(self) -> None:
        self.write(
            "crates/domains/ironclaw_alpha/src/types.rs",
            "pub struct Tiny {\n    id: String,\n}\n",
        )
        self.assertEqual(DUP.collect(min_items=3), [])


if __name__ == "__main__":
    unittest.main()
