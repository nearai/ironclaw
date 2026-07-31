#!/usr/bin/env python3
"""Require regression-test evidence for fix and high-risk pull requests."""

from __future__ import annotations

import argparse
import subprocess
import sys


HIGH_RISK_PREFIXES = (
    "crates/ironclaw_turns/src/",
    "crates/ironclaw_processes/src/",
    "crates/ironclaw_llm/src/",
    "crates/ironclaw_safety/src/",
)


def changed_files(repo: str, base: str, head: str) -> list[str]:
    result = subprocess.run(
        ["git", "-C", repo, "diff", "--name-only", f"{base}...{head}"],
        check=True,
        capture_output=True,
        text=True,
    )
    return [line for line in result.stdout.splitlines() if line]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--title", default="")
    parser.add_argument("--labels", default="")
    parser.add_argument("--body", default="")
    parser.add_argument("--author", default="")
    parser.add_argument("--approving-reviewers", default="")
    args = parser.parse_args()

    labels = {label.strip() for label in args.labels.split(",")}
    if "skip-regression-check" in labels or "[skip-regression-check]" in args.body:
        return 0

    files = changed_files(args.repo, args.base, args.head)
    fix = args.title.lower().startswith(("fix:", "fix(", "hotfix:", "bugfix:"))
    high_risk = any(path.startswith(prefix) for path in files for prefix in HIGH_RISK_PREFIXES)
    if not fix and not high_risk:
        return 0

    test_change = any(
        path.startswith("tests/")
        or path.endswith(("_test.rs", "_tests.rs", ".test.ts", ".test.mts", "_test.py", "test_.py"))
        or "/tests/" in path
        or path.startswith("scripts/ci/test-")
        for path in files
    )
    if test_change:
        return 0

    print("::warning::Please add regression tests or apply the skip-regression-check label.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
