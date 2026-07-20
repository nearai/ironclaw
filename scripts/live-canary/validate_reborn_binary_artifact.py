#!/usr/bin/env python3
"""Validate a packaged Reborn live-canary binary artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


ARCHIVE_NAME = "ironclaw-reborn.tar.gz"
CHECKSUM_NAME = f"{ARCHIVE_NAME}.sha256"
MANIFEST_NAME = "manifest.json"


def _feature_set(value: object, *, field: str) -> set[str]:
    if isinstance(value, str):
        features = [feature.strip() for feature in value.split(",")]
    elif isinstance(value, list) and all(
        isinstance(feature, str) for feature in value
    ):
        features = [feature.strip() for feature in value]
    else:
        raise ValueError(f"{field} must be a comma-separated string or string array")

    if not features or any(not feature for feature in features):
        raise ValueError(f"{field} contains an empty feature")
    if len(features) != len(set(features)):
        raise ValueError(f"{field} contains duplicate features")
    return set(features)


def validate_artifact(
    artifact_dir: Path,
    expected_ref: str,
    expected_features: str,
) -> None:
    archive = artifact_dir / ARCHIVE_NAME
    checksum = artifact_dir / CHECKSUM_NAME
    manifest_path = artifact_dir / MANIFEST_NAME

    expected_checksum = checksum.read_text(encoding="utf-8").split()[0]
    actual_checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual_checksum != expected_checksum:
        raise ValueError(
            f"archive checksum mismatch: expected {expected_checksum}, got {actual_checksum}"
        )

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("format_version") != 1:
        raise ValueError("unsupported artifact manifest format")
    if manifest.get("product_ref") != expected_ref:
        raise ValueError("artifact product_ref does not match the requested commit")
    artifact_features = _feature_set(
        manifest.get("features"), field="artifact features"
    )
    required_features = _feature_set(expected_features, field="required features")
    missing_features = sorted(required_features - artifact_features)
    if missing_features:
        raise ValueError(
            "artifact is missing required live QA features: "
            + ", ".join(missing_features)
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_dir", type=Path)
    parser.add_argument("expected_ref")
    parser.add_argument("expected_features")
    args = parser.parse_args()
    validate_artifact(args.artifact_dir, args.expected_ref, args.expected_features)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
