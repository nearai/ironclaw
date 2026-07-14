from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/ci/check-reborn-responses-e2e-manifest.py"
SPEC = importlib.util.spec_from_file_location(
    "check_reborn_responses_e2e_manifest", SCRIPT
)
assert SPEC is not None and SPEC.loader is not None
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class CheckRebornResponsesE2EManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)
        self.manifest = self.root / "manifest.txt"
        self.primary = self.root / "test_primary.py"
        self.legacy = self.root / "test_legacy.py"
        self.primary.write_text(
            "def test_reborn_responses_primary():\n    pass\n",
            encoding="utf-8",
        )
        self.legacy.write_text(
            "async def test_reborn_legacy():\n    pass\n",
            encoding="utf-8",
        )
        self.expected = [
            f"{self.primary.name}::test_reborn_responses_primary",
            f"{self.legacy.name}::test_reborn_legacy",
        ]
        self.paths = mock.patch.multiple(
            CHECKER,
            ROOT=self.root,
            MANIFEST=self.manifest,
            PRIMARY=self.primary,
            LEGACY_PORT=self.legacy,
        )
        self.paths.start()

    def tearDown(self) -> None:
        self.paths.stop()
        self.temp_dir.cleanup()

    def write_manifest(self, selectors: list[str]) -> None:
        self.manifest.write_text("\n".join(selectors) + "\n", encoding="utf-8")

    def test_rejects_duplicate_selector(self) -> None:
        duplicate = self.expected[0]
        self.write_manifest([*self.expected, duplicate])

        with self.assertRaisesRegex(SystemExit, rf"(?s)duplicate node ids:.*{duplicate}"):
            CHECKER.main()

    def test_rejects_missing_selector(self) -> None:
        missing = self.expected[1]
        self.write_manifest([self.expected[0]])

        with self.assertRaisesRegex(
            SystemExit, rf"(?s)missing required Responses tests:.*{missing}"
        ):
            CHECKER.main()

    def test_rejects_unexpected_selector(self) -> None:
        unexpected = f"{self.primary.name}::test_reborn_responses_unexpected"
        self.write_manifest([*self.expected, unexpected])

        with self.assertRaisesRegex(
            SystemExit, rf"(?s)unexpected Responses test selectors:.*{unexpected}"
        ):
            CHECKER.main()


if __name__ == "__main__":
    unittest.main()
