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
# Path classes with no Rust or E2E surface any Reborn lane can exercise.
# `.claude/` is agent guidance (skills, commands, rules) — prose in the same
# class as `docs/`. It was unclassified until 2026-08-03, which meant the
# planner's fail-closed arm rejected every PR that touched agent guidance:
# the only satisfiable behaviour for that class was "never edit it", which is
# not a policy anyone chose. Classifying it is the fix; loosening the
# fail-closed arm is not.
IGNORED_PREFIXES = ("docs/", ".claude/", ".github/ISSUE_TEMPLATE/")
DEDICATED_WORKFLOW_PREFIXES = ("tools/ironclaw_stress/",)
CHANGED_COVERAGE_MANIFEST = "tests/integration/changed-coverage-exemptions.toml"
PR_STATIC_CONTROL_PATHS = {
    "Cargo.toml",
    "rust-toolchain",
    "rust-toolchain.toml",
    ".cargo/config",
    ".cargo/config.toml",
    "tests/integration/coverage-exemptions.toml",
    "tests/integration/coverage-floor.toml",
    # Repo-root `scripts/` is deliberately NOT prefix-classified — the
    # `unmapped test or CI path` arm below exists to force a per-file decision.
    # These two are decided:
    #   * the panic baseline is enforced by Code Style
    #     (`check_no_panics.py --reborn-baseline`), which runs on every PR and
    #     owns the whole check; no Reborn lane reads it.
    #   * `reborn-e2e-rust.sh` is driven by the `Reborn E2E` workflow, which has
    #     its own scope detector (`Detect Reborn E2E scope`). This planner
    #     selects lanes for `Tests (Reborn)` only, and that workflow does not
    #     invoke the script.
    "scripts/no_panics_reborn_baseline.txt",
    "scripts/reborn-e2e-rust.sh",
}
PR_STATIC_CONTROL_PREFIXES = (".github/workflows/", "scripts/ci/")
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


def _lockfile_change_is_manifest_owned(
    *,
    current: dict[str, Any],
    base: dict[str, Any],
    changed_paths: set[str],
    metadata: dict[str, Any],
) -> bool:
    """Return true only for lockfile edits confined to changed workspace manifests."""
    if {key: value for key, value in current.items() if key != "package"} != {
        key: value for key, value in base.items() if key != "package"
    }:
        return False

    workspace_members = set(metadata["workspace_members"])
    workspace_packages = {
        package["name"]: str(
            Path(package["manifest_path"]).resolve().relative_to(ROOT)
        )
        for package in metadata["packages"]
        if package["id"] in workspace_members
    }
    changed_workspace_packages = {
        name for name, manifest in workspace_packages.items() if manifest in changed_paths
    }
    if not changed_workspace_packages:
        return False

    def indexed(lockfile: dict[str, Any]) -> dict[tuple[str, str, str], dict[str, Any]]:
        packages = lockfile.get("package", [])
        if not isinstance(packages, list):
            return {}
        result = {
            (
                str(package.get("name", "")),
                str(package.get("version", "")),
                str(package.get("source", "")),
            ): package
            for package in packages
            if isinstance(package, dict)
        }
        return result if len(result) == len(packages) else {}

    current_packages = indexed(current)
    base_packages = indexed(base)
    if not current_packages or current_packages.keys() != base_packages.keys():
        return False

    for key, current_package in current_packages.items():
        base_package = base_packages[key]
        if current_package == base_package:
            continue
        name, _version, source = key
        if source or name not in changed_workspace_packages:
            return False
        current_without_dependencies = {
            field: value
            for field, value in current_package.items()
            if field != "dependencies"
        }
        base_without_dependencies = {
            field: value for field, value in base_package.items() if field != "dependencies"
        }
        if current_without_dependencies != base_without_dependencies:
            return False
    return True


def _manifest_owned_lockfile_change(
    *, base_sha: str, changed_paths: set[str], metadata: dict[str, Any]
) -> bool:
    if not base_sha:
        return False
    return _lockfile_change_is_manifest_owned(
        current=tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8")),
        base=tomllib.loads(_run("git", "show", f"{base_sha}:Cargo.lock")),
        changed_paths=changed_paths,
        metadata=metadata,
    )


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
        "run_qa_replay": True,
        "coverage_mode": "full",
    }


