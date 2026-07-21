#!/usr/bin/env python3

import unittest

from scripts.reborn_webui_v2_live_qa.compare_compact_google_benchmark import build_report


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
