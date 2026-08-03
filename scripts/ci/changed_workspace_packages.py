#!/usr/bin/env python3
"""List workspace packages with changed production Rust inputs."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]


def changed_production_packages(
    changed_paths: list[str], metadata: dict[str, Any]
) -> list[str]:
    members = set(metadata["workspace_members"])
    packages = sorted(
        (
            Path(package["manifest_path"]).resolve().parent.relative_to(ROOT),
            package["name"],
        )
        for package in metadata["packages"]
        if package["id"] in members
    )
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--changed-files", type=Path, required=True)
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
    print(json.dumps(changed_production_packages(changed_paths, metadata)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
