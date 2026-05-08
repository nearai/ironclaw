#!/usr/bin/env python3
"""Canary report: walk lane artifacts, summarize via Haiku, post to Slack.

Invoked by the `canary-report` GitHub Actions job after every live-canary
lane finishes. Expects artifacts under ``--artifacts-dir`` following the
standard ``<lane>/<provider>/<timestamp>/`` layout produced by
``scripts/live-canary/run.sh``.

Zero external dependencies — uses only the stdlib so it can run in any CI
shell. Exits 0 even on Haiku / Slack failure so the notifier never blocks
CI; errors degrade to a raw "X/Y lanes failed — <run URL>" fallback.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path

MODEL = "claude-haiku-4-5-20251001"
ANTHROPIC_URL = "https://api.anthropic.com/v1/messages"
ANTHROPIC_VERSION = "2023-06-01"
MAX_LOG_BYTES = 20_000

HAIKU_SYSTEM = (
    "You analyze CI canary test logs. Given a lane's summary, JUnit digest, "
    "and log tail, return ONLY a JSON object with these keys:\n"
    '  status: "pass" | "fail" | "skip"\n'
    "  reason: string, <=200 chars, one-sentence cause if failed (else empty)\n"
    "  tool_calls_total: integer, 0 if none visible\n"
    "  tools_used: list of distinct tool names (up to 10)\n"
    "  notable: string, <=200 chars, anything worth flagging (else empty)\n"
    "  test_name: string, the failing test's identifier "
    "(file::test_fn or scenario name). Empty if not failed or not knowable.\n"
    "  error: string, <=300 chars, the assertion / exception / "
    "timeout the test reported. Empty if not failed.\n"
    "  root_cause: string, <=400 chars, your best hypothesis for "
    "why it failed (e.g. malformed env var, upstream regression, "
    "flake). Empty if not failed.\n"
    "  fix: string, <=300 chars, concrete next step (e.g. "
    "'trim CI variable LIVE_OPENAI_COMPATIBLE_BASE_URL'). Empty if "
    "not failed.\n"
    "Do not include prose outside the JSON. If the log is empty or ambiguous, "
    "still produce the object with best-effort fields. For passing or skipped "
    "lanes, leave test_name/error/root_cause/fix empty strings."
)

CATEGORIZE_SYSTEM = (
    "You group canary failures by root cause across multiple lanes. "
    "Given a JSON array of failed-lane summaries (each with lane, provider, "
    "test_name, error, root_cause, fix), return ONLY a JSON object:\n"
    '  categories: list of {category, jobs: [list of "lane (provider)"], fix}\n'
    "Group by SHARED root cause — e.g. all lanes hit by the same bug get one "
    "category. If a lane has a unique root cause, it gets its own category. "
    "Keep `category` <=60 chars (a short label like 'WASM tool dispatch "
    "regression' or 'Malformed CI variable'). Keep `fix` <=200 chars. "
    "Do not include prose outside the JSON."
)


@dataclass
class LaneReport:
    lane: str
    provider: str
    passed: int = 0
    failed: int = 0
    skipped: int = 0
    tests: int = 0
    duration_s: float = 0.0
    junit_failures: list[tuple[str, str]] = field(default_factory=list)
    status: str = "unknown"
    reason: str = ""
    tool_calls_total: int = 0
    tools_used: list[str] = field(default_factory=list)
    notable: str = ""
    summary_md: str = ""
    log_tail: str = ""
    # Haiku-derived diagnostic fields (failed lanes only).
    test_name: str = ""
    error: str = ""
    root_cause: str = ""
    fix: str = ""


def read_tail(path: Path, n_bytes: int) -> str:
    if not path.exists():
        return ""
    size = path.stat().st_size
    with path.open("rb") as f:
        if size > n_bytes:
            f.seek(size - n_bytes)
        data = f.read()
    return data.decode("utf-8", errors="replace")


def parse_junit(path: Path, report: LaneReport) -> None:
    if not path.exists() or path.stat().st_size == 0:
        return
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError:
        return
    for ts in root.iter("testsuite"):
        report.tests += int(ts.get("tests", 0) or 0)
        report.failed += int(ts.get("failures", 0) or 0) + int(ts.get("errors", 0) or 0)
        report.skipped += int(ts.get("skipped", 0) or 0)
        report.duration_s += float(ts.get("time", 0.0) or 0.0)
    report.passed = max(report.tests - report.failed - report.skipped, 0)
    for tc in root.iter("testcase"):
        name = tc.get("name", "?")
        failure = tc.find("failure")
        error = tc.find("error")
        node = failure if failure is not None else error
        if node is not None:
            msg = (node.get("message") or "").strip()
            report.junit_failures.append((name, msg[:240]))


def parse_results_json(path: Path, report: LaneReport) -> None:
    """Parse a workflow-canary-shaped ``results.json``.

    Schema (one entry per probe):

        {"results": [
            {"provider", "mode", "success": bool, "latency_ms": int, "details": {...}},
            ...
        ]}

    ``passed`` = count(success); ``failed`` = count(!success); each failure
    becomes a (name, message) entry on ``junit_failures`` so the Slack
    output renders the same way an auth-canary failure would. Skipped
    counts stay at 0 — workflow-canary scenarios always run when the
    lane is enabled.
    """
    if not path.exists() or path.stat().st_size == 0:
        return
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return
    results = data.get("results") or []
    if not isinstance(results, list):
        return
    for entry in results:
        if not isinstance(entry, dict):
            continue
        report.tests += 1
        if entry.get("success"):
            report.passed += 1
        else:
            report.failed += 1
            name = (
                f"{entry.get('provider', '?')}/{entry.get('mode', '?')}"
            )
            msg = ""
            details = entry.get("details") or {}
            if isinstance(details, dict):
                msg = str(details.get("error") or "")
                if not msg:
                    # No structured error — surface a short status pair so
                    # the Slack reason isn't blank for soft failures
                    # (e.g. run_terminal but assertion didn't match).
                    fragments: list[str] = []
                    for k, v in details.items():
                        if k in ("routine_id", "run_status", "run_count"):
                            continue
                        fragments.append(f"{k}={v}")
                        if len(fragments) >= 3:
                            break
                    msg = ", ".join(fragments)
            report.junit_failures.append((name, msg[:240]))
        latency = entry.get("latency_ms")
        if isinstance(latency, (int, float)):
            report.duration_s += latency / 1000.0


def collect_lane(lane_dir: Path) -> LaneReport | None:
    parts = lane_dir.parts
    if len(parts) < 3:
        return None
    lane = parts[-3]
    provider = parts[-2]
    r = LaneReport(lane=lane, provider=provider)
    # Auth-canary lanes write JUnit XML; workflow-canary writes its own
    # results.json. Read whichever exists — both populate the same
    # LaneReport fields so downstream rendering / Haiku enrichment is
    # source-agnostic.
    parse_junit(lane_dir / "auth-canary-junit.xml", r)
    parse_results_json(lane_dir / "results.json", r)
    r.summary_md = read_tail(lane_dir / "summary.md", 4_000)
    r.log_tail = read_tail(lane_dir / "test-output.log", MAX_LOG_BYTES)
    if r.tests == 0 and not r.log_tail:
        r.status = "skip"
    elif r.failed > 0:
        r.status = "fail"
    elif r.tests > 0:
        r.status = "pass"
    return r


def discover_lane_dirs(artifacts_root: Path) -> list[Path]:
    """Return the latest <lane>/<provider>/<timestamp> dir for each lane+provider."""
    if not artifacts_root.exists():
        return []
    out: list[Path] = []
    for lane_dir in sorted(p for p in artifacts_root.iterdir() if p.is_dir()):
        for provider_dir in sorted(p for p in lane_dir.iterdir() if p.is_dir()):
            runs = sorted(
                (p for p in provider_dir.iterdir() if p.is_dir()),
                reverse=True,
            )
            if runs:
                out.append(runs[0])
    return out


def post_json(url: str, payload: dict, headers: dict[str, str], timeout: int = 20) -> dict:
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json", **headers})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as e:
        # urlopen raises HTTPError for 4xx/5xx; the response body often
        # carries the useful detail (Anthropic "invalid API key" etc.).
        err_body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {err_body[:200]}") from e
    try:
        return json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return {"_raw": raw}


def get_json(url: str, headers: dict[str, str], timeout: int = 20) -> dict:
    """GET a JSON payload. Mirrors `post_json`'s error-mapping shape."""
    req = urllib.request.Request(url, headers=headers, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {err_body[:200]}") from e
    try:
        return json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return {"_raw": raw}


def run_haiku(api_key: str, report: LaneReport) -> None:
    """Enrich report with Haiku-derived fields. Degrades silently on failure."""
    junit = (
        f"tests={report.tests} passed={report.passed} failed={report.failed} "
        f"skipped={report.skipped} duration={report.duration_s:.1f}s"
    )
    failures_block = "\n".join(f"- {n}: {m}" for n, m in report.junit_failures[:10]) or "(none)"
    user_msg = (
        f"Lane: {report.lane}\n"
        f"Provider: {report.provider}\n"
        f"JUnit digest: {junit}\n"
        f"JUnit failures:\n{failures_block}\n\n"
        f"summary.md:\n{report.summary_md[:1500]}\n\n"
        f"test-output.log tail (up to {MAX_LOG_BYTES} bytes):\n"
        f"{report.log_tail}"
    )
    payload = {
        "model": MODEL,
        "max_tokens": 512,
        "system": HAIKU_SYSTEM,
        "messages": [{"role": "user", "content": user_msg}],
    }
    headers = {"x-api-key": api_key, "anthropic-version": ANTHROPIC_VERSION}
    try:
        resp = post_json(ANTHROPIC_URL, payload, headers, timeout=45)
    except Exception as e:
        report.notable = f"haiku call failed: {type(e).__name__}"[:200]
        return
    text = ""
    for block in resp.get("content", []):
        if block.get("type") == "text":
            text += block.get("text", "")
    text = text.strip()
    # Haiku is instructed to return ONLY a JSON object, but extract the
    # outermost `{...}` span so we survive the odd case where the model
    # adds a prose preamble or wraps the output in a ```json fence.
    # Greedy + DOTALL matches first `{` to last `}` — correct for a
    # single top-level object, which our schema requires.
    match = re.search(r"\{.*\}", text, re.DOTALL)
    if match is None:
        report.notable = f"haiku returned no JSON object: {text[:160]}"
        return
    try:
        data = json.loads(match.group(0))
    except json.JSONDecodeError:
        report.notable = f"haiku JSON parse failed: {match.group(0)[:160]}"
        return
    if isinstance(data.get("status"), str):
        report.status = data["status"]
    report.reason = str(data.get("reason", ""))[:200]
    try:
        report.tool_calls_total = int(data.get("tool_calls_total", 0))
    except (TypeError, ValueError):
        pass
    tu = data.get("tools_used", [])
    if isinstance(tu, list):
        report.tools_used = [str(x) for x in tu][:10]
    report.notable = str(data.get("notable", ""))[:200]
    # Per-failure diagnostic fields. Haiku returns empty strings for
    # passing/skipped lanes, so accept and store as-is — slack_payload
    # only renders the rich block when the field is non-empty.
    report.test_name = str(data.get("test_name", ""))[:200]
    report.error = str(data.get("error", ""))[:300]
    report.root_cause = str(data.get("root_cause", ""))[:400]
    report.fix = str(data.get("fix", ""))[:300]


def slack_payload(
    reports: list[LaneReport],
    run_url: str | None,
    commit: str | None,
    *,
    category_summary: str = "",
) -> dict:
    emoji = {"pass": ":white_check_mark:", "fail": ":x:", "skip": ":heavy_minus_sign:"}
    red = sum(1 for r in reports if r.status == "fail")
    green = sum(1 for r in reports if r.status == "pass")
    header = f"Canary: {green} passed, {red} failed of {len(reports)} lanes"
    blocks: list[dict] = [
        {"type": "header", "text": {"type": "plain_text", "text": header}},
    ]
    for r in reports:
        header_line = (
            f"{emoji.get(r.status, ':grey_question:')} *{r.lane}* ({r.provider}) — "
            f"{r.passed}/{r.tests} passed, {r.failed} failed in {r.duration_s:.0f}s"
        )
        lines = [header_line]
        # Rich failure block: shown when Haiku populated the diagnostic
        # fields. The shape mirrors the issue-friendly format reviewers
        # asked for so a Slack reader can paste it straight into a
        # GitHub issue if needed.
        if r.status == "fail" and (r.test_name or r.error or r.root_cause):
            if r.test_name:
                lines.append(f"  *Test:* `{r.test_name}`")
            if r.error:
                lines.append(f"  *Error:* {r.error}")
            if r.root_cause:
                lines.append(f"  *Root Cause:* {r.root_cause}")
            if r.fix:
                lines.append(f"  *Fix:* {r.fix}")
        elif r.reason:
            # For passing/skipped lanes we keep the existing single-
            # line reason summary (Haiku's free-form notable).
            lines.append(f"> {r.reason}")
        if r.tools_used:
            lines.append(f"tools: {', '.join(r.tools_used)} (≈{r.tool_calls_total} calls)")
        if r.notable:
            lines.append(f"_{r.notable}_")
        blocks.append({"type": "section", "text": {"type": "mrkdwn", "text": "\n".join(lines)}})

    # Cross-lane "Summary by Category" block — only emitted when there
    # are >=2 failures (with 1 the per-lane block is already enough).
    if category_summary:
        blocks.append({"type": "divider"})
        blocks.append(
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": f"*Summary by Category*\n{category_summary}",
                },
            }
        )
    ctx: list[str] = []
    if commit:
        ctx.append(f"commit `{commit[:7]}`")
    if run_url:
        ctx.append(f"<{run_url}|GitHub run>")
    if ctx:
        blocks.append({"type": "context", "elements": [{"type": "mrkdwn", "text": " • ".join(ctx)}]})
    return {"blocks": blocks}


