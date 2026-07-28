#!/usr/bin/env python3
"""Verify the required served Reborn Responses API E2E inventory is complete."""

from __future__ import annotations

import ast
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/e2e/reborn_responses_e2e_tests.txt"
PRIMARY = ROOT / "tests/e2e/scenarios/test_reborn_responses_api.py"
LEGACY_PORT = ROOT / "tests/e2e/scenarios/test_reborn_webui_v2_legacy_responses_api.py"
ROUTE_TESTS = {
    "test_reborn_openai_compat_route_mounts_authenticated_aliases_served",
    "test_reborn_openai_compat_route_mounts_require_bearer_served",
}


def test_names(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    return {
        node.name
        for node in tree.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name.startswith("test_")
    }


def node_id(path: Path, name: str) -> str:
    return f"{path.relative_to(ROOT)}::{name}"


def expected_node_ids() -> set[str]:
    primary = {
        name
        for name in test_names(PRIMARY)
        if name.startswith("test_reborn_responses_") or name in ROUTE_TESTS
    }
    legacy = test_names(LEGACY_PORT)
    return {
        *(node_id(PRIMARY, name) for name in primary),
        *(node_id(LEGACY_PORT, name) for name in legacy),
    }


def manifest_node_ids() -> list[str]:
    return [
        line
        for raw_line in MANIFEST.read_text(encoding="utf-8").splitlines()
        if (line := raw_line.split("#", 1)[0].strip())
    ]


def main() -> int:
    manifest = manifest_node_ids()
    manifest_set = set(manifest)
    expected = expected_node_ids()

    errors: list[str] = []
    duplicates = sorted(
        selector for selector, count in Counter(manifest).items() if count > 1
    )
    if duplicates:
        errors.append(
            "manifest contains duplicate node ids:\n  " + "\n  ".join(duplicates)
        )

    missing = sorted(expected - manifest_set)
    extra = sorted(manifest_set - expected)
    if missing:
        errors.append("missing required Responses tests:\n  " + "\n  ".join(missing))
    if extra:
        errors.append("unexpected Responses test selectors:\n  " + "\n  ".join(extra))

    if errors:
        raise SystemExit("\n".join(errors))

    print(f"Reborn Responses E2E manifest is complete ({len(expected)} tests).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
