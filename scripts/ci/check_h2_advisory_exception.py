#!/usr/bin/env python3
"""Keep the h2 advisory waiver scoped to libSQL's unpatchable legacy line."""

from __future__ import annotations

import pathlib
import re
import sys

ADVISORY_ID = "RUSTSEC-2026-0258"
LEGACY_VERSION = "0.3.27"
PATCHED_VERSION = (0, 4, 16)


def _version_tuple(version: str) -> tuple[int, ...] | None:
    try:
        return tuple(int(part) for part in version.split("."))
    except ValueError:
        return None


def validate(versions: list[str], ignored: set[str]) -> list[str]:
    errors: list[str] = []
    legacy_present = LEGACY_VERSION in versions
    for version in versions:
        parsed = _version_tuple(version)
        if parsed is None:
            errors.append(f"cannot classify h2 version {version!r}")
        elif parsed < PATCHED_VERSION and version != LEGACY_VERSION:
            errors.append(
                f"unexpected vulnerable h2 {version}; only legacy {LEGACY_VERSION} is waived"
            )
    if legacy_present and ADVISORY_ID not in ignored:
        errors.append(f"{ADVISORY_ID} waiver is missing while h2 {LEGACY_VERSION} remains")
    if not legacy_present and ADVISORY_ID in ignored:
        errors.append(
            f"remove {ADVISORY_ID} waiver because h2 {LEGACY_VERSION} is no longer present"
        )
    return errors


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[2]
    lock = (root / "Cargo.lock").read_text(encoding="utf-8")
    versions = []
    for package in lock.split("[[package]]")[1:]:
        if re.search(r'^name = "h2"$', package, re.MULTILINE):
            match = re.search(r'^version = "([^"]+)"$', package, re.MULTILINE)
            if match:
                versions.append(match.group(1))
    deny = (root / "deny.toml").read_text(encoding="utf-8")
    advisories = deny.split("[advisories]", 1)[-1].split("\n[", 1)[0]
    ignored = {ADVISORY_ID} if f'"{ADVISORY_ID}"' in advisories else set()
    errors = validate(versions, ignored)
    if errors:
        for error in errors:
            print(f"h2 advisory exception: {error}", file=sys.stderr)
        return 1
    print(f"h2 advisory exception: OK ({', '.join(versions)})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