def categorize_failures(api_key: str, reports: list[LaneReport]) -> str:
    """Second-pass Haiku call: group failed lanes by shared root cause.

    Returns a multi-line markdown string ready to paste into a Slack
    section block. Empty string if no failures or fewer than 2 (the
    per-lane block already conveys a single failure clearly without
    a category summary).
    """
    failed = [r for r in reports if r.status == "fail"]
    if len(failed) < 2:
        return ""
    payload_failures = [
        {
            "lane": r.lane,
            "provider": r.provider,
            "test_name": r.test_name,
            "error": r.error,
            "root_cause": r.root_cause,
            "fix": r.fix,
        }
        for r in failed
    ]
    user_msg = (
        "Failed lanes:\n"
        f"{json.dumps(payload_failures, indent=2)}"
    )
    payload = {
        "model": MODEL,
        "max_tokens": 800,
        "system": CATEGORIZE_SYSTEM,
        "messages": [{"role": "user", "content": user_msg}],
    }
    headers = {"x-api-key": api_key, "anthropic-version": ANTHROPIC_VERSION}
    try:
        resp = post_json(ANTHROPIC_URL, payload, headers, timeout=45)
    except Exception as e:
        return f"_(category summary unavailable: {type(e).__name__})_"
    text = ""
    for block in resp.get("content", []):
        if block.get("type") == "text":
            text += block.get("text", "")
    text = text.strip()
    match = re.search(r"\{.*\}", text, re.DOTALL)
    if match is None:
        return ""
    try:
        data = json.loads(match.group(0))
    except json.JSONDecodeError:
        return ""
    categories = data.get("categories", [])
    if not isinstance(categories, list) or not categories:
        return ""

    # Render as a Slack-friendly bulleted block — Slack's mrkdwn
    # doesn't support actual tables, so we fall back to a pretty
    # itemized list. Keeps the Slack message <=3000 chars per block.
    lines: list[str] = []
    for entry in categories:
        if not isinstance(entry, dict):
            continue
        cat = str(entry.get("category", "?"))[:60]
        jobs = entry.get("jobs", [])
        jobs_str = (
            ", ".join(str(j) for j in jobs[:6])
            if isinstance(jobs, list)
            else "?"
        )
        fix = str(entry.get("fix", ""))[:200]
        lines.append(f"• *{cat}* — _{jobs_str}_")
        if fix:
            lines.append(f"   Fix: {fix}")
    return "\n".join(lines)


