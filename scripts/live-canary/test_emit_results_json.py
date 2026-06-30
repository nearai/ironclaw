#!/usr/bin/env python3
"""Unit tests for emit_results_json.py.

The scraper is load-bearing canary alerting code — a regex regression
silently downgrades a failing lane to ``status=skip`` (the original
bug this whole PR exists to fix). Lock the parser behavior with
fixtures for every shape the producers actually emit:

  - cargo `ok` / `FAILED` summary lines
  - Rust >=1.73 multi-line panic shape (current codebase)
  - Rust <1.73 legacy inline panic shape
  - ``ignored`` tests collapsed onto the start line
  - ``note: run with`` follow-up line skipped
  - Token redaction in panic messages
  - Multi-binary logs (one cargo invocation feeding several lanes)
  - Interleaved output (parallel test threads) — must fail closed
  - Parser counts cross-checked against cargo's own ``test result:``

Run with::

    python3 -m pytest scripts/live-canary/test_emit_results_json.py -v

Or directly::

    python3 scripts/live-canary/test_emit_results_json.py
"""

from __future__ import annotations

import importlib.util
import re
import sys
import unittest
from pathlib import Path


# Load emit_results_json.py as a module without depending on a package
# layout. The script lives next to this test file by design.
_SPEC = importlib.util.spec_from_file_location(
    "emit_results_json",
    Path(__file__).parent / "emit_results_json.py",
)
emit = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(emit)


# ---------------------------------------------------------------------------
# Fixture logs — each block mirrors a real cargo invocation output shape.
# ---------------------------------------------------------------------------


MODERN_PANIC_LOG = """\

running 2 tests
test live_tests::zizmor_scan ... [LiveTest] Mode: LIVE — recording to /tmp/x

thread 'live_tests::zizmor_scan' (27813) panicked at tests/e2e_live.rs:85:9:
Expected shell tool to be used for running zizmor, got: []
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
FAILED
test live_tests::zizmor_scan_v2 ... [LiveTest] Trace recorded successfully
ok

failures:

failures:
    live_tests::zizmor_scan

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 14 filtered out; finished in 236.39s
"""


LEGACY_PANIC_LOG = """\

running 1 tests
test live_tests::legacy ... [stdout]

thread 'live_tests::legacy' panicked at 'expected X, got Y', src/lib.rs:1:1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
FAILED

failures:

failures:
    live_tests::legacy

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s
"""


IGNORED_INLINE_LOG = """\

running 3 tests
test foo::bar ... ignored
test foo::baz ... [stdout]
ok
test foo::qux ... [stdout]
ok

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.50s
"""


TOKEN_REDACTION_LOG = """\

running 1 tests
test leaky::tok ... [stdout]

thread 'leaky::tok' (1) panicked at tests/leaky.rs:1:1:
got header Authorization: Bearer sk-ant-abcdef0123456789xyz instead of expected
FAILED

failures:
    leaky::tok

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
"""


MULTI_BINARY_LOG = """\

running 1 tests
test bin_a::first ... [stdout]
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s

running 2 tests
test bin_b::pass ... [stdout]
ok
test bin_b::fail ... [stdout]

thread 'bin_b::fail' (2) panicked at tests/b.rs:5:1:
boom
FAILED

failures:
    bin_b::fail

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s
"""


PARALLEL_INTERLEAVED_LOG = """\

running 2 tests
test bin_a::first ... [stdout chunk 1]
test bin_a::second ... [stdout chunk 2]
ok
ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
"""


NON_CARGO_LOG = """\
some pytest output
running workflow probes
passed
PASSED tests/foo.py::test_bar
"""


# ---------------------------------------------------------------------------
# Helpers — cross-check parsed counts against cargo's own `test result:` summary
# ---------------------------------------------------------------------------


def _cargo_summary_totals(log: str) -> tuple[int, int, int]:
    """Return cumulative (passed, failed, ignored) across every
    ``test result:`` line in a multi-binary log."""
    passed = failed = ignored = 0
    for m in emit.RESULT_RE.finditer(log):
        passed += int(m.group("passed"))
        failed += int(m.group("failed"))
        ignored += int(m.group("ignored"))
    return passed, failed, ignored


