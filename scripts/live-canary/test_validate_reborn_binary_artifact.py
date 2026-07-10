from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("validate_reborn_binary_artifact.py")
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
        (self.artifact_dir / VALIDATOR.MANIFEST_NAME).write_text(
            json.dumps(
                {
                    "format_version": 1,
                    "product_ref": "a" * 40,
                    "features": "webui-v2-beta,slack-v2-host-beta",
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def test_accepts_matching_artifact(self) -> None:
        VALIDATOR.validate_artifact(
            self.artifact_dir,
            "a" * 40,
            "webui-v2-beta,slack-v2-host-beta",
        )

    def test_rejects_corrupt_archive(self) -> None:
        self.archive.write_bytes(b"corrupt")

        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            VALIDATOR.validate_artifact(
                self.artifact_dir,
                "a" * 40,
                "webui-v2-beta,slack-v2-host-beta",
            )

    def test_rejects_mismatched_manifest(self) -> None:
        with self.assertRaisesRegex(ValueError, "product_ref"):
            VALIDATOR.validate_artifact(
                self.artifact_dir,
                "b" * 40,
                "webui-v2-beta,slack-v2-host-beta",
            )


if __name__ == "__main__":
    unittest.main()
