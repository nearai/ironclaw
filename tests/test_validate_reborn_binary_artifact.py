from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/live-canary/validate_reborn_binary_artifact.py"
SPEC = importlib.util.spec_from_file_location("validate_reborn_binary_artifact", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class ValidateRebornBinaryArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.artifact_dir = Path(self.temp_dir.name)
        self.archive = self.artifact_dir / VALIDATOR.ARCHIVE_NAME
        self.archive.write_bytes(b"tested reborn binary")
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        (self.artifact_dir / VALIDATOR.CHECKSUM_NAME).write_text(
            f"{digest}  {VALIDATOR.ARCHIVE_NAME}\n",
            encoding="utf-8",
        )
        self.write_manifest([])

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def write_manifest(
        self, features: object, *, product_ref: str | None = None
    ) -> None:
        (self.artifact_dir / VALIDATOR.MANIFEST_NAME).write_text(
            json.dumps(
                {
                    "format_version": 1,
                    "product_ref": product_ref or "a" * 40,
                    "features": features,
                }
            ),
            encoding="utf-8",
        )

    def test_accepts_matching_artifact(self) -> None:
        VALIDATOR.validate_artifact(
            self.artifact_dir,
            "a" * 40,
            "",
        )

    def test_accepts_array_feature_superset(self) -> None:
        self.write_manifest(
            [
                "extra-feature",
                "replay",
            ]
        )

        VALIDATOR.validate_artifact(
            self.artifact_dir,
            "a" * 40,
            "replay",
        )

    def test_rejects_missing_required_feature(self) -> None:
        self.write_manifest(["other"])

        with self.assertRaisesRegex(ValueError, "replay"):
            VALIDATOR.validate_artifact(
                self.artifact_dir,
                "a" * 40,
                "replay",
            )

    def test_rejects_invalid_feature_shape(self) -> None:
        self.write_manifest(["replay", 42])

        with self.assertRaisesRegex(
            ValueError, "comma-separated string or string array"
        ):
            VALIDATOR.validate_artifact(
                self.artifact_dir,
                "a" * 40,
                "replay",
            )

    def test_rejects_empty_feature_entries(self) -> None:
        for features in ("replay,", ["replay", ""]):
            with self.subTest(features=features):
                self.write_manifest(features)

                with self.assertRaisesRegex(ValueError, "empty feature"):
                    VALIDATOR.validate_artifact(
                        self.artifact_dir,
                        "a" * 40,
                        "replay",
                    )

    def test_rejects_duplicate_features(self) -> None:
        for features in (
            "replay,replay",
            ["replay", "replay"],
        ):
            with self.subTest(features=features):
                self.write_manifest(features)

                with self.assertRaisesRegex(ValueError, "duplicate features"):
                    VALIDATOR.validate_artifact(
                    self.artifact_dir,
                    "a" * 40,
                    "replay",
                )

    def test_rejects_corrupt_archive(self) -> None:
        self.archive.write_bytes(b"corrupt")

        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            VALIDATOR.validate_artifact(
            self.artifact_dir,
            "a" * 40,
            "",
        )

    def test_rejects_mismatched_manifest(self) -> None:
        self.write_manifest(
            [],
            product_ref="b" * 40,
        )

        with self.assertRaisesRegex(ValueError, "product_ref"):
            VALIDATOR.validate_artifact(
                self.artifact_dir,
                "a" * 40,
                "",
            )


if __name__ == "__main__":
    unittest.main()