def _counts_from_entries(entries: list[dict]) -> tuple[int, int]:
    p = sum(1 for e in entries if e.get("success"))
    f = sum(1 for e in entries if not e.get("success"))
    return p, f


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class HasCargoOutputTests(unittest.TestCase):
    def test_modern_panic_log_is_cargo(self):
        self.assertTrue(emit.has_cargo_output(MODERN_PANIC_LOG))

    def test_non_cargo_log_is_skipped(self):
        self.assertFalse(emit.has_cargo_output(NON_CARGO_LOG))

    def test_empty_log_is_skipped(self):
        self.assertFalse(emit.has_cargo_output(""))


class ParseLogModernPanicTests(unittest.TestCase):
    def test_two_entries_one_failed(self):
        entries = emit.parse_log(MODERN_PANIC_LOG)
        self.assertEqual(len(entries), 2)
        names = [e["mode"] for e in entries]
        self.assertIn("live_tests::zizmor_scan", names)
        self.assertIn("live_tests::zizmor_scan_v2", names)

    def test_modern_panic_message_extracted(self):
        entries = emit.parse_log(MODERN_PANIC_LOG)
        failed = [e for e in entries if not e["success"]]
        self.assertEqual(len(failed), 1)
        self.assertEqual(
            failed[0]["details"]["error"],
            "Expected shell tool to be used for running zizmor, got: []",
        )

    def test_modern_counts_match_cargo_summary(self):
        entries = emit.parse_log(MODERN_PANIC_LOG)
        passed, failed = _counts_from_entries(entries)
        cargo_passed, cargo_failed, _ = _cargo_summary_totals(MODERN_PANIC_LOG)
        self.assertEqual(
            (passed, failed),
            (cargo_passed, cargo_failed),
            f"parser drift: got {passed}p/{failed}f vs cargo {cargo_passed}p/{cargo_failed}f",
        )


class ParseLogLegacyPanicTests(unittest.TestCase):
    def test_legacy_inline_panic_message_extracted(self):
        entries = emit.parse_log(LEGACY_PANIC_LOG)
        self.assertEqual(len(entries), 1)
        # Legacy header carries the message inline alongside the
        # location; we accept the verbose form since we don't try to
        # strip the trailing file:line:col from the message.
        self.assertIn(
            "expected X, got Y",
            entries[0]["details"]["error"],
        )

    def test_legacy_counts_match_cargo_summary(self):
        entries = emit.parse_log(LEGACY_PANIC_LOG)
        passed, failed = _counts_from_entries(entries)
        self.assertEqual((passed, failed), (0, 1))


class IgnoredTestsTests(unittest.TestCase):
    def test_ignored_inline_excluded_from_results(self):
        entries = emit.parse_log(IGNORED_INLINE_LOG)
        self.assertEqual(len(entries), 2)
        for entry in entries:
            self.assertNotIn("ignored", entry["mode"])
            self.assertTrue(entry["success"])

    def test_ignored_counts_match_cargo_summary(self):
        # cargo reports "2 passed; 0 failed; 1 ignored"; the parser
        # emits 2 entries (passed). The ignored count is by design
        # not represented as entries — ignored tests aren't results.
        entries = emit.parse_log(IGNORED_INLINE_LOG)
        passed, failed = _counts_from_entries(entries)
        cargo_passed, cargo_failed, cargo_ignored = _cargo_summary_totals(
            IGNORED_INLINE_LOG
        )
        self.assertEqual(passed, cargo_passed)
        self.assertEqual(failed, cargo_failed)
        # Ignored stays out of entries — sanity-check that contract.
        self.assertEqual(len(entries), cargo_passed + cargo_failed)
        self.assertEqual(cargo_ignored, 1)


class NoteLineSkipTests(unittest.TestCase):
    def test_note_run_with_line_not_picked_up_as_message(self):
        # Both fixture logs have a `note: run with` line after the
        # panic header. The parser must skip past it and find the
        # real message on the line above (modern) or accept the
        # inline message (legacy).
        for log in (MODERN_PANIC_LOG, LEGACY_PANIC_LOG):
            entries = emit.parse_log(log)
            failed = [e for e in entries if not e["success"]]
            for f in failed:
                self.assertNotIn(
                    "note:",
                    f["details"]["error"],
                    f"`note: run with` leaked into panic message: {f['details']['error']}",
                )


