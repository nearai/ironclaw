#!/usr/bin/env python3
"""Scrape `cargo test --nocapture` output into a results.json file that
notify_slack.py::parse_results_json can already consume.

Invoked from scripts/live-canary/run.sh at the end of every cargo-based
lane. Skipped (early-exits as no-op) when:

  * --out already exists — workflow-canary writes its own results.json
    via scripts/workflow_canary/run_workflow_canary.py, and that file
    must not be overwritten.
  * The log contains no `test result:` line — auth-* lanes use pytest +
    JUnit XML and produce no cargo-style output, so there is nothing to
    scrape.

Schema matches the workflow-canary contract (see parse_results_json):

    {"results": [
        {"provider": "...", "mode": "<test_name>",
         "success": bool, "latency_ms": int,
         "details": {"error": "<short panic msg>"}},
        ...
    ]}

We only emit one entry per executed test (cargo `ok` or `FAILED`).
`ignored` tests are not results — they didn't run — so they are skipped
entirely.

Per-test latency is unknowable from cargo's plain stdout, so we leave
``latency_ms`` at 0 and put the lane-level wall-clock duration on a
single ``meta`` entry consumed only by future tooling. notify_slack.py
sums ``latency_ms`` per entry so the 0 values are harmless — lane-level
duration is captured by the workflow itself.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# Cargo test status line — emitted by cargo at the end of each test
# binary invocation, after all per-test output. Its presence is also our
# gate for "is this a cargo lane".
#
#   test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 14 filtered out; finished in 236.39s
#   test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.34s
RESULT_RE = re.compile(
    r"test result: (?P<outcome>ok|FAILED)\. "
    r"(?P<passed>\d+) passed; "
    r"(?P<failed>\d+) failed; "
    r"(?P<ignored>\d+) ignored"
)

# Test-start line. With --nocapture, the test's own stdout gets glued
# onto the same line right after the `...`, so we can't expect a clean
# trailing outcome here — only the test name on the left.
#
#   test live_tests::zizmor_scan ... [LiveTest] Mode: LIVE — recording to ...
TEST_START_RE = re.compile(r"^test (?P<name>[\w:]+) \.\.\. ?(?P<trailer>.*)$")

# Standalone end-of-test outcome lines (cargo emits these on a line of
# their own after the test's stdout in --nocapture mode). With
# --test-threads=1 the most recent TEST_START_RE match owns the next
# standalone outcome.
OUTCOME_RE = re.compile(r"^(?P<outcome>ok|FAILED|ignored)\s*$")

# Panic header. Two shapes in the wild:
#
#   Rust >= 1.73 (current — message on next line):
#     thread 'live_tests::zizmor_scan' (27813) panicked at tests/e2e_live.rs:85:9:
#     Expected shell tool to be used for running zizmor, got: []
#
#   Rust < 1.73 (legacy — message inline on the header):
#     thread 'foo' panicked at 'expected X, got Y', src/lib.rs:1:1
#
# `.*?` (lazy) is critical: it lets the regex match the legacy form
# where there's nothing between the closing quote and ` panicked at `.
# Greedy `.*` here would have required at least one character of
# between-text (a worker-id like `(27813)`) and missed the legacy form.
PANIC_RE = re.compile(r"^thread '(?P<name>[\w:]+)'.*? panicked at ")

MAX_ERROR_LEN = 240

# Defense in depth: panic messages from e2e tests *could* embed a real
# token if an assertion happens to dump a captured response body. Redact
# the obvious shapes before writing so a token can never reach the
# artifact store via results.json, regardless of what scrub-artifacts.sh
# decides to do downstream. Keep this list aligned with scrub-artifacts.sh.
REDACT_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"), "<REDACTED_GITHUB_TOKEN>"),
    (re.compile(r"github_pat_[A-Za-z0-9_]{20,}"), "<REDACTED_GITHUB_PAT>"),
    (re.compile(r"ya29\.[A-Za-z0-9._-]{20,}"), "<REDACTED_GOOGLE_TOKEN>"),
    (re.compile(r"xox[baprs]-[A-Za-z0-9-]{10,}"), "<REDACTED_SLACK_TOKEN>"),
    (re.compile(r"sk-ant-[A-Za-z0-9_-]{10,}"), "<REDACTED_ANTHROPIC_KEY>"),
    (re.compile(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]+"), "Bearer <REDACTED>"),
]


def redact(text: str) -> str:
    for pattern, replacement in REDACT_PATTERNS:
        text = pattern.sub(replacement, text)
    return text


def parse_log(log_text: str) -> list[dict]:
    """Return one entry per executed test (cargo ok / FAILED).

    Tests reported as ``ignored`` are excluded — they did not run, so
    they are not results.

    Strategy: walk the log once and pair each `test <name> ...` start
    with the next standalone `ok`/`FAILED`/`ignored` token. With
    --test-threads=1 (which every cargo lane in run.sh uses) tests run
    serially, so each start owns the next outcome unambiguously.

    Inline `... ignored` shortcut (cargo collapses ignored tests onto
    one line because they produce no stdout) is also handled.
    """
    lines = log_text.splitlines()

    # First pass: panic messages keyed by test name.
    panic_messages: dict[str, str] = {}
    for i, line in enumerate(lines):
        m = PANIC_RE.match(line)
        if not m:
            continue

        # Rust < 1.73 emits the panic message inline on the header:
        #
        #   thread 'foo' panicked at 'expected X, got Y', src/lib.rs:1:1
        #
        # Rust >= 1.73 puts only the location on the header and the
        # message on the following line:
        #
        #   thread 'foo' panicked at src/lib.rs:1:1:
        #   expected X, got Y
        #
        # A trailing `:` on the suffix is the location-only signal; if
        # the suffix carries anything else, that *is* the message.
        suffix = line[m.end() :].strip()
        if suffix and not suffix.endswith(":") and not suffix.startswith("note:"):
            panic_messages[m.group("name")] = redact(suffix)[:MAX_ERROR_LEN]
            continue

        for follow in lines[i + 1 : i + 6]:
            stripped = follow.strip()
            if not stripped:
                continue
            if stripped.startswith("note: run with"):
                break
            panic_messages[m.group("name")] = redact(stripped)[:MAX_ERROR_LEN]
            break

    # Second pass: pair starts with outcomes.
    entries: list[dict] = []
    current: str | None = None

    def record(name: str, outcome: str) -> None:
        if outcome == "ignored":
            return
        success = outcome == "ok"
        entry: dict = {
            "provider": "",  # filled in by caller
            "mode": name,
            "success": success,
            "latency_ms": 0,
        }
        if not success:
            entry["details"] = {
                "error": panic_messages.get(
                    name, "test failed (no panic message captured)"
                ),
            }
        entries.append(entry)

    for line in lines:
        start = TEST_START_RE.match(line)
        if start:
            trailer = start.group("trailer").strip()
            # Cargo collapses ignored tests inline: `test foo ... ignored`.
            if trailer in {"ok", "FAILED", "ignored"}:
                record(start.group("name"), trailer)
                current = None
                continue
            current = start.group("name")
            continue

        outcome = OUTCOME_RE.match(line)
        if outcome and current is not None:
            record(current, outcome.group("outcome"))
            current = None

    return entries


def has_cargo_output(log_text: str) -> bool:
    """Cheap gate so non-cargo lanes (auth-*, workflow-canary) are no-ops."""
    return RESULT_RE.search(log_text) is not None


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--log", required=True, type=Path)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--lane", required=True)
    p.add_argument("--provider", required=True)
    args = p.parse_args()

    # Never clobber a results.json written by another tool (workflow-canary
    # writes its own, with richer per-probe details).
    if args.out.exists():
        print(
            f"[emit_results_json] {args.out} already present — leaving untouched",
            file=sys.stderr,
        )
        return 0

    if not args.log.exists():
        print(f"[emit_results_json] no log at {args.log} — skipping", file=sys.stderr)
        return 0

    log_text = args.log.read_text(encoding="utf-8", errors="replace")
    if not has_cargo_output(log_text):
        # Not a cargo lane — auth/workflow lanes have their own count
        # files. Silent no-op so this is safe to wire unconditionally.
        return 0

    entries = parse_log(log_text)
    for entry in entries:
        entry["provider"] = args.provider

    payload = {
        "lane": args.lane,
        "provider": args.provider,
        "results": entries,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(
        f"[emit_results_json] wrote {len(entries)} entries to {args.out}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
