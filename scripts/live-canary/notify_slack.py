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
SLACK_MAX_BLOCKS = 50
GITHUB_COMMENT_BODY_LIMIT = 65_000
GITHUB_MD_MENTION_BREAK = "@\u200b"

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
class RebornQaCaseReport:
    rows: tuple[str, ...]
    case: str
    feature: str
    success: bool
    latency_ms: int | float | None = None
    message: str = ""
    tool_calls: list["RebornQaToolCall"] = field(default_factory=list)
    debug_paths: list[str] = field(default_factory=list)


@dataclass
class RebornQaToolCall:
    name: str
    args_hash: str = ""
    output_digest: str = ""


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
    reborn_qa_cases: list[RebornQaCaseReport] = field(default_factory=list)


@dataclass(frozen=True)
class CanaryRunContext:
    repository: str = ""
    trigger_context: str = ""
    target_pr: str = ""
    target_branch: str = ""
    target_ref: str = ""


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


def _trim_slack_text(value: object, limit: int = 180) -> str:
    text = str(value or "").strip()
    text = re.sub(r"\s+", " ", text)
    if len(text) <= limit:
        return text
    return text[: max(0, limit - 1)].rstrip() + "…"


def _trim_slack_block_text(value: object, limit: int = 2900) -> str:
    text = str(value or "").strip()
    text = "\n".join(re.sub(r"[ \t]+", " ", line).strip() for line in text.splitlines())
    if len(text) <= limit:
        return text
    return text[: max(0, limit - 1)].rstrip() + "…"


def _reborn_case_from_result(entry: dict) -> str:
    details = entry.get("details") or {}
    if isinstance(details, dict):
        case = details.get("case")
        if isinstance(case, str) and case:
            return case
    mode = entry.get("mode")
    if isinstance(mode, str) and mode.startswith("live:"):
        return mode.removeprefix("live:")
    return str(mode or "?")


def _reborn_failure_message(entry: dict) -> str:
    if entry.get("success"):
        return ""
    details = entry.get("details") or {}
    if not isinstance(details, dict):
        return ""
    for key in ("error", "message", "reason", "gate"):
        value = details.get(key)
        if value:
            return _trim_slack_text(value, 360)
    blocked = details.get("blocked")
    if blocked:
        return _trim_slack_text(f"blocked={blocked}", 360)
    return ""


def _decode_payload_hex(value: object) -> dict:
    if not isinstance(value, str) or not value:
        return {}
    try:
        decoded = bytes.fromhex(value).decode("utf-8")
        payload = json.loads(decoded)
    except (ValueError, UnicodeDecodeError, json.JSONDecodeError):
        return {}
    return payload if isinstance(payload, dict) else {}


def _signature_key(signature: object) -> tuple[str, str] | None:
    if not isinstance(signature, dict):
        return None
    name = signature.get("name")
    if not name:
        return None
    args_hash = signature.get("args_hash")
    return (str(name), str(args_hash or ""))


