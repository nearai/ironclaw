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
    for waiver in INVENTORY["waivers"]
    for capability in waiver["capabilities"]
)
ALL_CLASSIFIED_CAPABILITY_IDS = (
    TESTED_CAPABILITY_IDS
    | LIVE_ONLY_CAPABILITY_IDS
    | UNSUPPORTED_CAPABILITY_IDS
    | WAIVED_CAPABILITY_IDS
)


def _production_extension_ids() -> set[str]:
    extension_ids = set()
    for manifest_path in sorted(ASSET_ROOT.glob("*/manifest.toml")):
        with manifest_path.open("rb") as manifest_file:
            manifest = tomllib.load(manifest_file)
        if manifest.get("tools"):
            extension_ids.add(manifest["id"])
    return extension_ids


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
)
LIVE_ONLY_TOOLS = frozenset(
    capability_id_to_wire_name(capability_id)
    for capability_id in LIVE_ONLY_CAPABILITY_IDS
)
