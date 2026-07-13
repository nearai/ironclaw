#!/usr/bin/env python3
"""Select live-QA cases that can run without Google OAuth."""

from __future__ import annotations

import argparse
from collections.abc import Iterable
from pathlib import Path


def parse_cases(value: str) -> list[str]:
    return [case.strip() for case in value.split(",") if case.strip()]


def requires_google(cases: Iterable[str], google_cases: set[str]) -> bool:
    return any(case in google_cases for case in cases)


def retain_without_google(cases: Iterable[str], google_cases: set[str]) -> list[str]:
    return [case for case in cases if case not in google_cases]


def _append(path: Path | None, line: str) -> None:
    if path is not None:
        with path.open("a", encoding="utf-8") as output:
            output.write(f"{line}\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", required=True)
    parser.add_argument("--google-cases", required=True)
    parser.add_argument("--mode", choices=("needs-google", "suppress"), required=True)
    parser.add_argument("--status", default="unknown")
    parser.add_argument("--github-env", type=Path)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()

    cases = parse_cases(args.cases)
    google_cases = set(parse_cases(args.google_cases))
    if args.mode == "needs-google":
        return 0 if requires_google(cases, google_cases) else 1

    retained = retain_without_google(cases, google_cases)
    if not retained:
        _append(args.github_output, "skip_shard=1")
        print(
            "All selected cases require Google OAuth; "
            f"refresh status={args.status or 'unknown'}."
        )
        return 0

    joined = ",".join(retained)
    _append(args.github_env, f"CASES={joined}")
    _append(args.github_output, "skip_shard=0")
    print(f"Google OAuth unavailable; continuing with non-Google cases: {joined}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