def parse_reborn_trace_tool_calls(trace_path: Path) -> list[RebornQaToolCall]:
    """Read a scrubbed Reborn trace and return hashed tool I/O summaries.

    Trace payloads can contain live integration data. The checkpoint state
    exposes stable argument hashes and output digests, which are enough for
    correlating "what tool ran with which input/output identity" without
    posting raw Gmail, Slack, Docs, Sheets, or web content into Slack.
    """
    if not trace_path.exists() or trace_path.stat().st_size == 0:
        return []
    try:
        data = json.loads(trace_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return []
    entries = data.get("entries") or []
    if not isinstance(entries, list):
        return []

    best_signatures: list[dict] = []
    best_outputs: list[dict] = []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        contents = entry.get("contents") or {}
        if not isinstance(contents, dict):
            continue
        payload = _decode_payload_hex(contents.get("payload_hex"))
        signatures = payload.get("recent_call_signatures", {}).get("items", [])
        outputs = payload.get("seen_capability_output_digests", {}).get("items", [])
        if isinstance(signatures, list) and len(signatures) > len(best_signatures):
            best_signatures = [item for item in signatures if isinstance(item, dict)]
        if isinstance(outputs, list) and len(outputs) > len(best_outputs):
            best_outputs = [item for item in outputs if isinstance(item, dict)]

    output_by_signature: dict[tuple[str, str], str] = {}
    for item in best_outputs:
        key = _signature_key(item.get("signature"))
        if key is None:
            continue
        output = item.get("output_digest")
        if output is not None:
            output_by_signature[key] = str(output)

    tool_calls: list[RebornQaToolCall] = []
    for signature in best_signatures:
        key = _signature_key(signature)
        if key is None:
            continue
        name, args_hash = key
        tool_calls.append(
            RebornQaToolCall(
                name=name,
                args_hash=args_hash,
                output_digest=output_by_signature.get(key, ""),
            )
        )
    return tool_calls


def parse_reborn_qa_case_reports(lane_dir: Path, report: LaneReport) -> None:
    if report.lane != "reborn-webui-v2-live-qa":
        return
    results_path = lane_dir / "results.json"
    if not results_path.exists() or results_path.stat().st_size == 0:
        return
    try:
        results_data = json.loads(results_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return
    results = results_data.get("results") or []
    if not isinstance(results, list):
        return

    manifest_by_case: dict[str, dict] = {}
    manifest_path = lane_dir / "case-manifest.json"
    if manifest_path.exists() and manifest_path.stat().st_size > 0:
        try:
            manifest_data = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            manifest_data = {}
        for case_data in manifest_data.get("cases") or []:
            if isinstance(case_data, dict) and isinstance(case_data.get("case"), str):
                manifest_by_case[case_data["case"]] = case_data

    cases: list[RebornQaCaseReport] = []
    artifact_path_prefix = "/".join(lane_dir.parts[-3:])
    for entry in results:
        if not isinstance(entry, dict):
            continue
        case = _reborn_case_from_result(entry)
        details = entry.get("details") or {}
        if not isinstance(details, dict):
            details = {}
        manifest = manifest_by_case.get(case, {})
        rows = (
            details.get("qa_rows")
            or details.get("rows")
            or manifest.get("qa_rows")
            or manifest.get("rows")
            or []
        )
        if isinstance(rows, str):
            row_tuple = (rows,)
        elif isinstance(rows, list):
            row_tuple = tuple(str(row) for row in rows if row)
        else:
            row_tuple = ()
        feature = (
            details.get("feature")
            or manifest.get("feature")
            or case.replace("_", " ")
        )
        latency = entry.get("latency_ms")
        tool_calls = parse_reborn_trace_tool_calls(lane_dir / "traces" / f"{case}.json")
        debug_paths = []
        if not entry.get("success"):
            debug_paths = [
                f"{artifact_path_prefix}/results.json",
                f"{artifact_path_prefix}/test-output.log",
                f"{artifact_path_prefix}/traces/{case}.json",
                f"{artifact_path_prefix}/traces/index.json",
            ]
        cases.append(
            RebornQaCaseReport(
                rows=row_tuple,
                case=case,
                feature=_trim_slack_text(feature, 120),
                success=bool(entry.get("success")),
                latency_ms=latency if isinstance(latency, (int, float)) else None,
                message=_reborn_failure_message(entry),
                tool_calls=tool_calls,
                debug_paths=debug_paths,
            )
        )
    report.reborn_qa_cases = cases


SUMMARY_STATUS_RE = re.compile(
    r"^\|\s*Status\s*\|\s*`(?P<status>-?\d+)`\s*\|\s*$", re.MULTILINE
)


def parse_summary_status(summary_md: str) -> int | None:
    """Extract the `| Status | \\`N\\` |` row from a lane summary.md.

    Returns the integer exit code or None if the file doesn't carry that
    row. Used as a last-resort fallback for summary-only lanes
    (private-oauth) and for any future lane whose results.json is
    deleted by strict scrub before upload.
    """
    if not summary_md:
        return None
    m = SUMMARY_STATUS_RE.search(summary_md)
    if not m:
        return None
    try:
        return int(m.group("status"))
    except ValueError:
        return None


def collect_lane(lane_dir: Path) -> LaneReport | None:
    parts = lane_dir.parts
    if len(parts) < 3:
        return None
    lane = parts[-3]
    provider = parts[-2]
    r = LaneReport(lane=lane, provider=provider)
    # Auth-canary lanes write JUnit XML; workflow-canary writes its own
    # results.json; cargo lanes get a scraped results.json from
    # emit_results_json.py. Read whichever exists — all three populate
    # the same LaneReport fields so downstream rendering / Haiku
    # enrichment is source-agnostic.
    parse_junit(lane_dir / "auth-canary-junit.xml", r)
    parse_results_json(lane_dir / "results.json", r)
    parse_reborn_qa_case_reports(lane_dir, r)
    r.summary_md = read_tail(lane_dir / "summary.md", 4_000)
    r.log_tail = read_tail(lane_dir / "test-output.log", MAX_LOG_BYTES)

    if r.failed > 0:
        r.status = "fail"
    elif r.tests > 0:
        r.status = "pass"
    else:
        # No structured counts. Fall back to the lane's exit code from
        # summary.md so summary-only lanes (private-oauth) and any lane
        # whose results.json got stripped by strict scrub still show
        # up as pass/fail instead of misleading "skip".
        summary_status = parse_summary_status(r.summary_md)
        if summary_status is not None:
            r.status = "pass" if summary_status == 0 else "fail"
            if summary_status != 0 and not r.reason:
                r.reason = f"lane exited with status {summary_status}"
        elif r.log_tail:
            r.status = "unknown"
        else:
            r.status = "skip"
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


QA_ROW_PREFIX_RE = re.compile(r"^(?P<num>\d+)")


def _qa_case_rows(case: RebornQaCaseReport) -> str:
    return ", ".join(case.rows) if case.rows else case.case


def _qa_group_key(case: RebornQaCaseReport) -> str:
    for row in case.rows:
        match = QA_ROW_PREFIX_RE.match(row)
        if match:
            return match.group("num")
    return _qa_case_rows(case)


def _qa_group_sort_key(value: str) -> tuple[int, int | str]:
    if value.isdigit():
        return (0, int(value))
    return (1, value)


def _format_reborn_tool_summary(cases: list[RebornQaCaseReport]) -> list[str]:
    calls: list[RebornQaToolCall] = []
    for case in cases:
        calls.extend(case.tool_calls)
    if not calls:
        return []

    distinct_tools = list(dict.fromkeys(call.name for call in calls))
    tool_names = ", ".join(f"`{name}`" for name in distinct_tools[:8])
    if len(distinct_tools) > 8:
        tool_names += f", +{len(distinct_tools) - 8} more"

    lines = [f"*Tools:* {len(calls)} calls across {len(distinct_tools)} tools: {tool_names}"]
    return lines


def _format_reborn_failure_lines(
    cases: list[RebornQaCaseReport],
    run_url: str | None,
) -> list[str]:
    lines: list[str] = []
    for case in cases:
        if case.success:
            continue
        rows = _qa_case_rows(case)
        message = case.message or "failed"
        lines.append(f"*Failure `{rows}`:* {message}")
        if case.debug_paths:
            paths = ", ".join(f"`{path}`" for path in case.debug_paths)
            if run_url:
                lines.append(f"*Debug `{rows}`:* <{run_url}|GitHub run artifacts> → {paths}")
            else:
                lines.append(f"*Debug `{rows}`:* artifacts → {paths}")
    return lines


def _format_reborn_qa_group(
    group: str,
    cases: list[RebornQaCaseReport],
    run_url: str | None = None,
) -> list[dict]:
    passed = sum(1 for case in cases if case.success)
    failed = len(cases) - passed
    status = ":white_check_mark:" if failed == 0 else ":x:"
    duration_ms = sum(
        case.latency_ms for case in cases if isinstance(case.latency_ms, (int, float))
    )
    duration = f" in {duration_ms / 1000.0:.1f}s" if duration_ms else ""

    case_summaries = [
        f"`{_qa_case_rows(case)}` {case.feature}"
        for case in cases
    ]
    lines = [
        f"{status} *QA {group}* — {passed}/{len(cases)} passed{duration}",
        f"*Cases:* {_trim_slack_text('; '.join(case_summaries), 900)}",
    ]
    lines.extend(_format_reborn_failure_lines(cases, run_url))
    lines.extend(_format_reborn_tool_summary(cases))
    blocks: list[dict] = []
    current: list[str] = []
    continuation_header = f"{status} *QA {group}* — continued"
    for line in lines:
        candidate = "\n".join([*current, line])
        if current and len(candidate) > 2900:
            blocks.append(
                {
                    "type": "section",
                    "text": {
                        "type": "mrkdwn",
                        "text": _trim_slack_block_text("\n".join(current), 2900),
                    },
                }
            )
            current = [continuation_header, line]
        else:
            current.append(line)
    if current:
        blocks.append(
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": _trim_slack_block_text("\n".join(current), 2900),
                },
            }
        )
    return blocks


