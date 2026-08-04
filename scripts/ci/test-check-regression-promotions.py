from __future__ import annotations

import copy
import datetime as dt
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock

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

    def validate(
        self,
        manifest: dict[str, object],
        today: dt.date = dt.date(2026, 7, 30),
        scheduled_cases: str = "all",
        removed_matrix_case: str | None = None,
    ) -> list[str]:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            path = root / "arbitrary/depth/case-manifest.json"
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps(manifest), encoding="utf-8")
            workflow_source = (
                pathlib.Path(__file__).parents[2] / ".github/workflows/live-canary.yml"
            )
            workflow_path = root / ".github/workflows/live-canary.yml"
            workflow_path.parent.mkdir(parents=True)
            workflow = workflow_source.read_text(encoding="utf-8")
            workflow = workflow.replace(
                "github.event_name == 'schedule' && 'all'",
                f"github.event_name == 'schedule' && '{scheduled_cases}'",
                1,
            )
            if removed_matrix_case is not None:
                lines = workflow.splitlines()
                for index, line in enumerate(lines):
                    if not line.startswith("            cases: "):
                        continue
                    matrix_cases = line.removeprefix("            cases: ").split(",")
                    if removed_matrix_case not in matrix_cases:
                        continue
                    matrix_cases.remove(removed_matrix_case)
                    lines[index] = "            cases: " + ",".join(matrix_cases)
                    break
                else:
                    self.fail(f"matrix case not found: {removed_matrix_case}")
                workflow = "\n".join(lines) + "\n"
            workflow_path.write_text(workflow, encoding="utf-8")
            lifecycle = manifest.get("promotion_metadata", {}).get("live_retirement", {})
            for entry in lifecycle.get("retired_cases", []):
                fixture = entry.get("deterministic_fixture")
                if fixture:
                    fixture_path = root / fixture
                    fixture_path.parent.mkdir(parents=True, exist_ok=True)
                    fixture_path.touch()
            return MODULE.validate(path, today, root)

    def test_committed_metadata_is_complete_and_fresh(self) -> None:
        self.assertEqual(self.validate(self.manifest, dt.date.today()), [])

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

    def test_boolean_max_replay_age_is_not_accepted_as_an_integer(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["promotion_metadata"]["max_replay_age_days"] = True

        self.assertIn(
            "max_replay_age_days must be a positive integer",
            self.validate(broken),
        )

    def test_retirement_requires_deterministic_evidence(self) -> None:
        broken = copy.deepcopy(self.manifest)
        del broken["promotion_metadata"]["live_retirement"]["retirement_evidence"]
        errors = self.validate(broken)
        self.assertIn("missing retirement_evidence", errors)
        self.assertTrue(any("deterministic_test" in error for error in errors), errors)

    def test_every_replayable_case_is_scheduled_or_retired_with_evidence(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["promotion_metadata"]["live_retirement"]["retired_cases"] = []
        errors = self.validate(
            broken,
            scheduled_cases=(
                "qa_3b_endpoint_status_live_chat,"
                "qa_9b_routine_dm_delivery_exactly_once,"
                "qa_10a_slack_self_attribution"
            ),
        )
        self.assertTrue(
            any(
                "replayable cases must be scheduled or retired" in error
                for error in errors
            ),
            errors,
        )

    def test_cases_without_active_replay_must_remain_scheduled(self) -> None:
        errors = self.validate(
            self.manifest,
            scheduled_cases=(
                "qa_3b_endpoint_status_live_chat,"
                "qa_9b_routine_dm_delivery_exactly_once,"
                "qa_10a_slack_self_attribution"
            ),
        )

        self.assertTrue(
            any(
                "cases without active deterministic replay must remain scheduled"
                in error
                for error in errors
            ),
            errors,
        )

    def test_all_selector_fails_when_matrix_omits_harvested_case(self) -> None:
        removed = "qa_10i_slack_raw_entity_hygiene"

        errors = self.validate(self.manifest, removed_matrix_case=removed)

        self.assertIn(
            f"harvested case is missing from live-canary matrix: {removed}",
            errors,
        )

    def test_invalid_minimum_reports_errors_instead_of_raising(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["promotion_metadata"]["live_retirement"][
            "minimum_representative_drift_cases"
        ] = "not-an-integer"

        errors = self.validate(broken)

        self.assertIn(
            "minimum_representative_drift_cases must be positive",
            errors,
        )

    def test_boolean_minimum_is_not_accepted_as_an_integer(self) -> None:
        broken = copy.deepcopy(self.manifest)
        broken["promotion_metadata"]["live_retirement"][
            "minimum_representative_drift_cases"
        ] = True

        self.assertIn(
            "minimum_representative_drift_cases must be positive",
            self.validate(broken),
        )

    def test_default_date_is_evaluated_for_each_invocation(self) -> None:
        dates = [dt.date(2026, 7, 30), dt.date(2026, 7, 31)]
        with mock.patch.object(MODULE.dt, "date") as date_type:
            date_type.today.side_effect = dates

            self.assertEqual(MODULE.current_date(None), dates[0])
            self.assertEqual(MODULE.current_date(None), dates[1])


if __name__ == "__main__":
    unittest.main()
