#!/usr/bin/env python3
"""Contract tests for the canonical integration-test inventory."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/ci/lib"))

from integration_test_inventory import (  # noqa: E402
    INTEGRATION_PARTITION_COUNT,
    cargo_test_names,
    inventory_document,
    planner_test_lanes,
    validate_inventory_document,
)


class IntegrationTestInventoryTests(unittest.TestCase):
    def test_preserves_current_registration_projections(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text(
                """
test = [
  { name = "reborn_integration_before", path = "tests/integration/duplicate.rs" },
  { name = 7, path = "tests/integration/ignored_name.rs" },
  { name = "ignored_path", path = 7 },
  { name = "outside_scope", path = "tests/other.rs" },
  { name = "reborn_group_shared", path = "tests/integration/group_shared/main.rs" },
  { name = "reborn_integration_after", path = "tests/integration/duplicate.rs" },
  { name = "reborn_integration_after", path = "tests/integration/second.rs" },
]
""",
                encoding="utf-8",
            )

            self.assertEqual(
                cargo_test_names(root),
                [
                    "reborn_group_shared",
                    "reborn_integration_after",
                    "reborn_integration_before",
                ],
            )
            self.assertEqual(
                planner_test_lanes(root),
                {
                    "tests/integration/duplicate.rs": 1,
                    "tests/integration/group_shared/main.rs": "groups",
                    "tests/integration/second.rs": 1,
                },
            )

    def test_document_is_versioned_and_self_validating(self) -> None:
        document = inventory_document(ROOT)

        self.assertEqual(INTEGRATION_PARTITION_COUNT, 4)
        self.assertEqual(document["schema_version"], 1)
        self.assertEqual(
            document["partition_count"], INTEGRATION_PARTITION_COUNT
        )
        self.assertEqual(validate_inventory_document(document), document)

        malformed = dict(document)
        malformed["partition_count"] = 0
        with self.assertRaisesRegex(ValueError, "partition_count"):
            validate_inventory_document(malformed)


if __name__ == "__main__":
    unittest.main()
