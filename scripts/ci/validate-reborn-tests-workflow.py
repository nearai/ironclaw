#!/usr/bin/env python3
"""Validate the dynamic matrices in .github/workflows/reborn-tests.yml."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "reborn-tests.yml"
EXPECTED_WEBUI_TESTS = {
    "webui_v2_descriptors_contract",
    "webui_v2_handlers_contract",
    "webui_v2_operator_config_key_contract",
    "webui_v2_operator_route_predicate_contract",
    "webui_v2_schema_contract",
}


def cargo_metadata() -> dict:
    raw = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
    )
    return json.loads(raw)


def reborn_package_matrix(metadata: dict) -> list[str]:
    packages = []
    for package in metadata["packages"]:
        name = package["name"]
        if (
            name.startswith("ironclaw_reborn")
            or name in {"ironclaw_product_workflow", "ironclaw_product_adapters"}
            or name.startswith("ironclaw_webui_v2")
        ) and name != "ironclaw_webui_v2":
            packages.append(name)
    return sorted(set(packages))


def webui_target_matrix(metadata: dict) -> list[dict[str, str]]:
    for package in metadata["packages"]:
        if package["name"] != "ironclaw_webui_v2":
            continue
        targets = []
        for target in package["targets"]:
            kinds = set(target["kind"])
            if "lib" in kinds:
                targets.append({"name": "lib", "kind": "lib"})
            elif "test" in kinds:
                targets.append({"name": target["name"], "kind": "test"})
        return sorted({(item["name"], item["kind"]) for item in targets})
    raise AssertionError("ironclaw_webui_v2 package missing from cargo metadata")


def main() -> None:
    workflow = WORKFLOW.read_text()
    assert '(.name != "ironclaw_webui_v2")' in workflow
    assert '"kind": "lib"' in workflow
    assert '"kind": "test"' in workflow
    assert '--test "$TARGET_NAME"' in workflow

    metadata = cargo_metadata()
    packages = reborn_package_matrix(metadata)
    assert "ironclaw_webui_v2" not in packages
    assert "ironclaw_webui_v2_static" in packages

    targets = webui_target_matrix(metadata)
    target_names = {name for name, _kind in targets}
    assert ("lib", "lib") in targets
    assert EXPECTED_WEBUI_TESTS <= target_names, (
        "missing expected ironclaw_webui_v2 test targets: "
        f"{sorted(EXPECTED_WEBUI_TESTS - target_names)}"
    )

    print("reborn-tests workflow matrix validation passed")


if __name__ == "__main__":
    main()