def build_plan(
    *,
    event: str,
    changed_paths: list[str],
    metadata: dict[str, Any],
    canonical_packages: list[str],
    lockfile_manifest_owned: bool = False,
) -> dict[str, Any]:
    """Build a deterministic test plan, rejecting unknown PR inputs."""
    if event in FULL_EVENTS:
        return _full_plan(f"{event} requires exhaustive coverage", canonical_packages)
    if event != "pull_request":
        return _full_plan(f"unknown event {event!r}", canonical_packages)

    paths = {path.strip().replace("\\", "/") for path in changed_paths if path.strip()}
    if not paths:
        raise ValueError(
            "empty pull-request diff cannot be classified; refusing to launch "
            "an unbounded PR matrix"
        )

    package_directories, reverse = _workspace_packages(metadata)
    production_packages: set[str] = set()
    direct_test_packages: set[str] = set()
    exact_test_targets: dict[str, set[tuple[str, str]]] = defaultdict(set)
    packages_requiring_all_targets: set[str] = set()
    root_partitions: set[int] = set()
    integration_lanes: set[str | int] = set()
    # Recorded replay is a repository-wide ordering and integration sentinel,
    # not affected-area coverage. Keep it on for every pull request even when
    # no changed path maps to another Reborn lane.
    run_qa_replay = True
    qa_evidence_changed = False
    reasons: list[str] = []
    root_inventory = _root_test_partitions()
    integration_inventory = _integration_test_lanes()

    for path in sorted(paths):
        if path == "Cargo.lock":
            if lockfile_manifest_owned:
                reasons.append(
                    "Cargo.lock change is owned by a changed crate manifest"
                )
            else:
                reasons.append(
                    "workspace lockfile breadth is deferred to the exhaustive merge-queue gate"
                )
            continue
        if path in PR_STATIC_CONTROL_PATHS or path.startswith(
            PR_STATIC_CONTROL_PREFIXES
        ):
            reasons.append(f"static CI or workspace-policy checks own: {path}")
            continue
        if path.startswith(DEDICATED_WORKFLOW_PREFIXES):
            reasons.append(f"dedicated stress workflow owns: {path}")
            continue
        if path == CHANGED_COVERAGE_MANIFEST:
            reasons.append("changed-coverage policy is statically validated")
            continue
        if path.startswith(IGNORED_PREFIXES) or (
            path.endswith(".md") and "/" not in path
        ):
            continue
        if path.startswith("crates/ironclaw_webui/frontend/"):
            reasons.append("Code Style owns WebUI lint, tests, and production build")
            continue
        if path in root_inventory:
            root_partitions.add(root_inventory[path])
            reasons.append(f"root test changed: {path}")
            continue
        if (
            path.startswith("tests/support/reborn_parity_qa/")
            or path == "tests/support_unit_tests.rs"
        ):
            root_partitions.add(0)
            reasons.append(
                "shared root-test support changed; PR runs a representative partition"
            )
            continue
        if path in integration_inventory:
            integration_lanes.add(integration_inventory[path])
            reasons.append(f"integration test changed: {path}")
            continue
        if path.startswith("tests/integration/"):
            integration_lanes.add(0)
            reasons.append(
                "shared integration support changed; PR runs a representative lane"
            )
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
                raise ValueError(f"unmapped crate path: {path}")
            directory = next(
                directory
                for directory, name in package_directories.items()
                if name == package
            )
            relative = path.removeprefix(f"{directory}/")
            if relative.startswith(("tests/", "benches/", "examples/")):
                direct_test_packages.add(package)
                reasons.append(f"package-owned test surface changed: {package}")
                parts = Path(relative).parts
                target_kinds = {
                    "tests": "test",
                    "benches": "bench",
                    "examples": "example",
                }
                if len(parts) == 2 and Path(parts[1]).suffix == ".rs":
                    exact_test_targets[package].add(
                        (target_kinds[parts[0]], Path(parts[1]).stem)
                    )
                else:
                    packages_requiring_all_targets.add(package)
            else:
                production_packages.add(package)
                reasons.append(f"production package changed: {package}")
            continue
        if path.startswith(("tests/reborn_", "tests/e2e/reborn_", "scripts/ci/reborn-")):
            raise ValueError(f"unmapped Reborn test path: {path}")
        if path.startswith(("scripts/", "tests/", ".github/actions/")):
            raise ValueError(f"unmapped test or CI path: {path}")
        raise ValueError(f"unclassified pull-request path: {path}")

    canonical_set = set(canonical_packages)
    changed_packages = production_packages | direct_test_packages
    affected = (
        _affected_packages(production_packages, reverse) | direct_test_packages
    ) & canonical_set
    if changed_packages and not affected:
        raise ValueError(
            "changed packages are outside the canonical Reborn package set: "
            f"{', '.join(sorted(changed_packages))}"
        )

    buckets = _bucket_packages(sorted(affected)) if affected else []
    full_target_packages = (
        _affected_packages(production_packages, reverse)
        | packages_requiring_all_targets
    ) & canonical_set
    for bucket in buckets:
        bucket_packages = set(bucket["packages"])
        if bucket_packages & full_target_packages:
            continue
        if not all(package in exact_test_targets for package in bucket_packages):
            continue
        bucket["exact_targets"] = [
            {"package": package, "kind": kind, "name": name}
            for package in sorted(bucket_packages)
            for kind, name in sorted(exact_test_targets[package])
        ]
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
    parser.add_argument(
        "--base-sha",
        help="pull-request base commit used to verify manifest-owned lockfile edits",
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
        normalized_paths = {
            path.strip().replace("\\", "/") for path in changed_paths if path.strip()
        }
        metadata = _metadata()
        lockfile_manifest_owned = (
            _manifest_owned_lockfile_change(
                base_sha=args.base_sha or "",
                changed_paths=normalized_paths,
                metadata=metadata,
            )
            if "Cargo.lock" in normalized_paths
            else False
        )
        plan = build_plan(
            event=args.event,
            changed_paths=changed_paths,
            metadata=metadata,
            canonical_packages=canonical_packages,
            lockfile_manifest_owned=lockfile_manifest_owned,
        )
    except (OSError, KeyError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Reborn PR test planner failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(plan, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
