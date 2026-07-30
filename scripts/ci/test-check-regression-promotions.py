from __future__ import annotations

import copy
import datetime as dt
import importlib.util
import json
import pathlib
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name("check-regression-promotions.py")
SPEC = importlib.util.spec_from_file_location("promotion_check", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class RegressionPromotionMetadataTest(unittest.TestCase):
    def setUp(self) -> None:
        source = (
            pathlib.Path(__file__).parents[2]
            / "tests/fixtures/llm_traces/reborn_qa/live_canary/case-manifest.json"
        )
        self.manifest = json.loads(source.read_text(encoding="utf-8"))

    def validate(self, manifest: dict[str, object]) -> list[str]:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            path = root / "a/b/c/d/e/case-manifest.json"
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps(manifest), encoding="utf-8")
            workflow_source = (
                pathlib.Path(__file__).parents[2] / ".github/workflows/live-canary.yml"
            )
            workflow_path = root / ".github/workflows/live-canary.yml"
            workflow_path.parent.mkdir(parents=True)
            workflow_path.write_text(
                workflow_source.read_text(encoding="utf-8"), encoding="utf-8"
            )
            lifecycle = manifest.get("promotion_metadata", {}).get("live_retirement", {})
            for entry in lifecycle.get("retired_cases", []):
                fixture = entry.get("deterministic_fixture")
                if fixture:
                    fixture_path = root / fixture
                    fixture_path.parent.mkdir(parents=True, exist_ok=True)
                    fixture_path.touch()
            return MODULE.validate(path, dt.date(2026, 7, 30))

    def test_committed_metadata_is_complete_and_fresh(self) -> None:
        self.assertEqual(self.validate(self.manifest), [])

    def test_missing_provenance_fails_loudly(self) -> None:
        broken = copy.deepcopy(self.manifest)
        del broken["promotion_metadata"]["provenance"]
        self.assertIn("missing provenance", self.validate(broken))

    def test_stale_replay_fails_loudly(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["promotion_metadata"]["last_successful_replay"]["date"] = "2020-01-01"
        self.assertTrue(
            any("stale" in error for error in self.validate(broken)),
            self.validate(broken),
        )

    def test_retirement_requires_deterministic_evidence(self) -> None:
        broken = copy.deepcopy(self.manifest)
        del broken["promotion_metadata"]["live_retirement"]["retirement_evidence"]
        errors = self.validate(broken)
        self.assertIn("missing retirement_evidence", errors)
        self.assertTrue(any("deterministic_test" in error for error in errors), errors)

    def test_every_replayable_case_is_live_or_retired_with_evidence(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["promotion_metadata"]["live_retirement"]["retired_cases"] = []
        errors = self.validate(broken)
        self.assertTrue(
            any(
                "replayable cases must be representative drift or retired" in error
                for error in errors
            ),
            errors,
        )


if __name__ == "__main__":
    unittest.main()
