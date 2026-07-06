#!/usr/bin/env python3
"""Unit tests for notify_slack.py helpers.

Focus is on `parse_summary_status` — the `summary.md` → exit-code
fallback that classifies lane status when neither JUnit XML nor
``results.json`` is present (summary-only lanes like private-oauth,
or any lane whose detailed results are unavailable after artifact scrub).
This path is part of the status-classification surface, so
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
from unittest import mock
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


def _trace_json(tool_calls: list[dict]) -> str:
    signatures = [
        {
            "name": call["name"],
            "args_hash": call.get("args_hash", ""),
        }
        for call in tool_calls
    ]
    outputs = [
        {
            "signature": {
                "name": call["name"],
                "args_hash": call.get("args_hash", ""),
            },
            "output_digest": call.get("output_digest", ""),
        }
        for call in tool_calls
        if call.get("output_digest")
    ]
    payload = {
        "recent_call_signatures": {"items": signatures},
        "seen_capability_output_digests": {"items": outputs},
    }
    return json.dumps(
        {
            "entries": [
                {
                    "contents": {
                        "payload_hex": json.dumps(payload).encode("utf-8").hex()
                    }
                }
            ]
        }
    )


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
            traces_dir = lane_dir / "traces"
            traces_dir.mkdir()
            (traces_dir / "qa_2a_gmail_connect.json").write_text(
                _trace_json(
                    [
                        {
                            "name": "gmail.list_messages",
                            "args_hash": "1234567890123",
                            "output_digest": "9876543210987",
                        }
                    ]
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
        self.assertEqual(len(report.reborn_qa_cases[0].tool_calls), 1)
        self.assertEqual(report.reborn_qa_cases[0].tool_calls[0].name, "gmail.list_messages")
        self.assertEqual(report.reborn_qa_cases[0].tool_calls[0].args_hash, "1234567890123")
        self.assertEqual(report.reborn_qa_cases[0].tool_calls[0].output_digest, "9876543210987")
        self.assertEqual(report.reborn_qa_cases[1].rows, ("2D",))
        self.assertEqual(
            report.reborn_qa_cases[1].message,
            "requires live Google runtime access",
        )
        self.assertEqual(
            report.reborn_qa_cases[1].debug_paths,
            [
                "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/results.json",
                "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/test-output.log",
                "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/traces/qa_2d_calendar_prep_live_chat.json",
                "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/traces/index.json",
            ],
        )
        self.assertEqual(report.reborn_qa_cases[0].debug_paths, [])

    def test_slack_payload_renders_each_reborn_qa_row(self):
        report = notify.LaneReport(
            lane="reborn-webui-v2-live-qa",
            provider="reborn-webui-v2",
            passed=1,
            failed=2,
            tests=3,
            duration_s=1.2,
            status="fail",
            reborn_qa_cases=[
                notify.RebornQaCaseReport(
                    rows=("2A",),
                    case="qa_2a_gmail_connect",
                    feature="Gmail connection flow",
                    success=True,
                    latency_ms=1200,
                    tool_calls=[
                        notify.RebornQaToolCall(
                            name="gmail.list_messages",
                            args_hash="1234567890123",
                            output_digest="9876543210987",
                        )
                    ],
                ),
                notify.RebornQaCaseReport(
                    rows=("2D",),
                    case="qa_2d_calendar_prep_live_chat",
                    feature="Calendar prep assistant using Google Docs and live news",
                    success=False,
                    latency_ms=0,
                    message="requires live Google runtime access",
                    debug_paths=[
                        "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/results.json",
                        "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/test-output.log",
                        "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/traces/qa_2d_calendar_prep_live_chat.json",
                        "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/traces/index.json",
                    ],
                    tool_calls=[
                        notify.RebornQaToolCall(
                            name="google-calendar.list_events",
                            args_hash="2234567890123",
                            output_digest="8876543210987",
                        )
                    ],
                ),
                notify.RebornQaCaseReport(
                    rows=("2E",),
                    case="qa_2e_calendar_prep_email_routine",
                    feature="Scheduled meeting-prep email routine",
                    success=False,
                    latency_ms=0,
                    message=(
                        "assistant returned success but routine scope "
                        "'reborn-qa-2e-calendar-prep-email' did not add a trigger_record"
                    ),
                ),
            ],
        )

        payload = notify.slack_payload(
            [report],
            "https://github.com/nearai/ironclaw/actions/runs/123",
            "abcdef0123456789",
        )
        section_texts = [
            block["text"]["text"]
            for block in payload["blocks"]
            if block.get("type") == "section"
        ]

        qa_sections = [text for text in section_texts if "*QA 2*" in text]
        self.assertEqual(len(qa_sections), 1)
        self.assertTrue(
            any(
                "*reborn-webui-v2-live-qa* (reborn-webui-v2) — 1/3 passed"
                in text
                for text in section_texts
            )
        )
        qa_text = qa_sections[0]
        self.assertIn("1/3 passed", qa_text)
        self.assertIn("\n*Cases:*", qa_text)
        self.assertIn("\n*Tools:*", qa_text)
        self.assertNotIn("\n*Tool I/O digests:*", qa_text)
        self.assertIn("`2A` Gmail connection flow", qa_text)
        self.assertIn("`2D` Calendar prep assistant using Google Docs and live news", qa_text)
        self.assertIn(
            "*Failure `2D`:* requires live Google runtime access",
            qa_text,
        )
        self.assertIn(
            "*Debug `2D`:* <https://github.com/nearai/ironclaw/actions/runs/123|GitHub run artifacts> → "
            "`reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/results.json`, "
            "`reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/test-output.log`, "
            "`reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/traces/qa_2d_calendar_prep_live_chat.json`, "
            "`reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/traces/index.json`",
            qa_text,
        )
        self.assertIn(
            "*Failure `2E`:* assistant returned success but routine scope "
            "'reborn-qa-2e-calendar-prep-email' did not add a trigger_record",
            qa_text,
        )
        self.assertNotIn("*Debug `2A`", qa_text)
        self.assertIn("*Tools:* 2 calls across 2 tools", qa_text)
        self.assertNotIn("in#1234567890", qa_text)
        self.assertNotIn("out#9876543210", qa_text)

    def test_slack_payload_includes_pr_branch_context_when_present(self):
        report = notify.LaneReport(
            lane="reborn-webui-v2-live-qa",
            provider="reborn-webui-v2",
            passed=1,
            failed=0,
            tests=1,
            duration_s=1.2,
            status="pass",
        )

        payload = notify.slack_payload(
            [report],
            "https://github.com/nearai/ironclaw/actions/runs/123",
            "mainsha0123456789",
            run_context=notify.CanaryRunContext(
                repository="nearai/ironclaw",
                trigger_context="/canary PR #42 abcdef0123 comment 987 by @maintainer",
                target_pr="42",
                target_branch="codex/canary-smoke",
                target_ref="abcdef0123456789",
            ),
        )

        context_texts = [
            element["text"]
            for block in payload["blocks"]
            if block.get("type") == "context"
            for element in block.get("elements", [])
        ]
        combined_context = "\n".join(context_texts)
        self.assertIn(
            "PR <https://github.com/nearai/ironclaw/pull/42|#42>",
            combined_context,
        )
        self.assertIn("branch `codex/canary-smoke`", combined_context)
        self.assertIn("target `abcdef0123`", combined_context)
        self.assertIn(
            "/canary PR #42 abcdef0123 comment 987 by @maintainer",
            combined_context,
        )
        self.assertIn("commit `abcdef0`", combined_context)
        self.assertNotIn("mainsha", combined_context)

    def test_github_comment_body_includes_pr_branch_and_case_results(self):
        report = notify.LaneReport(
            lane="reborn-webui-v2-live-qa",
            provider="reborn-webui-v2",
            passed=1,
            failed=1,
            tests=2,
            duration_s=2.5,
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
                    feature="Calendar prep assistant",
                    success=False,
                    latency_ms=1300,
                    message="requires live Google runtime access",
                    debug_paths=[
                        "reborn-webui-v2-live-qa/reborn-webui-v2/20260628T000000Z/results.json",
                    ],
                ),
            ],
        )

        body = notify.github_comment_body(
            [report],
            "https://github.com/nearai/ironclaw/actions/runs/123",
            "mainsha0123456789",
            run_context=notify.CanaryRunContext(
                repository="nearai/ironclaw",
                trigger_context="/canary all PR #42 abcdef0123 comment 987 by @maintainer",
                target_pr="42",
                target_branch="codex/canary-smoke",
                target_ref="abcdef0123456789",
            ),
        )

        self.assertIn("## Live canary result: 0 passed, 1 failed of 1 lanes", body)
        self.assertIn("- PR: [#42](https://github.com/nearai/ironclaw/pull/42)", body)
        self.assertIn("- Branch: `codex/canary-smoke`", body)
        self.assertIn("- Target: `abcdef0123`", body)
        self.assertIn("- Commit: `abcdef0`", body)
        self.assertIn("/canary all PR #42 abcdef0123 comment 987 by @\u200bmaintainer", body)
        self.assertNotIn("mainsha", body)
        self.assertIn("| `reborn-webui-v2-live-qa` | `reborn-webui-v2` |", body)
        self.assertIn("#### QA 2: 1/2 passed", body)
        self.assertIn(":white_check_mark: `2A` Gmail connection flow", body)
        self.assertIn(":x: `2D` Calendar prep assistant", body)
        self.assertIn("Failure: requires live Google runtime access", body)
        self.assertIn("[GitHub run artifacts]", body)

    def test_github_comment_body_includes_junit_fallback_for_failed_lane(self):
        report = notify.LaneReport(
            lane="auth|lane",
            provider="mock|provider",
            passed=0,
            failed=1,
            tests=1,
            duration_s=0.5,
            status="fail",
            junit_failures=[("test|name", "bad | value from @team\n[link](http://evil)")],
        )

        body = notify.github_comment_body(
            [report],
            "https://github.com/nearai/ironclaw/actions/runs/123",
            "abcdef0123456789",
            category_summary="failure from @team | table\n[link](http://evil)",
            run_context=notify.CanaryRunContext(
                repository="nearai/ironclaw",
                target_pr="42",
                target_branch="branch`name",
                target_ref="abcdef0123456789",
                trigger_context="/canary @team | table\n[link](http://evil)",
            ),
        )

        self.assertIn("- Branch: `branch'name`", body)
        self.assertIn("/canary @\u200bteam \\| table \\[link\\](http://evil)", body)
        self.assertIn("failure from @\u200bteam \\| table \\[link\\](http://evil)", body)
        self.assertIn("| `auth\\|lane` | `mock\\|provider` |", body)
        self.assertIn("### auth\\|lane (mock\\|provider)", body)
        self.assertIn("- Failures:", body)
        self.assertIn(
            "  - `test\\|name`: bad \\| value from @\u200bteam \\[link\\](http://evil)",
            body,
        )

    def test_post_pr_comment_validates_target_is_decimal_pull_request(self):
        for target_pr in ("42abc", "\u0664\u0662"):
            with self.assertRaisesRegex(ValueError, "decimal pull request number"):
                notify.post_pr_comment(
                    [],
                    repo="nearai/ironclaw",
                    github_token="token",
                    run_context=notify.CanaryRunContext(target_pr=target_pr),
                    run_url=None,
                    commit=None,
                )

        with (
            mock.patch.object(notify, "get_json", return_value={"number": 42}) as get_json,
            mock.patch.object(
                notify,
                "post_json",
                return_value={"html_url": "https://github.com/nearai/ironclaw/pull/42#issuecomment-1"},
            ) as post_json,
        ):
            url = notify.post_pr_comment(
                [],
                repo="nearai/ironclaw",
                github_token="token",
                run_context=notify.CanaryRunContext(target_pr="42"),
                run_url=None,
                commit=None,
            )

        self.assertEqual(
            url,
            "https://github.com/nearai/ironclaw/pull/42#issuecomment-1",
        )
        get_json.assert_called_once()
        self.assertIn("/pulls/42", get_json.call_args.args[0])
        post_json.assert_called_once()
        self.assertIn("/issues/42/comments", post_json.call_args.args[0])

    def test_main_posts_pr_comment_without_slack_webhook(self):
        report = notify.LaneReport(
            lane="reborn-webui-v2-live-qa",
            provider="reborn-webui-v2",
            passed=1,
            failed=0,
            tests=1,
            status="pass",
        )
        env = {
            "CANARY_POST_PR_COMMENT": "1",
            "CANARY_TARGET_PR": "42",
            "CANARY_TARGET_BRANCH": "codex/canary-smoke",
            "CANARY_TARGET_REF": "abcdef0123456789",
            "GITHUB_REPOSITORY": "nearai/ironclaw",
            "GITHUB_TOKEN": "token",
        }
        with (
            mock.patch.dict(notify.os.environ, env, clear=True),
            mock.patch.object(sys, "argv", ["notify_slack.py"]),
            mock.patch.object(
                notify,
                "discover_lane_dirs",
                return_value=[Path("reborn-webui-v2-live-qa/reborn-webui-v2/run")],
            ),
            mock.patch.object(notify, "collect_lane", return_value=report),
            mock.patch.object(notify, "post_json") as post_json,
            mock.patch.object(
                notify,
                "post_pr_comment",
                return_value="https://github.com/nearai/ironclaw/pull/42#issuecomment-1",
            ) as post_pr_comment,
        ):
            rc = notify.main()

        self.assertEqual(rc, 0)
        post_json.assert_not_called()
        post_pr_comment.assert_called_once()

    def test_main_ignores_pr_comment_failure_without_slack_webhook(self):
        report = notify.LaneReport(
            lane="reborn-webui-v2-live-qa",
            provider="reborn-webui-v2",
            passed=1,
            failed=0,
            tests=1,
            status="pass",
        )
        env = {
            "CANARY_POST_PR_COMMENT": "1",
            "CANARY_TARGET_PR": "42",
            "CANARY_TARGET_BRANCH": "codex/canary-smoke",
            "CANARY_TARGET_REF": "abcdef0123456789",
            "GITHUB_REPOSITORY": "nearai/ironclaw",
            "GITHUB_TOKEN": "token",
        }
        with (
            mock.patch.dict(notify.os.environ, env, clear=True),
            mock.patch.object(sys, "argv", ["notify_slack.py"]),
            mock.patch.object(
                notify,
                "discover_lane_dirs",
                return_value=[Path("reborn-webui-v2-live-qa/reborn-webui-v2/run")],
            ),
            mock.patch.object(notify, "collect_lane", return_value=report),
            mock.patch.object(notify, "post_json") as post_json,
            mock.patch.object(
                notify,
                "post_pr_comment",
                side_effect=RuntimeError("github unavailable"),
            ) as post_pr_comment,
        ):
            rc = notify.main()

        self.assertEqual(rc, 0)
        post_json.assert_not_called()
        post_pr_comment.assert_called_once()

    def test_reborn_rows_fit_with_scheduled_all_lane_report(self):
        case_rows = [
            f"{group}{suffix}"
            for group in range(2, 9)
            for suffix in ("A", "B", "C", "D", "E")
        ]
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
                passed=len(case_rows),
                failed=0,
                tests=len(case_rows),
                status="pass",
                reborn_qa_cases=[
                    notify.RebornQaCaseReport(
                        rows=(row,),
                        case=f"qa_case_{idx}",
                        feature=f"Feature {idx}",
                        success=True,
                    )
                    for idx, row in enumerate(case_rows, start=1)
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
        self.assertTrue(any("*QA 2*" in text for text in section_texts))
        self.assertTrue(any("*QA 8*" in text for text in section_texts))
        self.assertFalse(any("*QA 2A" in text for text in section_texts))

    def test_reborn_group_continuation_blocks_repeat_group_label(self):
        cases = [
            notify.RebornQaCaseReport(
                rows=("7A",),
                case=f"qa_7a_failure_{idx}",
                feature=f"Slack product channel connect {idx}",
                success=False,
                message="failure detail " + ("x" * 900),
            )
            for idx in range(8)
        ]

        blocks = notify._format_reborn_qa_group("7", cases)
        section_texts = [
            block["text"]["text"]
            for block in blocks
            if block.get("type") == "section"
        ]

        self.assertGreater(len(section_texts), 1)
        self.assertTrue(section_texts[0].startswith(":x: *QA 7* — "))
        self.assertTrue(
            all(text.startswith(":x: *QA 7* — ") for text in section_texts[1:])
        )
        self.assertTrue(any("continued" in text for text in section_texts[1:]))


if __name__ == "__main__":
    unittest.main()
