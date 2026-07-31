#!/usr/bin/env python3
"""Select focused Reborn test lanes for pull requests.

Pull requests run direct evidence for changed packages and test surfaces.
Merge-queue, main, and manual runs remain exhaustive.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
MAX_PR_CRATE_BUCKETS = 3
FULL_EVENTS = {"merge_group", "push", "workflow_call", "workflow_dispatch", "schedule"}
IGNORED_PREFIXES = ("docs/", ".github/ISSUE_TEMPLATE/")
FULL_PR_PATHS = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    ".cargo/config",
    ".cargo/config.toml",
    ".github/workflows/reborn-tests.yml",
    ".github/workflows/coverage.yml",
    ".github/workflows/nightly-deep-ci.yml",
    ".github/workflows/reborn-e2e.yml",
    ".github/workflows/reborn-playwright.yml",
    ".github/workflows/reborn-release-compile.yml",
    "scripts/ci/reborn_pr_test_plan.py",
    "scripts/ci/test_reborn_pr_test_plan.py",
    "scripts/ci/discover-reborn-package-crates.sh",
    "scripts/ci/reborn-crate-test-buckets.sh",
    "scripts/ci/package-feature-flags.sh",
    "scripts/ci/run-hermetic-deterministic-suite.sh",
    "scripts/ci/run-reborn-root-partition.sh",
    "scripts/ci/run-reborn-group-tests.sh",
    "scripts/ci/reborn-coverage-int-tier-tests.sh",
    "scripts/ci/reborn-coverage-lane-run.sh",
    "tests/integration/coverage-exemptions.toml",
    "tests/integration/coverage-floor.toml",
}
BUCKET_WEIGHTS = {
    "reborn-core": 12,
    "auth-security": 9,
    "extension-operator": 8,
    "product-workflow": 8,
    "webui-ingress": 8,
    "composition-core": 8,
    "wasm-sandbox": 8,
    "agent-runtime": 7,
    "llm-mcp": 7,
    "events-conversations": 7,
    "host-runtime": 6,
    "channel-adapters": 6,
    "architecture-misc": 5,
    "memory-skills": 5,
}


def _run(*argv: str) -> str:
    return subprocess.run(
        argv,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def _metadata() -> dict[str, Any]:
    return json.loads(_run("cargo", "metadata", "--format-version", "1"))


def _canonical_packages() -> list[str]:
    return json.loads(_run("scripts/ci/discover-reborn-package-crates.sh"))


def _bucket_packages(packages: list[str]) -> list[dict[str, Any]]:
    return json.loads(
        _run("scripts/ci/reborn-crate-test-buckets.sh", json.dumps(packages))
    )


def _bound_pr_buckets(
    buckets: list[dict[str, Any]], max_buckets: int = MAX_PR_CRATE_BUCKETS
) -> list[dict[str, Any]]:
    """Pack canonical buckets into bounded PR jobs without splitting them."""
    if len(buckets) <= max_buckets:
        return buckets

    bounded = [
        {"name": f"affected-{index + 1}", "packages": []}
        for index in range(min(max_buckets, len(buckets)))
    ]
    weights = [0] * len(bounded)
    ordered = sorted(
        buckets,
        key=lambda bucket: (
            -BUCKET_WEIGHTS.get(
                str(bucket.get("name")), len(bucket.get("packages", []))
            ),
            str(bucket.get("name")),
        ),
    )
    for bucket in ordered:
        target = min(range(len(bounded)), key=lambda index: (weights[index], index))
        bounded[target]["packages"].extend(bucket["packages"])
        weights[target] += BUCKET_WEIGHTS.get(
            str(bucket.get("name")), len(bucket.get("packages", []))
        )
    return bounded


def _root_test_partitions() -> dict[str, int]:
    support_tests = (
        ["support_unit_tests"]
        if (ROOT / "tests/support_unit_tests.rs").is_file()
        else []
    )
    names = sorted(
        [
            path.stem
            for path in (ROOT / "tests").glob("reborn_*.rs")
            if path.is_file()
        ]
        + support_tests
    )
    return {f"tests/{name}.rs": index % 4 for index, name in enumerate(names)}


def _integration_test_lanes() -> dict[str, str | int]:
    with (ROOT / "Cargo.toml").open("rb") as manifest:
        data = tomllib.load(manifest)
    tests = {
        entry["path"]: entry["name"]
        for entry in data.get("test", [])
        if isinstance(entry, dict)
        and isinstance(entry.get("name"), str)
        and isinstance(entry.get("path"), str)
        and entry["path"].startswith("tests/integration/")
    }
    flat_names = sorted(
        name
        for name in tests.values()
        if name.startswith(("reborn_integration_", "reborn_generated_"))
    )
    flat_lanes = {name: index % 4 for index, name in enumerate(flat_names)}
    return {
        path: "groups" if name.startswith("reborn_group_") else flat_lanes[name]
        for path, name in tests.items()
    }


def _workspace_packages(metadata: dict[str, Any]) -> tuple[dict[str, str], dict[str, set[str]]]:
    members = set(metadata["workspace_members"])
    packages_by_id = {
        package["id"]: package
        for package in metadata["packages"]
        if package["id"] in members
    }
    directories = {
        str(Path(package["manifest_path"]).resolve().parent.relative_to(ROOT)): package[
            "name"
        ]
        for package in packages_by_id.values()
        if Path(package["manifest_path"]).resolve().parent != ROOT
    }
    reverse: dict[str, set[str]] = defaultdict(set)
    for node in metadata["resolve"]["nodes"]:
        if node["id"] not in packages_by_id:
            continue
        dependent = packages_by_id[node["id"]]["name"]
        for dependency in node["deps"]:
            if dependency["pkg"] in packages_by_id:
                reverse[packages_by_id[dependency["pkg"]]["name"]].add(dependent)
    return directories, reverse


def _affected_packages(changed: set[str], reverse: dict[str, set[str]]) -> set[str]:
    affected = set(changed)
    pending = list(changed)
    while pending:
        package = pending.pop()
        for dependent in reverse.get(package, set()):
            if dependent not in affected:
                affected.add(dependent)
                pending.append(dependent)
    return affected


def _full_plan(
    reason: str,
    canonical_packages: list[str],
) -> dict[str, Any]:
    return {
        "mode": "full",
        "reasons": [reason],
        "changed_packages": [],
        "affected_packages": canonical_packages,
        "crate_buckets": _bucket_packages(canonical_packages),
        "root_partitions": [0, 1, 2, 3],
        "integration_lanes": [0, 1, 2, 3, "groups"],
        "run_group_tests": True,
        "run_frontend": True,
        "run_qa_replay": True,
        "coverage_mode": "full",
    }


def build_plan(
    *,
    event: str,
    changed_paths: list[str],
    metadata: dict[str, Any],
    canonical_packages: list[str],
) -> dict[str, Any]:
    """Build a deterministic test plan, failing open on unknown Reborn inputs."""
    if event in FULL_EVENTS:
        return _full_plan(f"{event} requires exhaustive coverage", canonical_packages)
    if event != "pull_request":
        return _full_plan(f"unknown event {event!r}", canonical_packages)

    paths = {path.strip().replace("\\", "/") for path in changed_paths if path.strip()}
    if not paths:
        return _full_plan(
            "empty pull-request diff requires fail-closed exhaustive coverage",
            canonical_packages,
        )
    if any(path in FULL_PR_PATHS for path in paths):
        return _full_plan(
            "Reborn test infrastructure or workspace topology changed",
            canonical_packages,
        )

    package_directories, reverse = _workspace_packages(metadata)
    changed_packages: set[str] = set()
    root_partitions: set[int] = set()
    integration_lanes: set[str | int] = set()
    run_frontend = False
    # Recorded replay is a repository-wide ordering and integration sentinel,
    # not affected-area coverage. Keep it on for every pull request even when
    # no changed path maps to another Reborn lane.
    run_qa_replay = True
    qa_evidence_changed = False
    reasons: list[str] = []
    root_inventory = _root_test_partitions()
    integration_inventory = _integration_test_lanes()

    for path in sorted(paths):
        if path.startswith(IGNORED_PREFIXES) or (
            path.endswith(".md") and "/" not in path
        ):
            continue
        if path.startswith(".github/workflows/"):
            continue
        if path.startswith("crates/ironclaw_webui/frontend/"):
            run_frontend = True
            reasons.append("WebUI frontend changed")
            continue
        if path in root_inventory:
            root_partitions.add(root_inventory[path])
            reasons.append(f"root test changed: {path}")
            continue
        if (
            path.startswith("tests/support/reborn_parity_qa/")
            or path == "tests/support_unit_tests.rs"
        ):
            root_partitions.update(range(4))
            reasons.append("shared root-test support changed")
            continue
        if path in integration_inventory:
            integration_lanes.add(integration_inventory[path])
            reasons.append(f"integration test changed: {path}")
            continue
        if path.startswith("tests/integration/"):
            integration_lanes.update([0, 1, 2, 3, "groups"])
            reasons.append("shared integration support changed")
            continue
        if path.startswith("tests/fixtures/llm_traces/reborn_qa/") or path in {
            "scripts/ci/check-reborn-qa-fixtures.sh",
            "scripts/ci/test-check-reborn-qa-fixtures.sh",
            "scripts/ci/test-check-regression-promotions.py",
        }:
            qa_evidence_changed = True
            reasons.append("recorded QA evidence changed")
            continue
        if path.startswith("crates/"):
            package = next(
                (
                    name
                    for directory, name in package_directories.items()
                    if path == directory or path.startswith(f"{directory}/")
                ),
                None,
            )
            if package is None:
                return _full_plan(
                    f"unmapped crate path {path} requires fail-closed coverage",
                    canonical_packages,
                )
            changed_packages.add(package)
            reasons.append(f"production package changed: {package}")
            continue
        if path.startswith(("tests/reborn_", "tests/e2e/reborn_", "scripts/ci/reborn-")):
            return _full_plan(
                f"unmapped Reborn test path {path} requires fail-closed coverage",
                canonical_packages,
            )
        if path.startswith(("scripts/", "tests/", ".github/actions/")):
            return _full_plan(
                f"unmapped test or CI path {path} requires fail-closed coverage",
                canonical_packages,
            )
        return _full_plan(
            f"unclassified pull-request path {path} requires fail-closed coverage",
            canonical_packages,
        )

    canonical_set = set(canonical_packages)
    affected = _affected_packages(changed_packages, reverse) & canonical_set
    if changed_packages and not affected:
        return _full_plan(
            "changed packages are outside the canonical Reborn set; "
            "fail-closed exhaustive coverage required",
            canonical_packages,
        )

    buckets = _bucket_packages(sorted(affected)) if affected else []
    if len(buckets) > MAX_PR_CRATE_BUCKETS:
        original_bucket_count = len(buckets)
        buckets = _bound_pr_buckets(buckets)
        reasons.append(
            f"coalesced {original_bucket_count} affected crate buckets into "
            f"{len(buckets)} PR jobs without omitting packages"
        )
    active = bool(
        buckets
        or root_partitions
        or integration_lanes
        or run_frontend
        or qa_evidence_changed
    )
    return {
        "mode": "selected" if active else "none",
        "reasons": reasons or ["no Reborn test surface changed"],
        "changed_packages": sorted(changed_packages),
        "affected_packages": sorted(affected),
        "crate_buckets": buckets,
        "root_partitions": sorted(root_partitions),
        "integration_lanes": sorted(
            integration_lanes, key=lambda value: (isinstance(value, str), str(value))
        ),
        "run_group_tests": False,
        "run_frontend": run_frontend,
        "run_qa_replay": run_qa_replay,
        "coverage_mode": "none",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--event", required=True)
    parser.add_argument(
        "--changed-files",
        type=Path,
        help="newline-delimited changed paths; required for pull_request",
    )
    parser.add_argument(
        "--canonical-packages",
        type=Path,
        help="JSON package array produced by discover-reborn-package-crates.sh",
    )
    args = parser.parse_args()
    try:
        changed_paths = (
            args.changed_files.read_text(encoding="utf-8").splitlines()
            if args.changed_files
            else []
        )
        canonical_packages = (
            json.loads(args.canonical_packages.read_text(encoding="utf-8"))
            if args.canonical_packages
            else _canonical_packages()
        )
        plan = build_plan(
            event=args.event,
            changed_paths=changed_paths,
            metadata=_metadata(),
            canonical_packages=canonical_packages,
        )
    except (OSError, KeyError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Reborn PR test planner failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(plan, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
