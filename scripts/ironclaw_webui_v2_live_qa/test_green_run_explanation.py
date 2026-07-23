#!/usr/bin/env python3
"""Unit tests for IronClaw WebUI v2 live QA green-run explanations."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary.common import ProbeResult  # noqa: E402
from scripts.ironclaw_webui_v2_live_qa.green_run_explanation import (  # noqa: E402
    write_green_run_explanation,
)


class GreenRunExplanationTests(unittest.TestCase):
    def test_write_green_run_explanation_records_success_reasons(self):
        results = [
            ProbeResult(
                provider="test",
                mode="live:literal_case",
                success=True,
                latency_ms=1,
                details={
                    "case": "literal_case",
                    "required_text": [None, "", "routine"],
                    "text_excerpt": "Truncated excerpt without the matched word.",
                    "semantic_judge_used": False,
                    "semantic_judge_reason": "literal_required_text_matched",
                },
            ),
            ProbeResult(
                provider="test",
                mode="live:semantic_case",
                success=True,
                latency_ms=1,
                details={
                    "case": "semantic_case",
                    "required_text": ["routine"],
                    "text_excerpt": "Schedule: every hour. Trigger ID: trigger-123.",
                    "semantic_judge_reason": "semantic_judge_completed",
                    "semantic_judge": {
                        "completed": True,
                        "confidence": 0.91,
                        "reason": "The response confirms the scheduled task.",
                        "evidence": ["Trigger ID: trigger-123"],
                    },
                },
            ),
            ProbeResult(
                provider="test",
                mode="live:side_effect_case",
                success=True,
                latency_ms=1,
                details={
                    "case": "side_effect_case",
                    "delivery_verified": True,
                },
            ),
        ]

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            path = write_green_run_explanation(output_dir, results)
            payload = json.loads(path.read_text(encoding="utf-8"))

        self.assertEqual(payload["total_cases"], 3)
        self.assertEqual(payload["successful_cases"], 3)
        self.assertEqual(payload["failed_cases"], 0)
        self.assertEqual(payload["successful_cases_matching_required_text_literally"], 1)
        self.assertEqual(payload["successful_cases_using_semantic_judge"], 1)
        self.assertIn("All 3 cases were green", payload["why_things_were_green"])
        cases_by_name = {case["case"]: case for case in payload["cases"]}
        self.assertEqual(cases_by_name["literal_case"]["required_text"], ["routine"])
        self.assertTrue(cases_by_name["literal_case"]["literal_required_text_matched"])
        self.assertTrue(cases_by_name["semantic_case"]["semantic_judge_used"])
        self.assertEqual(
            cases_by_name["literal_case"]["success_reasons"],
            ["literal_required_text_matched"],
        )
        self.assertEqual(
            cases_by_name["semantic_case"]["success_reasons"],
            ["semantic_judge_completed"],
        )
        self.assertEqual(
            cases_by_name["side_effect_case"]["success_reasons"],
            ["case_success_from_non_text_assertions"],
        )
        self.assertEqual(
            cases_by_name["semantic_case"]["semantic_judge_summary"],
            {
                "completed": True,
                "confidence": 0.91,
                "reason": "The response confirms the scheduled task.",
                "enabled": None,
            },
        )

    def test_success_with_required_text_but_no_gate_is_unclassified(self):
        result = ProbeResult(
            provider="test",
            mode="live:unexpected_case",
            success=True,
            latency_ms=1,
            details={
                "case": "unexpected_case",
                "required_text": ["routine"],
                "text_excerpt": "No matching text here.",
            },
        )

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            path = write_green_run_explanation(output_dir, [result])
            payload = json.loads(path.read_text(encoding="utf-8"))

        self.assertEqual(
            payload["cases"][0]["success_reasons"],
            ["case_success_reason_unclassified"],
        )

    def test_failed_case_updates_summary_and_message(self):
        results = [
            ProbeResult(
                provider="test",
                mode="live:passed_case",
                success=True,
                latency_ms=1,
                details={
                    "case": "passed_case",
                    "required_text": ["routine"],
                    "text_excerpt": "Routine created.",
                },
            ),
            ProbeResult(
                provider="test",
                mode="live:failed_case",
                success=False,
                latency_ms=1,
                details={
                    "case": "failed_case",
                    "error": "setup failed",
                },
            ),
        ]

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            path = write_green_run_explanation(output_dir, results)
            payload = json.loads(path.read_text(encoding="utf-8"))

        self.assertEqual(payload["total_cases"], 2)
        self.assertEqual(payload["successful_cases"], 1)
        self.assertEqual(payload["failed_cases"], 1)
        self.assertIn(
            "1 of 2 cases were green; 1 failed.",
            payload["why_things_were_green"],
        )
        cases_by_name = {case["case"]: case for case in payload["cases"]}
        self.assertEqual(cases_by_name["failed_case"]["success_reasons"], [])


if __name__ == "__main__":
    unittest.main()
