"""Executable classification of the shipped provider capability surface."""

from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[2]
INVENTORY_PATH = ROOT / "tests/e2e/fixtures/provider_capability_coverage.toml"
ASSET_ROOT = ROOT / "crates/ironclaw_first_party_extensions/assets"


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
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        if manifest.get("tools"):
            extension_ids.add(manifest["id"])
    return extension_ids


def _capability_operation_kinds() -> dict[str, str]:
    """Read/write kind per capability, derived from shipped manifests.

    The manifests already declare `external_write` in each tool's `effects`,
    so the kind is production-derived like the capability denominator itself
    rather than a hand-maintained list that can drift from what ships.
    """
    kinds: dict[str, str] = {}
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        for tool in manifest.get("tools", []):
            effects = tool.get("effects", [])
            kinds[tool["id"]] = "write" if "external_write" in effects else "read"
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
