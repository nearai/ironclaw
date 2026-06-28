#!/usr/bin/env python3
"""Unit tests for notify_slack.py helpers.

Focus is on `parse_summary_status` — the `summary.md` → exit-code
fallback that classifies lane status when neither JUnit XML nor
``results.json`` is present (summary-only lanes like private-oauth,
or any lane whose ``results.json`` got stripped by strict scrub before
upload). This path is part of the status-classification surface, so
parser drift would silently mislabel lanes.

Run with::

    python3 -m pytest scripts/live-canary/test_notify_slack.py -v

Or directly::

    python3 scripts/live-canary/test_notify_slack.py
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


# Mirror test_emit_results_json.py's loader so this file also runs
# standalone without a package layout. notify_slack.py uses
# ``@dataclass``, which introspects ``sys.modules`` for the owning
# module, so we have to register the module before executing it —
# otherwise dataclass decoration raises an AttributeError on import.
_SPEC = importlib.util.spec_from_file_location(
    "notify_slack",
    Path(__file__).parent / "notify_slack.py",
)
notify = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = notify
_SPEC.loader.exec_module(notify)


# Canonical summary.md produced by scripts/live-canary/run.sh's
# `write_summary` helper. The status row is the single field this
# parser cares about — everything else is decoration that must not
# trigger the regex.
_SUMMARY_TEMPLATE = """\
## Live Canary Summary

| Field | Value |
| --- | --- |
| Lane | `private-oauth` |
| Scenario | `<default>` |
| Provider | `dedicated-runner` |
| Status | `{status}` |
| Started | `2026-05-17T12:00:00Z` |
| Finished | `2026-05-17T12:42:13Z` |
| Commit | `abcdef0123456789` |

