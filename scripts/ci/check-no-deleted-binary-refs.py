#!/usr/bin/env python3
"""Reject executable references to the removed gateway binary and fixtures."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


DEFAULT_ROOT = Path(__file__).resolve().parents[2]
SCAN_ROOTS = (
    Path(".github/workflows"),
    Path("scripts/live-canary"),
    Path("scripts/live_canary"),
    Path("scripts/reborn_webui_v2_live_qa"),
    Path("tests/e2e"),
)
EXECUTABLE_SUFFIXES = {".py", ".sh", ".txt", ".yaml", ".yml"}
FORBIDDEN = {
    "removed binary": re.compile(r"\bironclaw[-_]legacy\b"),
    "removed CLI flag": re.compile(r"(?<![\w-])--no-onboard(?![\w-])"),
    "removed binary fixture": re.compile(r"\bironclaw_binary\b"),
    "removed server fixture": re.compile(r"\bironclaw_server\b"),
}


def executable_files(root: Path):
    for relative_root in SCAN_ROOTS:
        scan_root = root / relative_root
        if not scan_root.exists():
            continue
        for path in scan_root.rglob("*"):
            if path.is_file() and path.suffix in EXECUTABLE_SUFFIXES:
                yield path


def violations(root: Path) -> list[str]:
    errors: list[str] = []
    for path in executable_files(root):
        text = path.read_text(encoding="utf-8")
        for line_number, line in enumerate(text.splitlines(), start=1):
            for label, pattern in FORBIDDEN.items():
                if match := pattern.search(line):
                    relative = path.relative_to(root)
                    errors.append(
                        f"{relative}:{line_number}: {label}: {match.group(0)}"
                    )
    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=DEFAULT_ROOT,
        help="repository root to scan (defaults to this checkout)",
    )
    return parser.parse_args()


def main() -> int:
    root = parse_args().root.resolve()
    errors = violations(root)
    if errors:
        raise SystemExit(
            "Deleted-binary references are not allowed in executable "
            "canary, E2E, or CI files:\n  " + "\n  ".join(errors)
        )
    print("No executable references to the deleted gateway binary remain.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
