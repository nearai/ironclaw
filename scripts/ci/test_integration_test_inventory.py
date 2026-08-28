"""Contract tests for the canonical integration-test inventory."""

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/ci/lib"))

import integration_test_inventory as inventory  # noqa: E402


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
                inventory.cargo_test_names(root),
                ["reborn_group_shared", "reborn_integration_after", "reborn_integration_before"],
            )
            self.assertEqual(
                inventory.planner_test_lanes(root),
                {
                    "tests/integration/duplicate.rs": 1,
                    "tests/integration/group_shared/main.rs": "groups",
                    "tests/integration/second.rs": 1,
                },
            )

            (root / "Cargo.toml").write_text(
                'test = [{ name = "new_test", path = "tests/integration/new_test.rs" }]\n',
                encoding="utf-8",
            )
            self.assertEqual(inventory.cargo_test_names(root), ["new_test"])
            for projection in (inventory.planner_test_lanes, inventory.inventory_document):
                with self.subTest(projection=projection.__name__):
                    with self.assertRaisesRegex(ValueError, "unsupported.*new_test"):
                        projection(root)

    def test_document_is_versioned_and_self_validating(self) -> None:
        document = inventory.inventory_document(ROOT)

        self.assertEqual(inventory.INTEGRATION_PARTITION_COUNT, 4)
        self.assertEqual(document["schema_version"], 1)
        self.assertEqual(
            document["partition_count"], inventory.INTEGRATION_PARTITION_COUNT
        )
        self.assertEqual(inventory.validate_inventory_document(document), document)

        invalid_fields = (
            ("schema_version", True),
            ("schema_version", 1.0),
            ("partition_count", 0),
            ("partition_count", 4.0),
        )
        for field, value in invalid_fields:
            malformed = dict(document)
            malformed[field] = value
            with self.subTest(field=field, value=value):
                with self.assertRaisesRegex(ValueError, field):
                    inventory.validate_inventory_document(malformed)


if __name__ == "__main__":
    unittest.main()
