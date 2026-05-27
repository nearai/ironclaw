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
import sys
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


if __name__ == "__main__":
    unittest.main()
