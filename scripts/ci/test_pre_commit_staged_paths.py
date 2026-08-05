#!/usr/bin/env python3
"""Pin `.githooks/pre-commit`'s staged-path selector.

The hook runs the version-bump check only when a staged path matches a literal
path pattern. Wave 3 moved the WIT directory into its owning crate, so that
pattern changed from `^wit/` to `^crates/ironclaw_wasm/wit/`. A path-literal
gate has a silent failure mode: move the directory it names and the hook keeps
exiting 0, so version bumps stop being checked and nothing says so.

These tests pin both halves — that the selector still matches the directories it
is meant to gate, and that those directories still exist. The second half is the
one that catches a future move.
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import unittest

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
HOOK = REPO_ROOT / ".githooks" / "pre-commit"

# The alternation branches are directory prefixes, so each must name a real
# directory. Anchors and the trailing slash are stripped to get the path.
GATED_PREFIXES = ("crates/ironclaw_wasm/wit/", "channels-src/", "tools-src/")


def selector_pattern() -> str:
    """The extended-regex literal the hook feeds to `grep -qE`."""
    match = re.search(r"grep -qE '([^']+)'", HOOK.read_text(encoding="utf-8"))
    if match is None:
        raise AssertionError(
            f"{HOOK}: no `grep -qE '<pattern>'` staged-path selector found. "
            "If the hook was restructured, repoint this test at the new "
            "selector rather than deleting it."
        )
    return match.group(1)


def matches(pattern: str, path: str) -> bool:
    """Match through `grep -E`, so the test sees the hook's own regex dialect."""
    return (
        subprocess.run(
            ["grep", "-qE", pattern],
            input=f"{path}\n",
            text=True,
            check=False,
        ).returncode
        == 0
    )


class PreCommitStagedPathSelectorTests(unittest.TestCase):
    def test_selector_matches_every_gated_directory(self) -> None:
        pattern = selector_pattern()
        for prefix in GATED_PREFIXES:
            with self.subTest(prefix=prefix):
                self.assertTrue(
                    matches(pattern, f"{prefix}example.txt"),
                    f"hook no longer gates {prefix}",
                )

    def test_relocated_wit_directory_exists(self) -> None:
        """The rename guard: a path literal naming a directory that no longer
        exists is a gate that can never fire again.

        Scoped to the WIT prefix, which is the one Wave 3 moved. `channels-src/`
        and `tools-src/` are gated by the hook but exist neither here nor on
        `origin/main` — pre-existing vestigial prefixes, not this branch's doing,
        so they are reported (see `test_vestigial_prefixes_are_recorded`) rather
        than failed, which would make the branch red for someone else's debt.
        """
        self.assertTrue(
            (REPO_ROOT / "crates/ironclaw_wasm/wit/").is_dir(),
            "crates/ironclaw_wasm/wit/ is gated by .githooks/pre-commit but "
            "does not exist; move the gate with the directory",
        )

    def test_vestigial_prefixes_are_recorded(self) -> None:
        """Pins which gated prefixes are known-missing, so a *new* dead prefix
        fails here instead of joining the list silently."""
        missing = {p for p in GATED_PREFIXES if not (REPO_ROOT / p).is_dir()}
        self.assertEqual(
            missing,
            {"channels-src/", "tools-src/"},
            "the set of gated-but-missing prefixes changed; a gate either "
            "gained a dead path literal or an old one was cleaned up",
        )

    def test_selector_is_anchored_and_ignores_unrelated_paths(self) -> None:
        pattern = selector_pattern()
        for path in (
            "README.md",
            "crates/ironclaw_capabilities/src/host.rs",
            # Anchoring: the gated names must not match mid-path, or unrelated
            # vendored trees would trigger the version-bump check.
            "vendor/tools-src/thing.rs",
            "docs/channels-src/notes.md",
            # The pre-move location must no longer match on its own.
            "wit/tool.wit",
        ):
            with self.subTest(path=path):
                self.assertFalse(
                    matches(pattern, path),
                    f"hook unexpectedly gates {path}",
                )


if __name__ == "__main__":
    unittest.main()