def _format_reborn_qa_groups(
    cases: list[RebornQaCaseReport],
    run_url: str | None = None,
) -> list[dict]:
    grouped: dict[str, list[RebornQaCaseReport]] = {}
    for case in cases:
        grouped.setdefault(_qa_group_key(case), []).append(case)
    blocks: list[dict] = []
    for group in sorted(grouped, key=_qa_group_sort_key):
        blocks.extend(_format_reborn_qa_group(group, grouped[group], run_url))
    return blocks


def _format_run_context(context: CanaryRunContext | None) -> str:
    if context is None:
        return ""
    parts: list[str] = []
    if context.target_pr:
        if context.repository:
            parts.append(
                f"PR <https://github.com/{context.repository}/pull/"
                f"{context.target_pr}|#{context.target_pr}>"
            )
        else:
            parts.append(f"PR #{context.target_pr}")
    if context.target_branch:
        parts.append(f"branch `{_trim_slack_text(context.target_branch, 120)}`")
    if context.target_ref:
        parts.append(f"target `{_trim_slack_text(context.target_ref, 40)[:10]}`")
    if context.trigger_context:
        parts.append(_trim_slack_text(context.trigger_context, 160))
    return " • ".join(parts)


def _github_md_text(value: object, limit: int | None = None) -> str:
    text = _trim_slack_text(value, limit) if limit else str(value or "").strip()
    return (
        text.replace("\\", "\\\\")
        .replace("\n", " ")
        .replace("@", GITHUB_MD_MENTION_BREAK)
        .replace("|", "\\|")
        .replace("`", "\\`")
        .replace("[", "\\[")
        .replace("]", "\\]")
    )


