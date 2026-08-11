#!/usr/bin/env python3
"""Verify the deterministic ICWM G0C tracked publication manifest."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path


ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "PUBLICATION-MANIFEST.json"
DIGEST = ROOT / "PUBLICATION-MANIFEST.sha256"
CONTRACT_VERSION = b"icwm.g0c.harness.v1"
PAYLOAD_ROOTS = (
    ROOT,
    ROOT.parent.parent / "docs/internal/research/icwm-g0c",
)
EXCLUDED_PAYLOAD_FILES = frozenset(
    {
        "PUBLICATION-MANIFEST.json",
        "PUBLICATION-MANIFEST.sha256",
    }
)
EXCLUDED_GENERATED_DIRECTORIES = frozenset(
    {
        "target",
        "__pycache__",
        ".pytest_cache",
    }
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stable_id(domain: str, components: list[bytes]) -> str:
    """Mirror the harness's typed, length-delimited StableId construction."""
    digest = hashlib.sha256()
    for value in (CONTRACT_VERSION, domain.encode("utf-8")):
        digest.update(len(value).to_bytes(8, "big"))
        digest.update(value)
    digest.update(len(components).to_bytes(8, "big"))
    for component in components:
        digest.update(len(component).to_bytes(8, "big"))
        digest.update(component)
    return digest.hexdigest()


def canonical_json(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def payload_relative_path(path: Path) -> str:
    return Path(os.path.relpath(path, ROOT)).as_posix()


def enumerate_payload_files() -> set[str]:
    files: set[str] = set()
    for payload_root in PAYLOAD_ROOTS:
        if not payload_root.is_dir():
            raise SystemExit(f"declared payload root is missing: {payload_root}")
        for current, directory_names, file_names in os.walk(
            payload_root, topdown=True, followlinks=False
        ):
            current_path = Path(current)
            for directory_name in tuple(directory_names):
                directory = current_path / directory_name
                if directory.is_symlink():
                    raise SystemExit(
                        f"publication payload contains unsupported symlink: "
                        f"{payload_relative_path(directory)}"
                    )
            directory_names[:] = [
                name
                for name in directory_names
                if name not in EXCLUDED_GENERATED_DIRECTORIES
            ]
            for file_name in file_names:
                path = current_path / file_name
                if path.is_symlink():
                    raise SystemExit(
                        f"publication payload contains unsupported symlink: "
                        f"{payload_relative_path(path)}"
                    )
                relative = payload_relative_path(path)
                if payload_root == ROOT and relative in EXCLUDED_PAYLOAD_FILES:
                    continue
                files.add(relative)
    return files


def validate_manifest_completeness(paths: list[str]) -> None:
    manifest_paths = set(paths)
    payload_files = enumerate_payload_files()
    missing = sorted(payload_files - manifest_paths, key=lambda value: value.encode())
    outside = sorted(manifest_paths - payload_files, key=lambda value: value.encode())
    if missing:
        raise SystemExit(f"unlisted publication payload files: {', '.join(missing)}")
    if outside:
        raise SystemExit(
            f"manifest entries outside declared payload roots: {', '.join(outside)}"
        )


def validate_named_evidence(manifest: dict[str, object]) -> None:
    scenario_path = ROOT / "fixtures/CONTROL-SCENARIO.json"
    result_path = ROOT / "fixtures/CONTROL-RESULT.json"
    scenario = json.loads(scenario_path.read_bytes())
    result = json.loads(result_path.read_bytes())

    if result["dependency_graph"]["artifact"] != "Cargo.lock":
        raise SystemExit("control dependency_graph artifact must be Cargo.lock")

    scenario_components = [
        scenario["schema_version"].encode("utf-8"),
        scenario["name"].encode("utf-8"),
        canonical_json(scenario["inputs"]),
        canonical_json(scenario["expected_effects"]),
        canonical_json(scenario["failpoints"]),
    ]
    if scenario["scenario_id"] != stable_id("scenario", scenario_components):
        raise SystemExit("control scenario_id is not derived from scenario content")

    named_paths = {
        "dependency_graph": ROOT / result["dependency_graph"]["artifact"],
        "scenario_hash": scenario_path,
        "identifier_admission_corpus": ROOT / "fixtures/IDENTIFIER-FIXTURE.json",
        "request_vectors_v1": ROOT / "fixtures/REQUEST-VECTORS-v1.json",
        "tested_harness_source": ROOT / "src/lib.rs",
    }
    named_values = {
        "dependency_graph": result["dependency_graph"]["sha256"],
        "scenario_hash": result["scenario_hash"],
        "identifier_admission_corpus": result["vector_hashes"][
            "identifier_admission_corpus"
        ],
        "request_vectors_v1": result["vector_hashes"]["request_vectors_v1"],
        "tested_harness_source": result["evidence_hashes"]["tested_harness_source"],
    }
    for name, path in named_paths.items():
        if named_values[name] != sha256(path):
            raise SystemExit(f"named evidence hash mismatch: {name}")

    candidate = result["candidate"]
    expected_candidate = stable_id(
        "candidate",
        [candidate["name"].encode("utf-8"), candidate["version"].encode("utf-8")],
    )
    if candidate["stable_id"] != expected_candidate:
        raise SystemExit("control candidate stable_id mismatch")

    baseline = manifest["tested_source_baseline"]
    if result["harness_commit"] != baseline:
        raise SystemExit("harness_commit does not match tested source baseline")
    if result["component_commits"].get("ironclaw_source_baseline") != baseline:
        raise SystemExit("component source baseline mismatch")
    if (
        result["evidence_hashes"]["approved_ignored_common_aggregate"]
        != manifest["approved_ignored_common_aggregate_sha256"]
    ):
        raise SystemExit("approved ignored common aggregate mismatch")


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
    validate_manifest_completeness(paths)

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
    validate_named_evidence(manifest)
    print(actual_manifest_digest)
    print(aggregate)


if __name__ == "__main__":
    main()