Artifacts:
- `test-output.log`
- `env-summary.txt`
- `trace-fixture-status.txt`
"""


class ParseSummaryStatusTests(unittest.TestCase):
    def test_zero_status_means_pass(self):
        self.assertEqual(
            notify.parse_summary_status(_SUMMARY_TEMPLATE.format(status="0")),
            0,
        )

    def test_nonzero_status_means_fail(self):
        self.assertEqual(
            notify.parse_summary_status(_SUMMARY_TEMPLATE.format(status="1")),
            1,
        )

    def test_negative_status_is_preserved(self):
        # `run.sh` shouldn't write negatives in practice, but the regex
        # allows them and `collect_lane` treats any non-zero as fail —
        # confirm the integer flows through unmodified.
        self.assertEqual(
            notify.parse_summary_status(_SUMMARY_TEMPLATE.format(status="-1")),
            -1,
        )

    def test_large_status_is_preserved(self):
        # Bash exit codes wrap at 256, but the regex is unbounded;
        # ensure no accidental truncation/clamping by the parser.
        self.assertEqual(
            notify.parse_summary_status(_SUMMARY_TEMPLATE.format(status="137")),
            137,
        )

    def test_missing_status_row_returns_none(self):
        # Workflow-canary summary.md (different writer) doesn't carry a
        # `| Status | \`N\` |` row — caller falls through to log-tail
        # heuristic. Must return None, not raise.
        no_status = (
            "## Live Canary Summary\n\n"
            "| Field | Value |\n"
            "| --- | --- |\n"
            "| Lane | `auth-canary` |\n"
        )
        self.assertIsNone(notify.parse_summary_status(no_status))

    def test_empty_string_returns_none(self):
        # `read_tail` returns "" when summary.md is missing entirely.
        self.assertIsNone(notify.parse_summary_status(""))

    def test_malformed_status_value_returns_none(self):
        # If the writer ever emits a non-integer literal in the status
        # cell, the parser must degrade to None rather than crash so
        # the lane still surfaces (as "unknown") in Slack.
        malformed = _SUMMARY_TEMPLATE.replace("`{status}`", "`oops`").format()
        self.assertIsNone(notify.parse_summary_status(malformed))

    def test_status_row_not_at_line_start_is_ignored(self):
        # The regex is anchored with `^...$` under MULTILINE. A row
        # appearing inline (e.g. quoted inside a prose paragraph) must
        # not be picked up — that would let a literal block-quoted
        # summary in a comment flip the lane status.
        inline = (
            "Some prose mentioning `| Status | `9` |` inline "
            "but not as a real table row."
        )
        self.assertIsNone(notify.parse_summary_status(inline))

    def test_status_row_with_extra_whitespace(self):
        # `write_summary` uses single-space padding, but accept the
        # common variations (no-pad, double-pad) so a future cosmetic
        # change to the writer doesn't break classification silently.
        for variant in (
            "|Status|`0`|",
            "|  Status  |  `0`  |",
            "| Status |\t`0`\t|",
        ):
            with self.subTest(variant=variant):
                doc = "## summary\n\n" + variant + "\n"
                # All variants should resolve to the same exit code.
                # If the regex is too strict to match a variant, the
                # test fails closed (we'd rather know now than discover
                # in prod that a writer tweak silently broke parsing).
                got = notify.parse_summary_status(doc)
                self.assertEqual(got, 0, f"variant not parsed: {variant!r}")


class RebornQaSlackReportTests(unittest.TestCase):
    def test_collect_lane_populates_per_case_reports(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            lane_dir = Path(tmpdir) / "reborn-webui-v2-live-qa" / "reborn-webui-v2" / "20260628T000000Z"
            lane_dir.mkdir(parents=True)
            (lane_dir / "results.json").write_text(
                json.dumps(
                    {
                        "results": [
                            {
                                "provider": "reborn-webui-v2",
                                "mode": "live:qa_2a_gmail_connect",
                                "success": True,
                                "latency_ms": 1200,
                                "details": {
                                    "case": "qa_2a_gmail_connect",
                                    "gate": "requires live Google browser consent state",
                                },
                            },
                            {
                                "provider": "reborn-webui-v2",
                                "mode": "live:qa_2d_calendar_prep_live_chat",
                                "success": False,
                                "latency_ms": 0,
                                "details": {
                                    "case": "qa_2d_calendar_prep_live_chat",
                                    "blocked": "missing_google_ready",
                                    "gate": "requires live Google runtime access",
                                },
                            },
                        ]
                    }
                ),
                encoding="utf-8",
            )
            (lane_dir / "case-manifest.json").write_text(
                json.dumps(
                    {
                        "cases": [
                            {
                                "case": "qa_2a_gmail_connect",
                                "qa_rows": ["2A"],
                                "feature": "Gmail connection flow",
                            },
                            {
                                "case": "qa_2d_calendar_prep_live_chat",
                                "qa_rows": ["2D"],
                                "feature": "Calendar prep assistant using Google Docs and live news",
                            },
                        ]
                    }
                ),
                encoding="utf-8",
            )

            report = notify.collect_lane(lane_dir)

        self.assertIsNotNone(report)
        self.assertEqual(report.tests, 2)
        self.assertEqual(report.passed, 1)
        self.assertEqual(report.failed, 1)
        self.assertEqual(len(report.reborn_qa_cases), 2)
        self.assertEqual(report.reborn_qa_cases[0].rows, ("2A",))
        self.assertEqual(report.reborn_qa_cases[0].feature, "Gmail connection flow")
        self.assertEqual(report.reborn_qa_cases[0].message, "")
        self.assertEqual(report.reborn_qa_cases[1].rows, ("2D",))
        self.assertEqual(
            report.reborn_qa_cases[1].message,
            "requires live Google runtime access",
        )

    def test_slack_payload_renders_each_reborn_qa_row(self):
        report = notify.LaneReport(
            lane="reborn-webui-v2-live-qa",
            provider="reborn-webui-v2",
            passed=1,
            failed=1,
            tests=2,
            duration_s=1.2,
            status="fail",
            reborn_qa_cases=[
                notify.RebornQaCaseReport(
                    rows=("2A",),
                    case="qa_2a_gmail_connect",
                    feature="Gmail connection flow",
                    success=True,
                    latency_ms=1200,
                ),
                notify.RebornQaCaseReport(
                    rows=("2D",),
                    case="qa_2d_calendar_prep_live_chat",
                    feature="Calendar prep assistant using Google Docs and live news",
                    success=False,
                    latency_ms=0,
                    message="requires live Google runtime access",
                ),
            ],
        )

        payload = notify.slack_payload([report], None, "abcdef0123456789")
        section_texts = [
            block["text"]["text"]
            for block in payload["blocks"]
            if block.get("type") == "section"
        ]

        self.assertTrue(any("*QA 2A: Gmail connection flow*" in text for text in section_texts))
        self.assertTrue(
            any(
                "*QA 2D: Calendar prep assistant using Google Docs and live news*" in text
                for text in section_texts
            )
        )
        self.assertTrue(
            any("requires live Google runtime access" in text for text in section_texts)
        )

    def test_reborn_rows_fit_with_scheduled_all_lane_report(self):
        reports = [
            notify.LaneReport(
                lane=f"lane-{idx}",
                provider="default",
                passed=1,
                failed=0,
                tests=1,
                status="pass",
            )
            for idx in range(14)
        ]
        reports.append(
            notify.LaneReport(
                lane="reborn-webui-v2-live-qa",
                provider="reborn-webui-v2",
                passed=33,
                failed=0,
                tests=33,
                status="pass",
                reborn_qa_cases=[
                    notify.RebornQaCaseReport(
                        rows=(f"{idx}",),
                        case=f"qa_case_{idx}",
                        feature=f"Feature {idx}",
                        success=True,
                    )
                    for idx in range(1, 34)
                ],
            )
        )

        payload = notify.slack_payload(
            reports,
            "https://github.com/nearai/ironclaw/actions/runs/1",
            "abcdef0123456789",
        )

        self.assertLessEqual(len(payload["blocks"]), notify.SLACK_MAX_BLOCKS)
        section_texts = [
            block["text"]["text"]
            for block in payload["blocks"]
            if block.get("type") == "section"
        ]
        self.assertTrue(any("*QA 1: Feature 1*" in text for text in section_texts))
        self.assertTrue(any("*QA 33: Feature 33*" in text for text in section_texts))


if __name__ == "__main__":
    unittest.main()