def _github_md_code(value: object, limit: int | None = None) -> str:
    text = _trim_slack_text(value, limit) if limit else str(value or "").strip()
    return text.replace("`", "'").replace("\n", " ").replace("|", "\\|")


def slack_payload(
    reports: list[LaneReport],
    run_url: str | None,
    commit: str | None,
    *,
    category_summary: str = "",
    run_context: CanaryRunContext | None = None,
) -> dict:
    emoji = {"pass": ":white_check_mark:", "fail": ":x:", "skip": ":heavy_minus_sign:"}
    red = sum(1 for r in reports if r.status == "fail")
    green = sum(1 for r in reports if r.status == "pass")
    header = f"Canary: {green} passed, {red} failed of {len(reports)} lanes"
    blocks: list[dict] = [
        {"type": "header", "text": {"type": "plain_text", "text": header}},
    ]
    context_text = _format_run_context(run_context)
    if context_text:
        blocks.append(
            {"type": "context", "elements": [{"type": "mrkdwn", "text": context_text}]}
        )
    for r in reports:
        renders_reborn_qa_groups = bool(r.reborn_qa_cases)
        header_line = (
            f"{emoji.get(r.status, ':grey_question:')} *{r.lane}* ({r.provider}) — "
            f"{r.passed}/{r.tests} passed, {r.failed} failed in {r.duration_s:.0f}s"
        )
        lines = [header_line]
        # Rich failure block: shown when Haiku populated the diagnostic
        # fields. The shape mirrors the issue-friendly format reviewers
        # asked for so a Slack reader can paste it straight into a
        # GitHub issue if needed.
        if (
            r.status == "fail"
            and not renders_reborn_qa_groups
            and (r.test_name or r.error or r.root_cause)
        ):
            if r.test_name:
                lines.append(f"  *Test:* `{r.test_name}`")
            if r.error:
                lines.append(f"  *Error:* {r.error}")
            if r.root_cause:
                lines.append(f"  *Root Cause:* {r.root_cause}")
            if r.fix:
                lines.append(f"  *Fix:* {r.fix}")
        elif r.reason and not renders_reborn_qa_groups:
            # For passing/skipped lanes we keep the existing single-
            # line reason summary (Haiku's free-form notable).
            lines.append(f"> {r.reason}")
        if r.tools_used and not renders_reborn_qa_groups:
            lines.append(f"tools: {', '.join(r.tools_used)} (≈{r.tool_calls_total} calls)")
        if r.notable and not renders_reborn_qa_groups:
            lines.append(f"_{r.notable}_")
        blocks.append({"type": "section", "text": {"type": "mrkdwn", "text": "\n".join(lines)}})
        if renders_reborn_qa_groups:
            remaining = max(0, SLACK_MAX_BLOCKS - len(blocks))
            blocks.extend(_format_reborn_qa_groups(r.reborn_qa_cases, run_url)[:remaining])

    # Cross-lane "Summary by Category" block — only emitted when there
    # are >=2 failures (with 1 the per-lane block is already enough).
    if category_summary and len(blocks) + 2 <= SLACK_MAX_BLOCKS:
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
    if run_context and run_context.target_ref:
        ctx.append(f"commit `{run_context.target_ref[:7]}`")
    elif commit:
        ctx.append(f"commit `{commit[:7]}`")
    if run_url:
        ctx.append(f"<{run_url}|GitHub run>")
    if ctx and len(blocks) < SLACK_MAX_BLOCKS:
        blocks.append({"type": "context", "elements": [{"type": "mrkdwn", "text": " • ".join(ctx)}]})
    return {"blocks": blocks}


