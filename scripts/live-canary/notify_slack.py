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
    "Do not include prose outside the JSON. If the log is empty or ambiguous, "
    "still produce the object with best-effort fields."
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


def collect_lane(lane_dir: Path) -> LaneReport | None:
    parts = lane_dir.parts
    if len(parts) < 3:
        return None
    lane = parts[-3]
    provider = parts[-2]
    r = LaneReport(lane=lane, provider=provider)
    parse_junit(lane_dir / "auth-canary-junit.xml", r)
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
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read().decode("utf-8", errors="replace")
        if resp.status >= 300:
            raise RuntimeError(f"HTTP {resp.status}: {raw[:200]}")
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
    if text.startswith("```"):
        text = text.strip("`")
        if text.lower().startswith("json"):
            text = text[4:].strip()
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        report.notable = f"haiku returned non-JSON: {text[:160]}"
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


def slack_payload(reports: list[LaneReport], run_url: str | None, commit: str | None) -> dict:
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
        if r.reason:
            lines.append(f"> {r.reason}")
        if r.tools_used:
            lines.append(f"tools: {', '.join(r.tools_used)} (≈{r.tool_calls_total} calls)")
        if r.notable:
            lines.append(f"_{r.notable}_")
        blocks.append({"type": "section", "text": {"type": "mrkdwn", "text": "\n".join(lines)}})
    ctx: list[str] = []
    if commit:
        ctx.append(f"commit `{commit[:7]}`")
    if run_url:
        ctx.append(f"<{run_url}|GitHub run>")
    if ctx:
        blocks.append({"type": "context", "elements": [{"type": "mrkdwn", "text": " • ".join(ctx)}]})
    return {"blocks": blocks}


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
    p.add_argument("--dry-run", action="store_true",
                   help="print the Slack payload to stdout instead of posting")
    args = p.parse_args()

    artifacts_root = Path(args.artifacts_dir)
    lane_dirs = discover_lane_dirs(artifacts_root)
    if not lane_dirs:
        print(f"[notify_slack] no lane artifacts under {artifacts_root}", file=sys.stderr)
        return 0

    reports: list[LaneReport] = []
    for d in lane_dirs:
        r = collect_lane(d)
        if r is not None:
            reports.append(r)

    if args.anthropic_api_key and reports:
        for r in reports:
            run_haiku(args.anthropic_api_key, r)
    else:
        print("[notify_slack] no ANTHROPIC_API_KEY — skipping haiku enrichment",
              file=sys.stderr)

    payload = slack_payload(reports, args.run_url, args.commit)

    if args.dry_run or not args.slack_webhook:
        print(json.dumps(payload, indent=2))
        return 0

    try:
        post_json(args.slack_webhook, payload, {}, timeout=10)
    except Exception as e:
        print(f"[notify_slack] slack post failed: {e} — sending fallback", file=sys.stderr)
        try:
            post_json(args.slack_webhook, fallback_payload(reports, args.run_url), {}, timeout=10)
        except Exception as e2:
            print(f"[notify_slack] fallback also failed: {e2}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