class TokenRedactionTests(unittest.TestCase):
    def test_anthropic_key_redacted(self):
        entries = emit.parse_log(TOKEN_REDACTION_LOG)
        msg = entries[0]["details"]["error"]
        self.assertNotIn("sk-ant-abcdef0123456789xyz", msg)
        self.assertIn("REDACTED", msg)

    def test_redact_function_covers_documented_shapes(self):
        # Belt-and-braces — verify each pattern listed in
        # REDACT_PATTERNS actually transforms its input. This list is
        # the union of provider-specific shapes the canary actually
        # exercises plus the generic assignment / JSON-quoted shapes
        # carried by scripts/live-canary/scrub-artifacts.sh — keep them
        # in sync so a token can't slip through emit_results_json that
        # the shell scrubber would have caught.
        cases = [
            ("token ghp_aaaaaaaaaaaaaaaaaaaa", "REDACTED_GITHUB_TOKEN"),
            ("token github_pat_bbbbbbbbbbbbbbbbbbbbb", "REDACTED_GITHUB_PAT"),
            ("token ya29.cccccccccccccccccc1234", "REDACTED_GOOGLE_TOKEN"),
            ("token xoxb-1234567890-abc", "REDACTED_SLACK_TOKEN"),
            ("Authorization: Bearer eyJabc.defg+hij/kl=", "Bearer <REDACTED>"),
            # OpenAI bare key shape (sk- prefix without `ant-`).
            ("token sk-abcDEF0123456789ghij", "REDACTED_OPENAI_KEY"),
            # AWS access key ID literal — fixed AKIA prefix + 16 upper/digit.
            ("aws AKIAIOSFODNN7EXAMPLE call", "REDACTED_AWS_ACCESS_KEY"),
            # Generic env-var / assignment shapes the persona harness
            # could leak (e.g. LIVE_CANARY_COMPOSIO_API_KEY=…). The
            # generic api_key rule is the catch-all for any provider
            # without a published key prefix (e.g. Composio).
            ("api_key=composio-abcdef12345", "api_key=<REDACTED>"),
            ("ACCESS_TOKEN: abcdef.1234567", "ACCESS_TOKEN=<REDACTED>"),
            ("refresh-token = xyz.987654321", "refresh-token=<REDACTED>"),
            ("client_secret=hunter2", "client_secret=<REDACTED>"),
            ("password=hunter2", "password=<REDACTED>"),
            ('"access_token": "abc.def.ghi"', '"access_token": "<REDACTED>"'),
            ('"refresh_token": "rt-abc-123"', '"refresh_token": "<REDACTED>"'),
            ('"api_key": "composio-abc"', '"api_key": "<REDACTED>"'),
            ('"client_secret": "shh"', '"client_secret": "<REDACTED>"'),
        ]
        for raw, marker in cases:
            redacted = emit.redact(raw)
            self.assertIn(marker, redacted, f"redact() failed for {raw!r}")

    def test_anthropic_key_wins_over_openai_pattern(self):
        # Both `sk-ant-…` and `sk-…` are valid prefixes. The Anthropic
        # rule is listed first so the dedicated label survives — and
        # the OpenAI rule uses a negative lookahead as belt-and-braces
        # for future reorderings. Lock both invariants here.
        out = emit.redact("token sk-ant-abcdef0123456789xyz")
        self.assertIn("REDACTED_ANTHROPIC_KEY", out)
        self.assertNotIn("REDACTED_OPENAI_KEY", out)

    def test_openai_pattern_does_not_swallow_sk_ant(self):
        # If someone reorders REDACT_PATTERNS, the negative lookahead
        # on the OpenAI rule still keeps `sk-ant-…` strings out of the
        # OpenAI bucket. Test the rule in isolation.
        for pattern, replacement in emit.REDACT_PATTERNS:
            if replacement == "<REDACTED_OPENAI_KEY>":
                self.assertIsNone(
                    pattern.search("sk-ant-abcdef0123456789xyz"),
                    "OpenAI pattern must not match sk-ant- strings",
                )
                self.assertIsNotNone(
                    pattern.search("sk-abcDEF0123456789ghij"),
                    "OpenAI pattern must match bare sk- strings",
                )
                break
        else:
            self.fail("no <REDACTED_OPENAI_KEY> rule in REDACT_PATTERNS")