def _markdown_run_context_lines(
    run_context: CanaryRunContext | None,
    run_url: str | None,
    commit: str | None,
) -> list[str]:
    lines: list[str] = []
    if run_context and run_context.target_pr:
        if run_context.repository:
            lines.append(
                f"- PR: [#{run_context.target_pr}]"
                f"(https://github.com/{run_context.repository}/pull/"
                f"{run_context.target_pr})"
            )
        else:
            lines.append(f"- PR: #{run_context.target_pr}")
    if run_context and run_context.target_branch:
        lines.append(f"- Branch: `{_github_md_code(run_context.target_branch, 120)}`")
    if run_context and run_context.target_ref:
        lines.append(f"- Target: `{_github_md_code(run_context.target_ref[:10])}`")
    if run_context and run_context.trigger_context:
        lines.append(f"- Trigger: {_github_md_text(run_context.trigger_context, 200)}")
    display_commit = (
        run_context.target_ref
        if run_context and run_context.target_ref
        else commit
    )
    if display_commit:
        lines.append(f"- Commit: `{_github_md_code(display_commit[:7])}`")
    if run_url:
        lines.append(f"- Run: [GitHub Actions]({run_url})")
    return lines


def _markdown_reborn_case_lines(
    cases: list[RebornQaCaseReport],
    run_url: str | None,
) -> list[str]:
    lines: list[str] = []
    grouped: dict[str, list[RebornQaCaseReport]] = {}
    for case in cases:
        grouped.setdefault(_qa_group_key(case), []).append(case)
    for group in sorted(grouped, key=_qa_group_sort_key):
        group_cases = grouped[group]
        passed = sum(1 for case in group_cases if case.success)
        lines.append(f"#### QA {group}: {passed}/{len(group_cases)} passed")
        for case in group_cases:
            status = ":white_check_mark:" if case.success else ":x:"
            latency = (
                f" ({case.latency_ms / 1000.0:.1f}s)"
                if isinstance(case.latency_ms, (int, float))
                else ""
            )
            lines.append(
                f"- {status} `{_github_md_code(_qa_case_rows(case))}` "
                f"{_github_md_text(case.feature)}{latency}"
            )
            if not case.success and case.message:
                lines.append(f"  - Failure: {_github_md_text(case.message)}")
            if not case.success and case.debug_paths:
                paths = ", ".join(f"`{_github_md_code(path)}`" for path in case.debug_paths)
                if run_url:
                    lines.append(
                        f"  - Debug: [GitHub run artifacts]({run_url}) - {paths}"
                    )
                else:
                    lines.append(f"  - Debug: {paths}")
        lines.append("")
    return lines


