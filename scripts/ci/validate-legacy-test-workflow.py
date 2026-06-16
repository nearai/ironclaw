#!/usr/bin/env python3
"""Validate the legacy test workflow sharding contract."""

from __future__ import annotations

import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "test.yml"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def shell_assignment(text: str, name: str) -> str:
    match = re.search(rf"^\s*{re.escape(name)}='([^']*)'$", text, re.MULTILINE)
    require(match is not None, f"missing {name} shell assignment")
    return match.group(1)


def json_assignment(text: str, name: str) -> list[dict[str, str]]:
    value = shell_assignment(text, name)
    parsed = json.loads(value)
    require(isinstance(parsed, list), f"{name} must be a JSON list")
    require(all(isinstance(item, dict) for item in parsed), f"{name} items must be objects")
    return parsed


def item_names(items: list[dict[str, str]]) -> list[str]:
    return [item.get("name", "") for item in items]


def main() -> None:
    text = WORKFLOW.read_text()

    all_features_flags = shell_assignment(text, "ALL_FEATURES_FLAGS")
    require(
        all_features_flags
        == "--no-default-features --features postgres,libsql,html-to-markdown,bedrock,import",
        "legacy all-features flags drifted",
    )

    full = json_assignment(text, "FULL")
    slim = json_assignment(text, "SLIM")
    windows_full = json_assignment(text, "WINDOWS_FULL")
    root_shards = json_assignment(text, "ROOT_SHARDS")
    extra_shards = json_assignment(text, "EXTRA_SHARDS")

    require(slim == [], "pull_request/merge_group legacy default matrix must stay slim")
    require(
        item_names(full) == ["default", "libsql-only"],
        "push default/libsql test matrix changed unexpectedly",
    )
    require(
        item_names(windows_full) == ["all-features", "default", "libsql-only"],
        "push/workflow_call Windows matrix changed unexpectedly",
    )

    require(len(root_shards) == 5, "legacy root matrix must contain one unit/doc shard plus four integration shards")
    require(
        root_shards[0] == {"name": "root unit/doc", "kind": "unit-doc", "partition": "0"},
        "first legacy root shard must be the unit/doc shard",
    )
    integration_shards = root_shards[1:]
    require(
        [item.get("kind") for item in integration_shards] == ["integration"] * 4,
        "legacy root integration shard kinds changed",
    )
    require(
        [item.get("partition") for item in integration_shards] == ["0", "1", "2", "3"],
        "legacy root integration partitions must remain 0..3",
    )
    require(len(set(item_names(root_shards))) == len(root_shards), "legacy root shard names must be unique")

    require(
        [item.get("suite") for item in extra_shards] == ["reborn-composition", "ironclaw-memory"],
        "legacy extra shard suites changed unexpectedly",
    )

    for output in (
        "has_test_matrix",
        "legacy_all_features_flags",
        "legacy_root_matrix",
        "legacy_extra_matrix",
        "test_matrix",
        "windows_matrix",
    ):
        require(f"{output}: ${{{{ steps.set.outputs.{output} }}}}" in text, f"matrix-config output {output} not exposed")

    require('echo "test_matrix=${SLIM}" >> "$GITHUB_OUTPUT"' in text, "pull/merge test_matrix must use SLIM")
    require('echo "has_test_matrix=false" >> "$GITHUB_OUTPUT"' in text, "pull/merge has_test_matrix must be false")
    require('echo "windows_matrix=${SLIM}" >> "$GITHUB_OUTPUT"' in text, "pull/merge windows_matrix must use SLIM")
    require('echo "test_matrix=${FULL}" >> "$GITHUB_OUTPUT"' in text, "push test_matrix must use FULL")
    require('echo "has_test_matrix=true" >> "$GITHUB_OUTPUT"' in text, "push has_test_matrix must be true")
    require('echo "windows_matrix=${WINDOWS_FULL}" >> "$GITHUB_OUTPUT"' in text, "push windows_matrix must use WINDOWS_FULL")

    require(
        "fromJSON(needs.matrix-config.outputs.legacy_root_matrix)" in text,
        "legacy root job must consume legacy_root_matrix via fromJSON",
    )
    require(
        "fromJSON(needs.matrix-config.outputs.legacy_extra_matrix)" in text,
        "legacy extra job must consume legacy_extra_matrix via fromJSON",
    )
    require(
        "needs.matrix-config.outputs.has_test_matrix == 'true'" in text,
        "legacy default tests job must be gated by has_test_matrix",
    )

    require("if: matrix.kind == 'integration'" in text, "WASM setup must stay gated to integration shards")
    require("legacy-workflow-contract" in text, "run-tests must depend on the legacy workflow contract job")
    require(
        'require_success "legacy-workflow-contract" "${{ needs.legacy-workflow-contract.result }}"' in text,
        "run-tests must require the legacy workflow contract job",
    )
    require(
        'require_success "legacy-all-features-root-tests" "${{ needs.legacy-all-features-root-tests.result }}"'
        in text,
        "run-tests must require legacy root shards",
    )
    require(
        'require_success "legacy-all-features-extra-tests" "${{ needs.legacy-all-features-extra-tests.result }}"'
        in text,
        "run-tests must require legacy extra shards",
    )

    print("legacy test workflow matrix contract OK")


if __name__ == "__main__":
    main()
