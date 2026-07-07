#!/usr/bin/env python3
"""status_core -- LFD run status for a feature.

Prints: wall-clock elapsed since the first real "RUN START <ISO>" marker in
lfd/<feature>/LOG.md; cycle score history from the dev-history jsonl;
holdout calls used (12h window) out of 3; spend so far vs the caps.json
ceiling; and a one-line gain-per-cycle trend (mean delta of last 3 cycles).
Stdlib only.
"""

import argparse
import datetime
import json
import re
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import score_core  # noqa: E402

RUN_START_RE = re.compile(r"RUN START\s+(\S+)")
TEMPLATE_RUN_START = "2026-01-01T00:00:00Z"


def _fmt_elapsed(delta):
    total_minutes = int(delta.total_seconds() // 60)
    days, rem = divmod(total_minutes, 24 * 60)
    hours, minutes = divmod(rem, 60)
    if days:
        return "%dd %dh %dm" % (days, hours, minutes)
    return "%dh %dm" % (hours, minutes)


def main(argv=None):
    ap = argparse.ArgumentParser(description="LFD feature status")
    ap.add_argument("--feature", required=True)
    ap.add_argument("--lfd-root", required=True)
    ap.add_argument("--state-root", default=score_core.DEFAULT_STATE_ROOT)
    args = ap.parse_args(argv)

    lfd_root = Path(args.lfd_root).resolve()
    state_root = Path(args.state_root)
    feature = args.feature
    now = datetime.datetime.now(datetime.timezone.utc)

    print("feature: %s" % feature)

    # --- elapsed since first RUN START marker -------------------------------
    log_path = lfd_root / feature / "LOG.md"
    start_ts = None
    if log_path.is_file():
        for line in log_path.read_text(encoding="utf-8", errors="ignore").splitlines():
            m = RUN_START_RE.search(line)
            if m:
                token = m.group(1)
                if token != TEMPLATE_RUN_START:
                    start_ts = score_core._parse_ts(token)
                break
    if start_ts is None:
        print("elapsed: n/a (RUN START not set in LOG.md)")
    else:
        if start_ts.tzinfo is None:
            start_ts = start_ts.replace(tzinfo=datetime.timezone.utc)
        print(
            "elapsed: %s (since %s)"
            % (_fmt_elapsed(now - start_ts), start_ts.isoformat(timespec="seconds"))
        )

    # --- cycle score history ------------------------------------------------
    history_path = state_root / "audit" / ("%s.dev-history.jsonl" % feature)
    cycles = [
        rec
        for rec in score_core._read_jsonl(history_path)
        if isinstance(rec, dict) and rec.get("kind", "dev") == "dev" and "score" in rec
    ]
    print("cycles: %d" % len(cycles))
    deltas = []
    prev = None
    for i, rec in enumerate(cycles, 1):
        score = float(rec["score"])
        if prev is None:
            print("  cycle %d: %.4f" % (i, score))
        else:
            delta = score - prev
            deltas.append(delta)
            print("  cycle %d: %.4f (delta %+.4f)" % (i, score, delta))
        prev = score

    # --- holdout budget -------------------------------------------------
    audit_path = state_root / "audit" / ("%s.log" % feature)
    used_window = score_core.holdout_calls_in_window(audit_path, now=now)
    total = len(score_core._read_jsonl(audit_path))
    print(
        "holdout: %d/%d used in last %dh (%d total)"
        % (used_window, score_core.HOLDOUT_MAX_CALLS, score_core.HOLDOUT_WINDOW_HOURS, total)
    )

    # --- spend ----------------------------------------------------------
    spend_path = state_root / "spend" / ("%s.jsonl" % feature)
    spent = 0.0
    have_spend = spend_path.is_file()
    if have_spend:
        for rec in score_core._read_jsonl(spend_path):
            if isinstance(rec, dict):
                try:
                    spent += float(rec.get("usd", 0))
                except (TypeError, ValueError):
                    pass
    ceiling = None
    caps_path = lfd_root / feature / "harness" / "caps.json"
    if caps_path.is_file():
        try:
            with open(caps_path, "r", encoding="utf-8") as f:
                ceiling = json.load(f).get("spend_ceiling_usd")
        except (OSError, ValueError):
            ceiling = None
    ceiling_txt = ("$%.2f" % float(ceiling)) if ceiling is not None else "n/a"
    if have_spend:
        print("spend: $%.2f / %s ceiling" % (spent, ceiling_txt))
    else:
        print("spend: $0.00 (no spend log) / %s ceiling" % ceiling_txt)

    # --- trend ------------------------------------------------------------
    tail = deltas[-3:]
    if tail:
        print(
            "trend: mean delta over last %d cycle(s) = %+.4f"
            % (len(tail), sum(tail) / len(tail))
        )
    else:
        print("trend: n/a (fewer than 2 cycles)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
