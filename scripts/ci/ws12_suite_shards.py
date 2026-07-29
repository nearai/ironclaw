#!/usr/bin/env python3
"""Validate and generate affinity-safe, duration-balanced CI shards."""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

import tomllib


class PolicyError(ValueError):
    """A checked-in scheduling contract is incomplete or unsafe."""


def load_policy(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def _required_text(record: dict, key: str, label: str) -> str:
    value = record.get(key)
    if not isinstance(value, str) or not value.strip():
        raise PolicyError(f"{label} has missing/invalid {key}")
    return value


def validate_and_generate(policy: dict, repo_root: Path) -> list[dict]:
    config = policy.get("playwright")
    if not isinstance(config, dict):
        raise PolicyError("missing [playwright] policy")

    shard_count = config.get("shard_count")
    budget = config.get("max_shard_seconds")
    patterns = config.get("discovery_globs")
    if not isinstance(shard_count, int) or shard_count < 1:
        raise PolicyError("playwright.shard_count must be a positive integer")
    if not isinstance(budget, int) or budget < 1:
        raise PolicyError("playwright.max_shard_seconds must be a positive integer")
    if not isinstance(patterns, list) or not patterns:
        raise PolicyError("playwright.discovery_globs must be a non-empty list")

    discovered: set[str] = set()
    for pattern in patterns:
        if not isinstance(pattern, str) or not pattern:
            raise PolicyError("playwright.discovery_globs contains an invalid pattern")
        discovered.update(
            path.relative_to(repo_root).as_posix() for path in repo_root.glob(pattern)
        )
    if not discovered:
        raise PolicyError("playwright discovery matched no suites")

    suites = policy.get("suite", [])
    if not isinstance(suites, list) or not suites:
        raise PolicyError("policy contains no [[suite]] entries")

    by_path: dict[str, dict] = {}
    affinity_members: dict[str, list[dict]] = defaultdict(list)
    for index, suite in enumerate(suites):
        if not isinstance(suite, dict):
            raise PolicyError(f"suite[{index}] is not a table")
        label = f"suite[{index}]"
        path = _required_text(suite, "path", label)
        affinity = _required_text(suite, "affinity", label)
        retry = _required_text(suite, "retry", label)
        seconds = suite.get("historical_seconds")
        if path in by_path:
            raise PolicyError(f"duplicate suite entry: {path}")
        if not isinstance(seconds, int) or seconds < 1:
            raise PolicyError(f"{path} has invalid historical_seconds")
        if retry != "never":
            raise PolicyError(
                f'{path} is deterministic/protected and must set retry = "never"'
            )
        if not (repo_root / path).is_file():
            raise PolicyError(f"suite path does not exist: {path}")
        by_path[path] = suite
        affinity_members[affinity].append(suite)

    waivers = policy.get("waiver", [])
    if not isinstance(waivers, list):
        raise PolicyError("[[waiver]] entries must be tables")
    waived: set[str] = set()
    for index, waiver in enumerate(waivers):
        if not isinstance(waiver, dict):
            raise PolicyError(f"waiver[{index}] is not a table")
        label = f"waiver[{index}]"
        path = _required_text(waiver, "path", label)
        _required_text(waiver, "owner", label)
        _required_text(waiver, "reason", label)
        evidence = _required_text(waiver, "evidence", label)
        if path in waived or path in by_path:
            raise PolicyError(f"duplicate suite/waiver classification: {path}")
        if not (repo_root / path).is_file():
            raise PolicyError(f"waived suite path does not exist: {path}")
        if not (repo_root / evidence).is_file():
            raise PolicyError(f"waiver evidence does not exist: {evidence}")
        waived.add(path)

    classified = set(by_path) | waived
    missing = sorted(discovered - classified)
    stale = sorted(classified - discovered)
    if missing:
        raise PolicyError(
            f"discovered suites missing from policy: {', '.join(missing)}"
        )
    if stale:
        raise PolicyError(f"policy entries outside discovery scope: {', '.join(stale)}")

    bundles: list[tuple[int, str, list[str]]] = []
    for affinity, members in affinity_members.items():
        seconds = sum(member["historical_seconds"] for member in members)
        if seconds > budget:
            raise PolicyError(
                f"affinity group {affinity} costs {seconds}s, above {budget}s shard budget"
            )
        bundles.append(
            (seconds, affinity, sorted(member["path"] for member in members))
        )

    # Longest-processing-time greedy placement. Affinity groups are indivisible.
    shards = [{"seconds": 0, "affinities": [], "files": []} for _ in range(shard_count)]
    for seconds, affinity, files in sorted(
        bundles, key=lambda item: (-item[0], item[1])
    ):
        target = min(
            range(shard_count), key=lambda index: (shards[index]["seconds"], index)
        )
        shards[target]["seconds"] += seconds
        shards[target]["affinities"].append(affinity)
        shards[target]["files"].extend(files)

    matrix: list[dict] = []
    assigned: list[str] = []
    for index, shard in enumerate(shards, start=1):
        if not shard["files"]:
            continue
        if shard["seconds"] > budget:
            raise PolicyError(
                f"generated shard {index} costs {shard['seconds']}s, above {budget}s budget"
            )
        assigned.extend(shard["files"])
        matrix.append(
            {
                "group": f"duration-{index}-" + "+".join(sorted(shard["affinities"])),
                "files": " ".join(sorted(shard["files"])),
                "historical_seconds": shard["seconds"],
                "affinities": ",".join(sorted(shard["affinities"])),
                "retry": "never",
            }
        )

    if len(assigned) != len(set(assigned)) or set(assigned) != set(by_path):
        raise PolicyError("generated matrix does not assign every suite exactly once")
    return matrix


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--manifest",
        type=Path,
        default=Path("scripts/ci/ws12-suite-shards.toml"),
    )
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument(
        "--github-output",
        action="store_true",
        help="emit matrix=<compact JSON> for GITHUB_OUTPUT",
    )
    args = parser.parse_args()
    try:
        matrix = validate_and_generate(
            load_policy(args.manifest), args.repo_root.resolve()
        )
    except (OSError, tomllib.TOMLDecodeError, PolicyError) as error:
        print(f"WS12 shard policy failed: {error}", file=sys.stderr)
        return 1
    encoded = json.dumps(matrix, separators=(",", ":"), sort_keys=True)
    print(f"matrix={encoded}" if args.github_output else encoded)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