class MultiBinaryTests(unittest.TestCase):
    def test_multi_binary_log_aggregates_all_entries(self):
        entries = emit.parse_log(MULTI_BINARY_LOG)
        # bin_a: 1 ok; bin_b: 1 ok + 1 FAILED → 3 entries total
        self.assertEqual(len(entries), 3)
        names = [e["mode"] for e in entries]
        self.assertEqual(
            names,
            ["bin_a::first", "bin_b::pass", "bin_b::fail"],
        )

    def test_multi_binary_panic_message_attached_to_correct_test(self):
        entries = emit.parse_log(MULTI_BINARY_LOG)
        failed = [e for e in entries if not e["success"]]
        self.assertEqual(len(failed), 1)
        self.assertEqual(failed[0]["mode"], "bin_b::fail")
        self.assertEqual(failed[0]["details"]["error"], "boom")

    def test_multi_binary_counts_match_cumulative_cargo_summary(self):
        entries = emit.parse_log(MULTI_BINARY_LOG)
        passed, failed = _counts_from_entries(entries)
        cargo_passed, cargo_failed, _ = _cargo_summary_totals(MULTI_BINARY_LOG)
        # cargo reports two summaries: 1p/0f then 1p/1f → cumulative 2p/1f
        self.assertEqual((passed, failed), (cargo_passed, cargo_failed))
        self.assertEqual((cargo_passed, cargo_failed), (2, 1))


class InterleavedOutputTests(unittest.TestCase):
    def test_parallel_log_raises_interleaved_error(self):
        with self.assertRaises(emit.InterleavedOutputError) as ctx:
            emit.parse_log(PARALLEL_INTERLEAVED_LOG)
        # Error message should name both tests so debugging is easy.
        msg = str(ctx.exception)
        self.assertIn("bin_a::first", msg)
        self.assertIn("bin_a::second", msg)
        self.assertIn("--test-threads=1", msg)


class RegexShapeTests(unittest.TestCase):
    """Lock in the regexes themselves — if any of these match
    something they shouldn't (or stop matching the canonical form),
    the parser silently produces wrong output. Catch that here."""

    def test_result_re_matches_canonical_failed_line(self):
        line = (
            "test result: FAILED. 1 passed; 1 failed; 0 ignored; "
            "0 measured; 14 filtered out; finished in 236.39s"
        )
        m = emit.RESULT_RE.search(line)
        self.assertIsNotNone(m)
        self.assertEqual(m.group("outcome"), "FAILED")
        self.assertEqual(m.group("passed"), "1")
        self.assertEqual(m.group("failed"), "1")
        self.assertEqual(m.group("ignored"), "0")

    def test_test_start_re_matches_with_stdout_glue(self):
        m = emit.TEST_START_RE.match(
            "test live_tests::zizmor_scan ... [LiveTest] Mode: LIVE — recording"
        )
        self.assertIsNotNone(m)
        self.assertEqual(m.group("name"), "live_tests::zizmor_scan")
        # `[LiveTest] Mode: ...` is not a clean outcome word.
        self.assertNotIn(m.group("trailer").strip(), {"ok", "FAILED", "ignored"})

    def test_test_start_re_matches_inline_ignored(self):
        m = emit.TEST_START_RE.match("test foo::bar ... ignored")
        self.assertIsNotNone(m)
        self.assertEqual(m.group("trailer").strip(), "ignored")

    def test_outcome_re_only_matches_bare_lines(self):
        for good in ("ok", "FAILED", "ignored"):
            self.assertIsNotNone(emit.OUTCOME_RE.match(good))
        for bad in (
            "ok and then some",
            "  ok",  # leading whitespace not allowed
            "OK",  # case-sensitive
            "test foo ... ok",
            "",
        ):
            self.assertIsNone(emit.OUTCOME_RE.match(bad), f"falsely matched: {bad!r}")

    def test_panic_re_matches_modern(self):
        m = emit.PANIC_RE.match(
            "thread 'live_tests::zizmor_scan' (27813) panicked at tests/e2e_live.rs:85:9:"
        )
        self.assertIsNotNone(m)
        self.assertEqual(m.group("name"), "live_tests::zizmor_scan")

    def test_panic_re_matches_legacy(self):
        # No worker id between thread name and `panicked at` — the
        # lazy `.*?` between them must allow this.
        m = emit.PANIC_RE.match(
            "thread 'live_tests::legacy' panicked at 'expected X', src/lib.rs:1:1"
        )
        self.assertIsNotNone(m)
        self.assertEqual(m.group("name"), "live_tests::legacy")


if __name__ == "__main__":
    unittest.main()
