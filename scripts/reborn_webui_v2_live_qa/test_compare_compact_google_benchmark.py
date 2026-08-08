#!/usr/bin/env python3

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.reborn_webui_v2_live_qa.compare_compact_google_benchmark import (
    _arm_for_path,
    build_report,
    load_results,
)


def result(*, calls: int, compact: int, input_tokens: int, latency_ms: int) -> dict:
    return {
        "success": True,
        "latency_ms": latency_ms,
        "details": {
            "run_metrics": {
                "google_tool_call_count": calls,
                "compact_google_tool_call_count": compact,
                "discovery_tool_call_count": 1,
                "usage": {"input_tokens": input_tokens, "output_tokens": 10},
            }
        },
    }


class CompactGoogleBenchmarkReportTests(unittest.TestCase):
    def test_arm_for_path_rejects_missing_and_ambiguous_markers(self):
        self.assertEqual(
            _arm_for_path(Path("qa-context-compact-disabled/results.json")),
            "disabled",
        )
        self.assertIsNone(_arm_for_path(Path("qa/results.json")))
        self.assertIsNone(
            _arm_for_path(
                Path(
                    "qa-context-compact-disabled/qa-context-compact-enabled/results.json"
                )
            )
        )

    def test_load_results_classifies_valid_benchmark_cases_only(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            disabled = root / "qa-context-compact-disabled"
            enabled = root / "qa-context-compact-enabled"
            malformed = root / "other" / "qa-context-compact-enabled"
            disabled.mkdir(parents=True)
            enabled.mkdir(parents=True)
            malformed.mkdir(parents=True)
            (disabled / "results.json").write_text(
                '{"results": ['
                '{"success": true, "details": {"case": "benchmark_google_email_digest"}},'
                '{"success": true, "details": {"case": "qa_unrelated"}},'
                'null]}'
            )
            (enabled / "results.json").write_text(
                '{"results": [{"success": true, "details": '
                '{"case": "benchmark_google_email_digest"}}]}'
            )
            (malformed / "results.json").write_text("not-json")

            arms = load_results(root)

        self.assertEqual(set(arms["disabled"]), {"benchmark_google_email_digest"})
        self.assertEqual(set(arms["enabled"]), {"benchmark_google_email_digest"})
        self.assertTrue(arms["enabled"]["benchmark_google_email_digest"]["success"])

    def test_load_results_skips_non_object_payloads(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            enabled = root / "qa-context-compact-enabled"
            enabled.mkdir(parents=True)
            (enabled / "results.json").write_text("null")

            arms = load_results(root)

        self.assertEqual(arms, {"disabled": {}, "enabled": {}})

    def test_malformed_metrics_are_normalized_without_becoming_successful(self):
        arms = {
            "disabled": {
                "benchmark_google_email_digest": {
                    "success": "false",
                    "latency_ms": "not-a-number",
                    "details": {"run_metrics": {"google_tool_call_count": "four"}},
                }
            },
            "enabled": {},
        }

        report, _ = build_report(arms)

        self.assertEqual(report["comparable_cases"], 0)
        self.assertFalse(report["rows"][0]["disabled_success"])
        self.assertEqual(report["rows"][0]["disabled_google_calls"], 0)
        self.assertEqual(report["rows"][0]["latency_delta_ms"], 0)

    def test_non_finite_metrics_are_normalized(self):
        result_with_non_finite_metrics = result(
            calls=1,
            compact=0,
            input_tokens=10,
            latency_ms=10,
        )
        result_with_non_finite_metrics["latency_ms"] = float("nan")
        result_with_non_finite_metrics["details"]["run_metrics"][
            "google_tool_call_count"
        ] = float("inf")
        arms = {
            "disabled": {"benchmark_google_email_digest": result_with_non_finite_metrics},
            "enabled": {},
        }

        report, _ = build_report(arms)

        self.assertEqual(report["rows"][0]["disabled_google_calls"], 0)
        self.assertEqual(report["rows"][0]["latency_delta_ms"], 0)

    def test_reports_call_token_and_latency_deltas(self):
        names = [
            "email_digest",
            "daily_brief",
            "meeting_prep",
            "document_lookup",
            "sheet_preview",
        ]
        arms = {
            "disabled": {
                f"benchmark_google_{name}": result(
                    calls=4, compact=0, input_tokens=1000, latency_ms=2000
                )
                for name in names
            },
            "enabled": {
                f"benchmark_google_{name}": result(
                    calls=1, compact=1, input_tokens=700, latency_ms=1200
                )
                for name in names
            },
        }

        report, markdown = build_report(arms)

        self.assertEqual(report["verdict"], "VERIFIED")
        self.assertEqual(report["google_calls_saved"], 15)
        self.assertEqual(report["google_calls_saved_percent"], 75.0)
        self.assertEqual(report["rows"][0]["input_token_delta"], -300)
        self.assertEqual(report["rows"][0]["latency_delta_ms"], -800)
        self.assertIn("Google calls 20 -> 5", markdown)


if __name__ == "__main__":
    unittest.main()