def github_comment_body(
    reports: list[LaneReport],
    run_url: str | None,
    commit: str | None,
    *,
    category_summary: str = "",
    run_context: CanaryRunContext | None = None,
) -> str:
    red = sum(1 for r in reports if r.status == "fail")
    green = sum(1 for r in reports if r.status == "pass")
    lines = [
        f"## Live canary result: {green} passed, {red} failed of {len(reports)} lanes",
        "",
    ]
    context_lines = _markdown_run_context_lines(run_context, run_url, commit)
    if context_lines:
        lines.extend(context_lines)
        lines.append("")

    lines.extend(
        [
            "| Lane | Provider | Status | Passed | Failed | Duration |",
            "| --- | --- | --- | ---: | ---: | ---: |",
        ]
    )
    for r in reports:
        status = {
            "pass": ":white_check_mark:",
            "fail": ":x:",
            "skip": ":heavy_minus_sign:",
        }.get(r.status, ":grey_question:")
        lines.append(
            f"| `{_github_md_code(r.lane)}` | `{_github_md_code(r.provider)}` | "
            f"{status} {r.status} | "
            f"{r.passed}/{r.tests} | {r.failed} | {r.duration_s:.0f}s |"
        )

    if category_summary:
        lines.extend(["", "### Summary by Category", "", _github_md_text(category_summary)])

    detail_lines: list[str] = []
    for r in reports:
        if r.reborn_qa_cases:
            detail_lines.extend(
                [
                    "",
                    f"### {_github_md_text(r.lane)} ({_github_md_text(r.provider)})",
                    "",
                ]
            )
            detail_lines.extend(_markdown_reborn_case_lines(r.reborn_qa_cases, run_url))
        elif r.status == "fail":
            detail_lines.extend(
                ["", f"### {_github_md_text(r.lane)} ({_github_md_text(r.provider)})"]
            )
            if r.test_name:
                detail_lines.append(f"- Test: `{_github_md_code(r.test_name)}`")
            if r.error:
                detail_lines.append(f"- Error: {_github_md_text(r.error)}")
            if r.root_cause:
                detail_lines.append(f"- Root Cause: {_github_md_text(r.root_cause)}")
            if r.fix:
                detail_lines.append(f"- Fix: {_github_md_text(r.fix)}")
            if r.reason:
                detail_lines.append(f"- Reason: {_github_md_text(r.reason)}")
            if r.notable:
                detail_lines.append(f"- Note: {_github_md_text(r.notable)}")
            if r.junit_failures:
                detail_lines.append("- Failures:")
                for name, msg in r.junit_failures[:10]:
                    detail_lines.append(
                        f"  - `{_github_md_code(name)}`: {_github_md_text(msg)}"
                    )
    if detail_lines:
        lines.extend(detail_lines)

    lines.append("")
    lines.append("_Auto-posted by `scripts/live-canary/notify_slack.py`._")
    body = "\n".join(lines)
    if len(body) <= GITHUB_COMMENT_BODY_LIMIT:
        return body
    suffix = "\n\n_Comment truncated because the canary report exceeded GitHub's body limit._"
    return body[: GITHUB_COMMENT_BODY_LIMIT - len(suffix)].rstrip() + suffix


