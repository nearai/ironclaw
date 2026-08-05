"""Executable classification of the shipped provider capability surface."""

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INVENTORY_PATH = ROOT / "tests/e2e/fixtures/provider_capability_coverage.toml"
ASSET_ROOT = ROOT / "crates/extensions/packages"

# WS2's package colocation put the two `[memory]` provider packages under the
# same `packages/` root as the extension packages. They are not *provider*
# capabilities in this file's sense: `ironclaw.memory.*` is the
# provider-neutral memory contract, identical whichever backend is bound, and
# none of its tools declares `external_write` or `network` — there is no vendor
# boundary for a journey to seed or assert against. Excluding them keeps this
# inventory measuring exactly what it measured before the move.
# `crates/ironclaw_architecture_tests/tests/reborn_extension_specificity.rs` carves
# the same two packages out of the vendor vocabulary, for the same reason.
NON_PROVIDER_PACKAGE_DIRS = frozenset({"memory-native", "mem0"})


def shipped_provider_manifests() -> list[Path]:
    """Every shipped provider package manifest, memory providers excluded.

    Fails closed: an empty result, or a carve-out entry naming a package that
    is not there, means the tree moved under this inventory rather than that
    there is nothing to classify.
    """
    for name in sorted(NON_PROVIDER_PACKAGE_DIRS):
        carved = ASSET_ROOT / name / "manifest.toml"
        if not carved.is_file():
            raise AssertionError(
                f"NON_PROVIDER_PACKAGE_DIRS names {name!r}, but {carved} does not exist. "
                "A stale carve-out excludes nothing and a renamed package silently "
                "rejoins the provider inventory; repoint it in the same change that "
                "moved the package."
            )
    manifests = [
        path
        for path in sorted(ASSET_ROOT.glob("*/manifest.toml"))
        if path.parent.name not in NON_PROVIDER_PACKAGE_DIRS
    ]
    if not manifests:
        raise AssertionError(
            f"no package manifests under {ASSET_ROOT} — the provider inventory would "
            "classify an empty surface and every coverage assertion would pass "
            "vacuously."
        )
    return manifests


def _load_inventory() -> dict:
    with INVENTORY_PATH.open("rb") as inventory_file:
        return tomllib.load(inventory_file)


INVENTORY = _load_inventory()
CLASSIFICATIONS = INVENTORY["classifications"]
TESTED_CAPABILITY_IDS = frozenset(CLASSIFICATIONS["tested"])
LIVE_ONLY_CAPABILITY_IDS = frozenset(CLASSIFICATIONS["live_only"])
UNSUPPORTED_CAPABILITY_IDS = frozenset(CLASSIFICATIONS["unsupported"])
WAIVED_CAPABILITY_IDS = frozenset(
    capability
    for waiver in INVENTORY.get("waivers", [])
    for capability in waiver["capabilities"]
)
INTEGRATION_EVIDENCE = tuple(INVENTORY.get("integration_evidence", []))
INTEGRATION_EVIDENCE_CAPABILITY_IDS = frozenset(
    evidence["capability"] for evidence in INTEGRATION_EVIDENCE
)
ALL_CLASSIFIED_CAPABILITY_IDS = (
    TESTED_CAPABILITY_IDS
    | LIVE_ONLY_CAPABILITY_IDS
    | UNSUPPORTED_CAPABILITY_IDS
    | WAIVED_CAPABILITY_IDS
)


COVERAGE_BACKLOG = tuple(INVENTORY.get("coverage_backlog", []))
JOURNEY_EVIDENCE = tuple(INVENTORY.get("journey_evidence", []))
JOURNEY_EVIDENCE_CAPABILITY_IDS = frozenset(
    evidence["capability"] for evidence in JOURNEY_EVIDENCE
)


def backlogged_capabilities(rule: str) -> frozenset[str]:
    """Capabilities with an owned, expiring exemption from `rule`."""
    return frozenset(
        capability
        for entry in COVERAGE_BACKLOG
        if entry.get("rule") == rule
        for capability in entry.get("capabilities", [])
    )


def _production_extension_ids() -> set[str]:
    extension_ids = set()
    for manifest_path in shipped_provider_manifests():
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        if manifest.get("tools"):
            extension_ids.add(manifest["id"])
    return extension_ids


def _capability_operation_kinds() -> dict[str, str]:
    """Provider operation kind per capability, derived from shipped manifests.

    `external_write` identifies mutations and `network` identifies provider
    reads. Capabilities with neither effect are local operations, not provider
    reads; requiring seeded provider outcomes for those would fabricate a
    boundary that production never crosses.
    """
    kinds: dict[str, str] = {}
    for manifest_path in shipped_provider_manifests():
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        inherited_effects = manifest.get("mcp", {}).get("effects", [])
        for tool in manifest.get("tools", []):
            effects = [*inherited_effects, *tool.get("effects", [])]
            if "external_write" in effects:
                kind = "write"
            elif "network" in effects:
                kind = "read"
            else:
                kind = "local"
            kinds[tool["id"]] = kind
    return kinds


CAPABILITY_OPERATION_KINDS = _capability_operation_kinds()
WRITE_CAPABILITY_IDS = frozenset(
    capability_id
    for capability_id in TESTED_CAPABILITY_IDS
    if CAPABILITY_OPERATION_KINDS.get(capability_id) == "write"
)
READ_CAPABILITY_IDS = frozenset(
    capability_id
    for capability_id in TESTED_CAPABILITY_IDS
    if CAPABILITY_OPERATION_KINDS.get(capability_id) == "read"
)

# Epic #6524 workstream 5: "seeded success and empty-result tests for every
# provider read operation". Transport and status faults stay in the reusable
# fault profiles (#6589); these are the per-operation semantic outcomes that
# no fault profile can stand in for.
REQUIRED_READ_OUTCOME_CLASSES = frozenset({"success", "empty"})


PROVIDER_WIRE_PREFIXES = tuple(
    f"{extension_id.replace('.', '__')}__"
    for extension_id in sorted(_production_extension_ids())
)


def capability_id_to_wire_name(capability_id: str) -> str:
    """Translate a canonical manifest ID to the model-facing wire name."""
    return capability_id.replace(".", "__")


EMULATE_SUPPORTED_TOOLS = frozenset(
    capability_id_to_wire_name(capability_id)
    for capability_id in TESTED_CAPABILITY_IDS
    - INTEGRATION_EVIDENCE_CAPABILITY_IDS
)
LIVE_ONLY_TOOLS = frozenset(
    capability_id_to_wire_name(capability_id)
    for capability_id in LIVE_ONLY_CAPABILITY_IDS
)