def create_canary_issues(
    reports: list[LaneReport],
    *,
    repo: str,
    github_token: str,
    run_url: str | None,
    commit: str | None,
) -> list[str]:
    """Open / update one GitHub issue per failed lane, deduplicated.

    Strategy:
    - Title: ``[canary] <lane>: <test_name or "lane failure">``. Stable
      across runs so the dedup search by title hits the same issue
      next time.
    - Search the repo for an OPEN issue with that exact title.
    - If found: comment "another occurrence on <run_url>" + bump.
    - If not found: create a new issue with the rich body and
      ``canary-failure`` label.

    Returns a list of issue URLs (created OR updated). Errors are
    swallowed and logged to stderr — the notifier never blocks CI.

    Gated on:
    - ``CANARY_CREATE_ISSUES=1`` env var (off by default — issue spam
      is a real risk if the same flake fires every 6h).
    - A non-empty ``GITHUB_TOKEN`` with ``issues: write`` permission.
    """
    failed = [r for r in reports if r.status == "fail"]
    if not failed:
        return []
    base = f"https://api.github.com/repos/{repo}"
    headers = {
        "Authorization": f"Bearer {github_token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    out: list[str] = []
    for r in failed:
        test_label = r.test_name or "lane failure"
        title = f"[canary] {r.lane}: {test_label}"
        body_lines = [
            f"**Lane:** `{r.lane}` (`{r.provider}`)",
            f"**Counts:** {r.passed}/{r.tests} passed, {r.failed} failed in {r.duration_s:.0f}s",
        ]
        if r.test_name:
            body_lines.append(f"**Test:** `{r.test_name}`")
        if r.error:
            body_lines.append(f"**Error:** {r.error}")
        if r.root_cause:
            body_lines.append(f"**Root Cause:** {r.root_cause}")
        if r.fix:
            body_lines.append(f"**Fix:** {r.fix}")
        if commit:
            body_lines.append(f"**Commit:** `{commit[:12]}`")
        if run_url:
            body_lines.append(f"**Run:** {run_url}")
        body_lines.append("")
        body_lines.append(
            "_Auto-opened by `scripts/live-canary/notify_slack.py`. "
            "Will be re-used on subsequent runs that hit the same "
            "failing test — close when fixed or convert to a "
            "tracking issue._"
        )
        body = "\n".join(body_lines)

        # Search for an existing open issue with the exact title. The
        # search API ranks fuzzily, so we filter by exact-title match
        # below before deciding whether to create or comment.
        from urllib.parse import quote_plus

        q = quote_plus(f'repo:{repo} is:issue is:open in:title "{title}"')
        try:
            search = get_json(
                f"https://api.github.com/search/issues?q={q}",
                headers,
                timeout=15,
            )
        except Exception:
            search = {"items": []}
        existing = next(
            (
                it
                for it in (search.get("items") or [])
                if it.get("title") == title
            ),
            None,
        )
        try:
            if existing:
                comment_url = existing.get("comments_url")
                if not comment_url:
                    continue
                comment_body = (
                    f"Another canary occurrence on `{commit[:7] if commit else '?'}`. "
                    f"Run: {run_url or '?'}"
                )
                post_json(
                    comment_url,
                    {"body": comment_body},
                    headers,
                    timeout=15,
                )
                out.append(existing.get("html_url", ""))
            else:
                created = post_json(
                    f"{base}/issues",
                    {
                        "title": title,
                        "body": body,
                        "labels": ["canary-failure", f"lane:{r.lane}"],
                    },
                    headers,
                    timeout=15,
                )
                out.append(created.get("html_url", ""))
        except Exception as e:
            print(
                f"[notify_slack] github issue create/update failed for "
                f"{r.lane}: {type(e).__name__}: {e}",
                file=sys.stderr,
            )
    return out


def fallback_payload(reports: list[LaneReport], run_url: str | None) -> dict:
    red = sum(1 for r in reports if r.status == "fail")
    text = f"Canary: {red}/{len(reports)} lanes failed"
    if run_url:
        text += f" — {run_url}"
    return {"text": text}


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--artifacts-dir", default="artifacts/live-canary",
                   help="root of downloaded lane artifacts")
    p.add_argument("--slack-webhook", default=os.environ.get("SLACK_WEBHOOK_URL"))
    p.add_argument("--anthropic-api-key", default=os.environ.get("ANTHROPIC_API_KEY"))
    p.add_argument("--run-url", default=os.environ.get("CANARY_RUN_URL"))
    p.add_argument("--commit", default=os.environ.get("GITHUB_SHA"))
    p.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", "nearai/ironclaw"),
        help="owner/name slug used for `gh issue` operations",
    )
    p.add_argument(
        "--github-token",
        default=os.environ.get("CANARY_ISSUES_TOKEN")
        or os.environ.get("GH_TOKEN")
        or os.environ.get("GITHUB_TOKEN"),
        help=(
            "GitHub token with `issues: write` for opening canary "
            "failure issues. Off when unset."
        ),
    )
    p.add_argument(
        "--create-issues",
        action="store_true",
        default=os.environ.get("CANARY_CREATE_ISSUES") == "1",
        help=(
            "Open / update one GitHub issue per failed lane. Gated "
            "behind an explicit flag (or CANARY_CREATE_ISSUES=1) "
            "because issue spam is a real risk on the 6h cadence "
            "if a flake recurs."
        ),
    )
    p.add_argument("--dry-run", action="store_true",
                   help="print the Slack payload to stdout instead of posting")
    args = p.parse_args()

    artifacts_root = Path(args.artifacts_dir)
    lane_dirs = discover_lane_dirs(artifacts_root)
    if not lane_dirs:
        print(f"[notify_slack] no lane artifacts under {artifacts_root}", file=sys.stderr)
        return 0

    print(
        f"[notify_slack] discovered {len(lane_dirs)} lane dir(s): "
        f"{', '.join(d.parts[-3] + '/' + d.parts[-2] for d in lane_dirs)}",
        file=sys.stderr,
    )

    reports: list[LaneReport] = []
    for d in lane_dirs:
        r = collect_lane(d)
        if r is not None:
            reports.append(r)
            print(
                f"[notify_slack]   {r.lane}/{r.provider}: "
                f"tests={r.tests} passed={r.passed} failed={r.failed} "
                f"skipped={r.skipped} status={r.status}",
                file=sys.stderr,
            )

    haiku_enriched = 0
    if args.anthropic_api_key and reports:
        for r in reports:
            run_haiku(args.anthropic_api_key, r)
            # run_haiku stamps `notable` with the failure reason on
            # network/JSON errors; treat anything starting with
            # "haiku " as a failed enrichment for accounting.
            if not r.notable.startswith("haiku "):
                haiku_enriched += 1
        print(
            f"[notify_slack] haiku enriched {haiku_enriched}/{len(reports)} lane(s)",
            file=sys.stderr,
        )
    else:
        print("[notify_slack] no ANTHROPIC_API_KEY — skipping haiku enrichment",
              file=sys.stderr)

    # Second-pass categorization across all failed lanes — only fires
    # when there are 2+ failures since one failure is already obvious
    # from its own block.
    category_summary = ""
    if args.anthropic_api_key:
        category_summary = categorize_failures(args.anthropic_api_key, reports)
        if category_summary:
            print(
                "[notify_slack] generated cross-lane category summary",
                file=sys.stderr,
            )

    payload = slack_payload(
        reports, args.run_url, args.commit, category_summary=category_summary
    )

    if args.dry_run or not args.slack_webhook:
        print(json.dumps(payload, indent=2))
        # Dry-run still surfaces what the issue creator WOULD do so a
        # local invocation can sanity-check title/body shapes.
        if args.create_issues and args.github_token:
            failed = [r for r in reports if r.status == "fail"]
            print(
                f"[notify_slack] (dry-run) would open / update "
                f"{len(failed)} issue(s) on {args.repo}",
                file=sys.stderr,
            )
        return 0

    try:
        post_json(args.slack_webhook, payload, {}, timeout=10)
        print(
            f"[notify_slack] posted Slack message for {len(reports)} lane(s)",
            file=sys.stderr,
        )
    except Exception as e:
        print(f"[notify_slack] slack post failed: {e} — sending fallback", file=sys.stderr)
        try:
            post_json(args.slack_webhook, fallback_payload(reports, args.run_url), {}, timeout=10)
            print("[notify_slack] fallback posted", file=sys.stderr)
        except Exception as e2:
            print(f"[notify_slack] fallback also failed: {e2}", file=sys.stderr)

    # Issue creation runs AFTER Slack so a Slack-side failure doesn't
    # block the GitHub-side bookkeeping (and vice versa).
    if args.create_issues and args.github_token:
        urls = create_canary_issues(
            reports,
            repo=args.repo,
            github_token=args.github_token,
            run_url=args.run_url,
            commit=args.commit,
        )
        if urls:
            print(
                f"[notify_slack] created/updated {len(urls)} canary issue(s):"
                f" {', '.join(urls)}",
                file=sys.stderr,
            )
    elif args.create_issues:
        print(
            "[notify_slack] --create-issues set but no GITHUB_TOKEN / "
            "GH_TOKEN / CANARY_ISSUES_TOKEN — skipping issue creation",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
