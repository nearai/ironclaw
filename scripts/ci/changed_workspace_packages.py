#!/usr/bin/env python3
"""List workspace packages with changed production Rust inputs."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]

MERGE_GROUP_FULL_PATHS = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    ".cargo/config",
    ".cargo/config.toml",
    "clippy.toml",
}
MERGE_GROUP_FULL_PREFIXES = (
    ".github/actions/",
    ".github/workflows/",
    "migrations/",
    "scripts/ci/",
)


def _workspace_packages(metadata: dict[str, Any]) -> list[tuple[Path, str]]:
    members = set(metadata["workspace_members"])
    return sorted(
        (
            Path(package["manifest_path"]).resolve().parent.relative_to(ROOT),
            package["name"],
        )
        for package in metadata["packages"]
        if package["id"] in members
    )


def _reverse_dependency_closure(
    metadata: dict[str, Any], roots: set[str]
) -> set[str]:
    workspace_members = set(metadata["workspace_members"])
    workspace_packages = {
        package["name"]: package
        for package in metadata["packages"]
        if package["id"] in workspace_members
    }
    workspace_package_by_directory = {
        Path(package["manifest_path"]).resolve().parent: package["name"]
        for package in workspace_packages.values()
    }
    dependents: dict[str, set[str]] = {
        package_name: set() for package_name in workspace_packages
    }
    for package_name, package in workspace_packages.items():
        for dependency in package.get("dependencies", []):
            dependency_name = None
            dependency_path = dependency.get("path")
            if dependency_path:
                dependency_name = workspace_package_by_directory.get(
                    Path(dependency_path).resolve()
                )
            if dependency_name is None:
                dependency_name = dependency.get("name")
            if dependency_name in dependents:
                dependents[dependency_name].add(package_name)

    closure = set(roots)
    pending = list(roots)
    while pending:
        package_name = pending.pop()
        for dependent in dependents.get(package_name, set()):
            if dependent not in closure:
                closure.add(dependent)
                pending.append(dependent)
    return closure


def changed_production_packages(
    changed_paths: list[str], metadata: dict[str, Any]
) -> list[str]:
    packages = _workspace_packages(metadata)
    selected: set[str] = set()
    normalized_paths = {
        path.strip().replace("\\", "/") for path in changed_paths if path.strip()
    }
    if normalized_paths & {"Cargo.toml", "Cargo.lock"}:
        return sorted(package for _directory, package in packages)

    for path in normalized_paths:
        if not path:
            continue
        for directory, package in sorted(
            packages, key=lambda item: len(item[0].parts), reverse=True
        ):
            prefix = "" if str(directory) == "." else f"{directory.as_posix()}/"
            if prefix and not path.startswith(prefix):
                continue
            relative = path.removeprefix(prefix)
            if (
                relative == "Cargo.toml"
                or relative == "build.rs"
                or relative.startswith("src/")
            ):
                selected.add(package)
            break
    return sorted(selected)


def classify_clippy_scope(
    changed_paths: list[str], metadata: dict[str, Any], *, event: str
) -> dict[str, Any]:
    """Classify the clippy work required by a pull request or merge group."""
    if event not in {"pull_request", "merge_group"}:
        raise ValueError(f"unsupported diff event: {event}")

    normalized_paths = {
        path.strip().replace("\\", "/") for path in changed_paths if path.strip()
    }
    if event == "merge_group" and not normalized_paths:
        raise ValueError("empty merge-group diff cannot be classified")
    if event == "pull_request":
        packages = changed_production_packages(list(normalized_paths), metadata)
        return {"mode": "selected" if packages else "none", "packages": packages}

    packages = _workspace_packages(metadata)
    workspace_manifests = {
        "Cargo.toml"
        if str(directory) == "."
        else f"{directory.as_posix()}/Cargo.toml"
        for directory, _package in packages
    }
    if any(
        path in MERGE_GROUP_FULL_PATHS
        or path in workspace_manifests
        or path.startswith(MERGE_GROUP_FULL_PREFIXES)
        for path in normalized_paths
    ):
        return {"mode": "full", "packages": []}

    selected: set[str] = set()
    ignored_prefixes = ("docs/", "openwiki/", ".claude/", ".github/ISSUE_TEMPLATE/")
    ignored_paths = {
        ".env.example",
        ".github/pull_request_template.md",
        "AGENTS.md",
        "CLAUDE.md",
        "README.md",
    }
    production_packages: set[str] = set()
    for path in normalized_paths:
        matched_workspace_package = False
        for directory, package in sorted(
            packages, key=lambda item: len(item[0].parts), reverse=True
        ):
            prefix = "" if str(directory) == "." else f"{directory.as_posix()}/"
            if prefix and not path.startswith(prefix):
                continue
            relative = path.removeprefix(prefix)
            production_input = (
                relative == "Cargo.toml"
                or relative == "build.rs"
                or relative.startswith("src/")
            )
            package_test_surface = relative.startswith(
                ("tests/", "benches/", "examples/")
            )
            if production_input or package_test_surface:
                selected.add(package)
                matched_workspace_package = True
                if production_input:
                    production_packages.add(package)
            break
        if path.startswith("crates/") and not matched_workspace_package:
            return {"mode": "full", "packages": []}
        if (
            not matched_workspace_package
            and path not in ignored_paths
            and not path.startswith(ignored_prefixes)
        ):
            return {"mode": "full", "packages": []}

    selected.update(_reverse_dependency_closure(metadata, production_packages))
    package_names = sorted(selected)
    return {
        "mode": "selected" if package_names else "none",
        "packages": package_names,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--changed-files", type=Path, required=True)
    parser.add_argument(
        "--event", choices=("pull_request", "merge_group"), required=True
    )
    args = parser.parse_args()
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    changed_paths = args.changed_files.read_text(encoding="utf-8").splitlines()
    print(json.dumps(classify_clippy_scope(changed_paths, metadata, event=args.event)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
