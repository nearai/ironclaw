#!/usr/bin/env python3
"""Verify the deterministic ICWM G0C tracked publication manifest."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "PUBLICATION-MANIFEST.json"
DIGEST = ROOT / "PUBLICATION-MANIFEST.sha256"


def main() -> None:
    manifest_bytes = MANIFEST.read_bytes()
    expected_manifest_digest = DIGEST.read_text(encoding="utf-8").split()[0]
    actual_manifest_digest = hashlib.sha256(manifest_bytes).hexdigest()
    if actual_manifest_digest != expected_manifest_digest:
        raise SystemExit("publication manifest digest mismatch")

    manifest = json.loads(manifest_bytes)
    paths = [entry["path"] for entry in manifest["entries"]]
    if paths != sorted(paths, key=lambda value: value.encode("utf-8")):
        raise SystemExit("publication manifest entries are not bytewise path-sorted")
    if len(paths) != len(set(paths)):
        raise SystemExit("publication manifest contains duplicate paths")

    lines: list[str] = []
    for entry in manifest["entries"]:
        path = ROOT / entry["path"]
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        if len(data) != entry["bytes"] or digest != entry["sha256"]:
            raise SystemExit(f"publication artifact mismatch: {entry['path']}")
        lines.append(f"{digest}  {entry['path']}\n")

    aggregate = hashlib.sha256("".join(lines).encode("utf-8")).hexdigest()
    if aggregate != manifest["aggregate_sha256"]:
        raise SystemExit("publication aggregate mismatch")
    print(actual_manifest_digest)
    print(aggregate)


if __name__ == "__main__":
    main()
