#!/usr/bin/env python3
"""Validate the retained Python E2E inventory for ``ironclaw serve``."""

from __future__ import annotations

import ast
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests/e2e/ironclaw_serve_e2e_tests.txt"
REQUIRED_CATEGORIES = {
    "auth-oauth",
    "conversation-thread",
    "engine-tool-extension",
}
FORBIDDEN_FIXTURES = {
    "auth_matrix_server",
    "hosted_oauth_refresh_server",
    "ironclaw_binary",
    "ironclaw_server",
}
SERVE_FIXTURE_PREFIX = "reborn_v2_"


@dataclass(frozen=True)
class Selection:
    category: str
    selector: str


def selections() -> list[Selection]:
    selected: list[Selection] = []
    category: str | None = None
    for raw_line in MANIFEST.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line.startswith("# category:"):
            category = line.partition(":")[2].strip()
            continue
        selector = line.split("#", 1)[0].strip()
        if not selector:
            continue
        if category is None:
            raise SystemExit(f"selector has no category: {selector}")
        selected.append(Selection(category=category, selector=selector))
    return selected


def test_functions(path: Path) -> dict[str, ast.FunctionDef | ast.AsyncFunctionDef]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    return {
        node.name: node
        for node in tree.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name.startswith("test_")
    }


def selected_tests(
    selection: Selection,
) -> tuple[Path, dict[str, ast.FunctionDef | ast.AsyncFunctionDef]]:
    path_text, separator, test_name = selection.selector.partition("::")
    path = ROOT / path_text
    if not path.is_file():
        raise SystemExit(f"selected scenario does not exist: {path_text}")
    tests = test_functions(path)
    if separator:
        try:
            return path, {test_name: tests[test_name]}
        except KeyError as exc:
            raise SystemExit(
                f"selected test does not exist: {selection.selector}"
            ) from exc
    if not tests:
        raise SystemExit(f"selected scenario has no tests: {path_text}")
    return path, tests


def fixture_names(node: ast.FunctionDef | ast.AsyncFunctionDef) -> set[str]:
    return {
        argument.arg
        for argument in (
            *node.args.posonlyargs,
            *node.args.args,
            *node.args.kwonlyargs,
        )
    }


def main() -> int:
    manifest = selections()
    errors: list[str] = []
    categories = {selection.category for selection in manifest}
    missing_categories = sorted(REQUIRED_CATEGORIES - categories)
    extra_categories = sorted(categories - REQUIRED_CATEGORIES)
    if missing_categories:
        errors.append(
            "missing retained coverage categories: "
            + ", ".join(missing_categories)
        )
    if extra_categories:
        errors.append(
            "unknown retained coverage categories: "
            + ", ".join(extra_categories)
        )

    duplicates = sorted(
        selector
        for selector, count in Counter(
            selection.selector for selection in manifest
        ).items()
        if count > 1
    )
    if duplicates:
        errors.append(
            "duplicate retained selectors:\n  " + "\n  ".join(duplicates)
        )

    selected_count = 0
    for selection in manifest:
        path, tests = selected_tests(selection)
        for test_name, node in tests.items():
            selected_count += 1
            fixtures = fixture_names(node)
            forbidden = sorted(fixtures & FORBIDDEN_FIXTURES)
            if forbidden:
                errors.append(
                    f"{path.relative_to(ROOT)}::{test_name} uses deleted-binary "
                    f"fixture(s): {', '.join(forbidden)}"
                )
            if not any(
                fixture.startswith(SERVE_FIXTURE_PREFIX)
                for fixture in fixtures
            ):
                errors.append(
                    f"{path.relative_to(ROOT)}::{test_name} is not routed "
                    "through a reborn_v2_* serve fixture"
                )

    if errors:
        raise SystemExit("\n".join(errors))

    print(
        "IronClaw serve E2E manifest is complete "
        f"({len(manifest)} selectors, {selected_count} tests)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
