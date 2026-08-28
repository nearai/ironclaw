#!/usr/bin/env python3
"""Canonical Cargo inventory for the integration-test CI topology."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import tomllib
from typing import Any

SCHEMA_VERSION = 1
INTEGRATION_PARTITION_COUNT = 4
INTEGRATION_PATH_PREFIX = "tests/integration/"
FLAT_NAME_PREFIXES = ("reborn_integration_", "reborn_generated_")
GROUP_NAME_PREFIX = "reborn_group_"

ROOT = pathlib.Path(__file__).resolve().parents[3]


def _registered_tests(repo_root: str | pathlib.Path = ROOT) -> list[tuple[str, str]]:
    root = pathlib.Path(repo_root)
    with (root / "Cargo.toml").open("rb") as manifest:
        data = tomllib.load(manifest)
    return [
        (entry["path"], entry["name"])
        for entry in data.get("test", [])
        if isinstance(entry, dict)
        and isinstance(entry.get("name"), str)
        and isinstance(entry.get("path"), str)
        and entry["path"].startswith(INTEGRATION_PATH_PREFIX)
    ]


def cargo_test_names(repo_root: str | pathlib.Path = ROOT) -> list[str]:
    """Return the shell selector's existing sorted-unique name projection."""

    return sorted({name for _, name in _registered_tests(repo_root)})


def _planner_registrations(
    repo_root: str | pathlib.Path = ROOT,
) -> dict[str, str]:
    """Return the planner's existing last-registration-per-path projection."""

    return {path: name for path, name in _registered_tests(repo_root)}


def planner_test_lanes(
    repo_root: str | pathlib.Path = ROOT,
) -> dict[str, str | int]:
    """Map registered integration paths to their current planner lanes."""

    tests = _planner_registrations(repo_root)
    for name in tests.values():
        if not (
            name.startswith(FLAT_NAME_PREFIXES)
            or name.startswith(GROUP_NAME_PREFIX)
        ):
            raise ValueError(f"unsupported integration test name: {name!r}")
    flat_names = sorted(
        name for name in tests.values() if name.startswith(FLAT_NAME_PREFIXES)
    )
    flat_lanes = {
        name: index % INTEGRATION_PARTITION_COUNT
        for index, name in enumerate(flat_names)
    }
    return {
        path: "groups" if name.startswith(GROUP_NAME_PREFIX) else flat_lanes[name]
        for path, name in tests.items()
    }


def inventory_document(repo_root: str | pathlib.Path = ROOT) -> dict[str, Any]:
    """Return the versioned normalized inventory consumed by later slices."""

    registrations = _planner_registrations(repo_root)
    lanes = planner_test_lanes(repo_root)
    document: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "partition_count": INTEGRATION_PARTITION_COUNT,
        "tests": [
            {
                "path": path,
                "name": name,
                "kind": "group" if name.startswith(GROUP_NAME_PREFIX) else "flat",
                "lane": lanes[path],
            }
            for path, name in sorted(
                registrations.items(), key=lambda item: (item[1], item[0])
            )
        ],
    }
    return validate_inventory_document(document)


def _required_integer(document: dict[str, Any], field: str, expected: int) -> int:
    value = document.get(field)
    if type(value) is not int or value != expected:
        raise ValueError(f"integration inventory {field} must be {expected}")
    return value


def validate_inventory_document(document: Any) -> dict[str, Any]:
    """Validate the generated cross-language inventory contract fail-closed."""

    if not isinstance(document, dict):
        raise ValueError("integration inventory must be an object")
    _required_integer(document, "schema_version", SCHEMA_VERSION)
    partition_count = _required_integer(
        document, "partition_count", INTEGRATION_PARTITION_COUNT
    )
    tests = document.get("tests")
    if not isinstance(tests, list):
        raise ValueError("integration inventory tests must be an array")
    for test in tests:
        if not isinstance(test, dict):
            raise ValueError("integration inventory test records must be objects")
        path = test.get("path")
        name = test.get("name")
        kind = test.get("kind")
        lane = test.get("lane")
        if not isinstance(path, str) or not path.startswith(INTEGRATION_PATH_PREFIX):
            raise ValueError("integration inventory test path is invalid")
        if not isinstance(name, str):
            raise ValueError("integration inventory test name is invalid")
        if kind == "group":
            if not name.startswith(GROUP_NAME_PREFIX) or lane != "groups":
                raise ValueError("integration inventory group record is invalid")
        elif kind == "flat":
            if (
                not name.startswith(FLAT_NAME_PREFIXES)
                or not isinstance(lane, int)
                or isinstance(lane, bool)
                or lane < 0
                or lane >= partition_count
            ):
                raise ValueError("integration inventory flat record is invalid")
        else:
            raise ValueError("integration inventory test kind is invalid")
    return document


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="print versioned JSON")
    parser.add_argument("repo_root", nargs="?", default=str(ROOT))
    args = parser.parse_args(argv)

    if args.json:
        print(json.dumps(inventory_document(args.repo_root), sort_keys=True))
        return 0

    names = cargo_test_names(args.repo_root)
    if not names:
        print("No Reborn integration-tier test binaries discovered", file=sys.stderr)
        return 1
    for name in names:
        print("--test")
        print(name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