def post_pr_comment(
    reports: list[LaneReport],
    *,
    repo: str,
    github_token: str,
    run_context: CanaryRunContext,
    run_url: str | None,
    commit: str | None,
    category_summary: str = "",
) -> str:
    if not run_context.target_pr:
        return ""
    if not run_context.target_pr.isascii() or not run_context.target_pr.isdigit():
        raise ValueError("target_pr must be a decimal pull request number")
    headers = {
        "Authorization": f"Bearer {github_token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    get_json(
        f"https://api.github.com/repos/{repo}/pulls/{run_context.target_pr}",
        headers,
        timeout=15,
    )
    created = post_json(
        f"https://api.github.com/repos/{repo}/issues/"
        f"{run_context.target_pr}/comments",
        {
            "body": github_comment_body(
                reports,
                run_url,
                commit,
                category_summary=category_summary,
                run_context=run_context,
            )
        },
        headers,
        timeout=15,
    )
    return str(created.get("html_url") or "")


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
        "--post-pr-comment",
        action="store_true",
        default=os.environ.get("CANARY_POST_PR_COMMENT") == "1",
        help=(
            "Post the final canary report as a PR comment when "
            "CANARY_TARGET_PR is set."
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

    run_context = CanaryRunContext(
        repository=args.repo,
        trigger_context=os.environ.get("CANARY_TRIGGER_CONTEXT", ""),
        target_pr=os.environ.get("CANARY_TARGET_PR", ""),
        target_branch=os.environ.get("CANARY_TARGET_BRANCH", ""),
        target_ref=os.environ.get("CANARY_TARGET_REF", ""),
    )
    payload = slack_payload(
        reports,
        args.run_url,
        args.commit,
        category_summary=category_summary,
        run_context=run_context,
    )

    if args.dry_run:
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
        if args.post_pr_comment and run_context.target_pr:
            print("\n--- GitHub PR Comment Body (Dry Run) ---", file=sys.stderr)
            print(
                github_comment_body(
                    reports,
                    args.run_url,
                    args.commit,
                    category_summary=category_summary,
                    run_context=run_context,
                ),
                file=sys.stderr,
            )
        return 0

    if args.slack_webhook:
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
    else:
        print("[notify_slack] no SLACK_WEBHOOK_URL — skipping Slack post", file=sys.stderr)

    if args.post_pr_comment and run_context.target_pr and args.github_token:
        try:
            comment_url = post_pr_comment(
                reports,
                repo=args.repo,
                github_token=args.github_token,
                run_context=run_context,
                run_url=args.run_url,
                commit=args.commit,
                category_summary=category_summary,
            )
            print(
                f"[notify_slack] posted PR canary comment: {comment_url or '?'}",
                file=sys.stderr,
            )
        except Exception as e:
            print(
                f"[notify_slack] PR comment post failed: {type(e).__name__}: {e}",
                file=sys.stderr,
            )
    elif args.post_pr_comment and run_context.target_pr:
        print(
            "[notify_slack] --post-pr-comment set but no GITHUB_TOKEN / "
            "GH_TOKEN / CANARY_ISSUES_TOKEN — skipping PR comment",
            file=sys.stderr,
        )

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
