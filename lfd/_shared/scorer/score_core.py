#!/usr/bin/env python3
"""score_core -- shared LFD scorer.

Modes:
  dev (default)   cases from lfd/<feature>/eval/dev/cases/, contracts from
                  lfd/<feature>/harness/answers.dev.json, outcomes from
                  --outcomes (or $LFD_OUT). Prints aggregate (4 decimals),
                  case count, PASS/FAIL for the <=5 worst cases (ids only).
  --holdout       cases+answers from <state_root>/holdout/<feature>/;
                  prints ONE aggregate number; appends an audit line;
                  max 3 calls per 24h window (4th -> exit 4).
  --probe map.json  applies the probe renaming map to the SEALED contracts
                  internally before matching; prints aggregate + gap vs the
                  last dev score from <state_root>/audit/<feature>.dev-history.jsonl.
  --self-test     runs the selftest suite (fixtures in scorer/selftest/).

lint_core runs first in every scoring mode; ANY violation prints exactly
"VOID: constraint violation" and exits 3 with no other output.

Scoring formula (SCHEMA.md section 4):
  case_score = (sat required weight / total required weight)
             * 0.5^(distinct forbidden matchers observed)
             * (0 if any ordered sequence violated else 1)
  status error + hard_fail_on_error -> 0; status unsupported -> 0
  feature_score = mean(case_score)

Exit codes: 0 ok, 2 broken eval spec, 3 VOID, 4 holdout budget exhausted.
Stdlib only, deterministic. See lfd/_shared/SCHEMA.md.
"""

import argparse
import datetime
import json
import os
import re
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import lint_core  # noqa: E402
import probe_core  # noqa: E402

DEFAULT_STATE_ROOT = lint_core.DEFAULT_STATE_ROOT
HOLDOUT_WINDOW_HOURS = 24
HOLDOUT_MAX_CALLS = 3

SEQUENCED_TYPES = frozenset(
    ["tool", "reply_contains", "reply_regex", "event", "egress", "gate"]
)
UNSEQUENCED_TYPES = frozenset(
    ["state_eq", "state_pred", "leak", "status", "transcript_wer"]
)
KNOWN_TYPES = SEQUENCED_TYPES | UNSEQUENCED_TYPES

_MISSING = object()


class EvalSpecError(Exception):
    """Broken eval (bad contract/config), not a broken agent. Exit 2."""


# ---------------------------------------------------------------------------
# matcher engine
# ---------------------------------------------------------------------------


def _contains(expected, actual):
    """Recursive containment: dicts are subset-matched, everything else ==."""
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            return False
        return all(k in actual and _contains(v, actual[k]) for k, v in expected.items())
    return expected == actual


def _invocation_params(inv):
    pj = inv.get("params_json")
    if pj is None:
        return {}
    try:
        parsed = json.loads(pj)
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _walk_path(value, path):
    if path in (None, ""):
        return value
    for part in str(path).split("."):
        if isinstance(value, dict):
            if part not in value:
                return _MISSING
            value = value[part]
        elif isinstance(value, list):
            try:
                idx = int(part)
            except ValueError:
                return _MISSING
            if not 0 <= idx < len(value):
                return _MISSING
            value = value[idx]
        else:
            return _MISSING
    return value


_WER_STRIP_RE = re.compile(r"[^\w\s]+", re.UNICODE)


def _wer_words(s):
    return _WER_STRIP_RE.sub(" ", s.casefold()).split()


def _levenshtein(a, b):
    if not a:
        return len(b)
    if not b:
        return len(a)
    prev = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        cur = [i]
        for j, y in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x != y)))
        prev = cur
    return prev[-1]


def word_error_rate(ref, hyp):
    """Word-level Levenshtein distance / max(1, len(ref words)),
    case-folded, punctuation stripped."""
    ref_words = _wer_words(ref)
    hyp_words = _wer_words(hyp)
    return _levenshtein(ref_words, hyp_words) / max(1, len(ref_words))


def _numeric(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _eval_state_pred(m, value):
    preds = 0
    ok = True
    if "exists" in m:
        preds += 1
        ok = ok and ((value is not _MISSING) == bool(m["exists"]))
    if "eq" in m:
        preds += 1
        ok = ok and (value is not _MISSING and value == m["eq"])
    if "ne" in m:
        preds += 1
        ok = ok and (value is not _MISSING and value != m["ne"])
    if "contains" in m:
        preds += 1
        needle = m["contains"]
        if value is _MISSING:
            ok = False
        elif isinstance(value, str):
            ok = ok and isinstance(needle, str) and needle in value
        elif isinstance(value, (list, dict)):
            ok = ok and needle in value
        else:
            ok = False
    for key, cmp in (("gt", lambda a, b: a > b), ("gte", lambda a, b: a >= b),
                     ("lt", lambda a, b: a < b), ("lte", lambda a, b: a <= b)):
        if key in m:
            preds += 1
            if value is _MISSING or not _numeric(value) or not _numeric(m[key]):
                ok = False
            else:
                ok = ok and cmp(value, m[key])
    if preds == 0:
        raise EvalSpecError("state_pred matcher has no predicate (eq/ne/contains/exists/gt/gte/lt/lte)")
    return ok


def eval_matcher(m, outcome):
    """Returns (satisfied, seqs). `seqs` lists the outcome seq numbers of the
    matching occurrences for sequenced matcher types (empty otherwise)."""
    mtype = m.get("type")
    if mtype not in KNOWN_TYPES:
        raise EvalSpecError("unknown matcher type: %r" % (mtype,))
    seqs = []

    if mtype == "tool":
        name = m.get("name")
        if not name:
            raise EvalSpecError("tool matcher missing 'name'")
        pc = m.get("params_contains")
        for inv in outcome.get("tool_invocations") or []:
            if inv.get("name") != name:
                continue
            if pc is not None and not _contains(pc, _invocation_params(inv)):
                continue
            if "ok" in m and inv.get("ok") != m["ok"]:
                continue
            seqs.append(inv.get("seq"))

    elif mtype == "reply_contains":
        subs = m.get("substrings_any")
        if not subs or not isinstance(subs, list):
            raise EvalSpecError("reply_contains matcher missing 'substrings_any'")
        for rep in outcome.get("replies") or []:
            if "channel" in m and rep.get("channel") != m["channel"]:
                continue
            text = rep.get("text") or ""
            if any(s in text for s in subs):
                seqs.append(rep.get("seq"))

    elif mtype == "reply_regex":
        pattern = m.get("pattern", m.get("regex"))
        if not pattern:
            raise EvalSpecError("reply_regex matcher missing 'pattern'")
        try:
            cre = re.compile(pattern)
        except re.error as e:
            raise EvalSpecError("reply_regex invalid pattern %r: %s" % (pattern, e))
        for rep in outcome.get("replies") or []:
            if "channel" in m and rep.get("channel") != m["channel"]:
                continue
            if cre.search(rep.get("text") or ""):
                seqs.append(rep.get("seq"))

    elif mtype == "event":
        kinds = m.get("kind_any")
        if kinds is None and "kind" in m:
            kinds = [m["kind"]]
        if not kinds:
            raise EvalSpecError("event matcher missing 'kind_any'")
        for ev in outcome.get("events") or []:
            if ev.get("kind") in kinds:
                seqs.append(ev.get("seq"))

    elif mtype == "egress":
        if "method" not in m and "url_contains" not in m:
            raise EvalSpecError("egress matcher needs 'method' and/or 'url_contains'")
        for eg in outcome.get("egress") or []:
            if "method" in m and str(eg.get("method") or "").upper() != str(m["method"]).upper():
                continue
            if "url_contains" in m and m["url_contains"] not in (eg.get("url") or ""):
                continue
            seqs.append(eg.get("seq"))

    elif mtype == "gate":
        for g in outcome.get("gates") or []:
            if "kind" in m and g.get("kind") != m["kind"]:
                continue
            if "resolution" in m and g.get("resolution") != m["resolution"]:
                continue
            seqs.append(g.get("seq"))

    elif mtype == "state_eq":
        if "query" not in m or "eq" not in m:
            raise EvalSpecError("state_eq matcher needs 'query' and 'eq'")
        state = outcome.get("state") or {}
        return (m["query"] in state and state[m["query"]] == m["eq"], [])

    elif mtype == "state_pred":
        if "query" not in m:
            raise EvalSpecError("state_pred matcher missing 'query'")
        state = outcome.get("state") or {}
        base = state.get(m["query"], _MISSING) if m["query"] in state else _MISSING
        value = _walk_path(base, m.get("path", "")) if base is not _MISSING else _MISSING
        return (_eval_state_pred(m, value), [])

    elif mtype == "leak":
        hits = (outcome.get("leaks") or {}).get("secret_scan_hits", 0)
        return (_numeric(hits) and hits > 0, [])

    elif mtype == "status":
        if "eq" not in m:
            raise EvalSpecError("status matcher missing 'eq'")
        return (outcome.get("status") == m["eq"], [])

    elif mtype == "transcript_wer":
        if "query" not in m or "ref" not in m or "max" not in m:
            raise EvalSpecError("transcript_wer matcher needs 'query', 'ref' and 'max'")
        if not isinstance(m["ref"], str) or not _numeric(m["max"]):
            raise EvalSpecError("transcript_wer 'ref' must be a string and 'max' numeric")
        state = outcome.get("state") or {}
        value = state.get(m["query"], _MISSING) if m["query"] in state else _MISSING
        if isinstance(value, dict):
            value = value.get("text", _MISSING)
        if value is _MISSING or not isinstance(value, str):
            return (False, [])
        return (word_error_rate(m["ref"], value) <= m["max"], [])

    return (len(seqs) > 0, seqs)


# ---------------------------------------------------------------------------
# case + set scoring
# ---------------------------------------------------------------------------


def _validate_contract(contract):
    """Structure checks that must fire even when the outcome short-circuits.
    Returns {matcher_id: matcher} for ordered resolution."""
    by_id = {}
    for m in contract.get("required") or []:
        mtype = m.get("type")
        if mtype not in KNOWN_TYPES:
            raise EvalSpecError("unknown matcher type: %r" % (mtype,))
        if mtype == "leak":
            raise EvalSpecError("leak matcher is forbidden-only, found in required")
        mid = m.get("id")
        if mid is not None:
            if mid in by_id:
                raise EvalSpecError("duplicate matcher id %r" % mid)
            by_id[mid] = m
    for m in contract.get("forbidden") or []:
        if m.get("type") not in KNOWN_TYPES:
            raise EvalSpecError("unknown matcher type: %r" % (m.get("type"),))
    for seq_list in contract.get("ordered") or []:
        for mid in seq_list:
            if mid not in by_id:
                raise EvalSpecError("ordered references unknown matcher id %r" % mid)
            if by_id[mid].get("type") not in SEQUENCED_TYPES:
                raise EvalSpecError(
                    "ordered references non-sequenceable matcher %r (type %s)"
                    % (mid, by_id[mid].get("type"))
                )


def score_case(contract, outcome):
    """Score one case per SCHEMA.md section 4. `outcome` may be None
    (missing outcome file -> 0, priced like unsupported)."""
    _validate_contract(contract)
    if outcome is None:
        return 0.0
    status = outcome.get("status")
    if status == "unsupported":
        return 0.0
    if status == "error" and contract.get("hard_fail_on_error", True):
        return 0.0

    total_w = 0.0
    got_w = 0.0
    sat_seqs = {}
    for m in contract.get("required") or []:
        try:
            w = float(m.get("weight", 1))
        except (TypeError, ValueError):
            raise EvalSpecError("non-numeric weight on matcher %r" % m.get("id"))
        total_w += w
        sat, seqs = eval_matcher(m, outcome)
        if sat:
            got_w += w
        if m.get("id") is not None:
            numeric_seqs = sorted(s for s in seqs if _numeric(s))
            sat_seqs[m["id"]] = (sat, numeric_seqs)
    base = (got_w / total_w) if total_w > 0 else 1.0

    forbidden_observed = 0
    for m in contract.get("forbidden") or []:
        sat, _ = eval_matcher(m, outcome)
        if sat:
            forbidden_observed += 1

    order_ok = True
    for seq_list in contract.get("ordered") or []:
        prev = float("-inf")
        for mid in seq_list:
            sat, seqs = sat_seqs[mid]
            if not sat:
                continue  # absence is already priced by the numerator
            nxt = None
            for s in seqs:
                if s > prev:
                    nxt = s
                    break
            if nxt is None:
                order_ok = False
                break
            prev = nxt
        if not order_ok:
            break

    return base * (0.5 ** forbidden_observed) * (1.0 if order_ok else 0.0)


def load_contracts(answers):
    contracts = {}
    for c in answers.get("contracts") or []:
        cid = c.get("case_id")
        if not cid:
            raise EvalSpecError("contract without case_id")
        if cid in contracts:
            raise EvalSpecError("duplicate contract for case %r" % cid)
        contracts[cid] = c
    return contracts


def load_case_ids(cases_dir):
    ids = []
    for _, case in probe_core.load_cases(cases_dir):
        cid = case.get("case_id")
        if not cid:
            raise EvalSpecError("case file without case_id in %s" % cases_dir)
        if cid in ids:
            raise EvalSpecError("duplicate case_id %r" % cid)
        ids.append(cid)
    if not ids:
        raise EvalSpecError("no cases found in %s" % cases_dir)
    return sorted(ids)


def load_outcome(outcomes_dir, case_id):
    path = Path(outcomes_dir) / ("%s.outcome.json" % case_id)
    if not path.is_file():
        return None
    try:
        with open(path, "r", encoding="utf-8") as f:
            outcome = json.load(f)
    except (OSError, ValueError):
        return None  # unreadable outcome is priced as a missing one
    if not isinstance(outcome, dict) or outcome.get("case_id") != case_id:
        return None  # mismatched outcome cannot vouch for this case
    return outcome


def run_scoring(case_ids, contracts, outcomes_dir):
    """Returns (aggregate, {case_id: score}, [outcome meta runner_hash])."""
    per_case = {}
    runner_hashes = []
    for cid in case_ids:
        contract = contracts.get(cid)
        if contract is None:
            raise EvalSpecError("no contract for case %r" % cid)
        outcome = load_outcome(outcomes_dir, cid)
        if outcome is not None:
            rh = (outcome.get("meta") or {}).get("runner_hash")
            if rh is not None:
                runner_hashes.append(rh)
        per_case[cid] = score_case(contract, outcome)
    aggregate = sum(per_case.values()) / len(per_case)
    return aggregate, per_case, runner_hashes


# ---------------------------------------------------------------------------
# history / audit plumbing
# ---------------------------------------------------------------------------


def _now_iso():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def _parse_ts(ts):
    try:
        return datetime.datetime.fromisoformat(str(ts).replace("Z", "+00:00"))
    except ValueError:
        return None


def _append_jsonl(path, record):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(record, sort_keys=True) + "\n")


def _read_jsonl(path):
    path = Path(path)
    if not path.is_file():
        return []
    records = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except ValueError:
                records.append({"_malformed": line})
    return records


def holdout_calls_in_window(audit_path, now=None):
    """Count holdout calls in the last 24h. Malformed lines and lines with
    unparsable timestamps COUNT toward the budget (fail closed)."""
    if now is None:
        now = datetime.datetime.now(datetime.timezone.utc)
    cutoff = now - datetime.timedelta(hours=HOLDOUT_WINDOW_HOURS)
    count = 0
    for rec in _read_jsonl(audit_path):
        ts = _parse_ts(rec.get("ts")) if isinstance(rec, dict) else None
        if ts is None or ts > cutoff:
            count += 1
    return count


def last_dev_score(history_path):
    last = None
    for rec in _read_jsonl(history_path):
        if isinstance(rec, dict) and rec.get("kind", "dev") == "dev" and "score" in rec:
            last = rec
    return last


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _load_answers(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        raise EvalSpecError("answers file not found: %s" % path)
    except (OSError, ValueError) as e:
        raise EvalSpecError("answers file unreadable (%s): %s" % (e, path))


def _resolve_outcomes_dir(args):
    outcomes = args.outcomes or os.environ.get("LFD_OUT")
    if not outcomes:
        raise EvalSpecError("no outcomes directory (--outcomes or $LFD_OUT)")
    d = Path(outcomes)
    if not d.is_dir():
        raise EvalSpecError("outcomes directory not found: %s" % d)
    return d


def _run(args):
    lfd_root = Path(args.lfd_root).resolve()
    repo_root = Path(args.repo_root).resolve() if args.repo_root else lfd_root.parent
    state_root = Path(args.state_root)
    feature = args.feature

    holdout_dir = state_root / "holdout" / feature
    holdout_answers_path = holdout_dir / "answers.holdout.json"

    lint = lint_core.run_lint(
        feature,
        lfd_root,
        repo_root=repo_root,
        state_root=state_root,
        holdout=args.holdout,
        holdout_answers_path=holdout_answers_path if args.holdout else None,
    )
    if lint.violations:
        print("VOID: constraint violation")
        return 3

    history_path = state_root / "audit" / ("%s.dev-history.jsonl" % feature)

    if args.holdout:
        audit_path = state_root / "audit" / ("%s.log" % feature)
        if holdout_calls_in_window(audit_path) >= HOLDOUT_MAX_CALLS:
            print("HOLDOUT BUDGET EXHAUSTED")
            return 4
        case_ids = load_case_ids(holdout_dir / "cases")
        contracts = load_contracts(_load_answers(holdout_answers_path))
        outcomes_dir = _resolve_outcomes_dir(args)
        aggregate, _, runner_hashes = run_scoring(case_ids, contracts, outcomes_dir)
        runner_hash_ok = bool(
            lint.pins_ok
            and lint.runner_hash is not None
            and runner_hashes
            and all(rh == lint.runner_hash for rh in runner_hashes)
        )
        print("%.4f" % aggregate)
        _append_jsonl(
            audit_path,
            {
                "ts": _now_iso(),
                "feature": feature,
                "score": aggregate,
                "n_cases": len(case_ids),
                "runner_hash_ok": runner_hash_ok,
            },
        )
        return 0

    cases_dir = lfd_root / feature / "eval" / "dev" / "cases"
    case_ids = load_case_ids(cases_dir)
    answers = _load_answers(lfd_root / feature / "harness" / "answers.dev.json")

    if args.probe:
        try:
            with open(args.probe, "r", encoding="utf-8") as f:
                map_data = json.load(f)
        except (OSError, ValueError) as e:
            raise EvalSpecError("probe map unreadable (%s): %s" % (e, args.probe))
        answers = probe_core.transform_contracts(answers, map_data)
        contracts = load_contracts(answers)
        outcomes_dir = _resolve_outcomes_dir(args)
        aggregate, _, _ = run_scoring(case_ids, contracts, outcomes_dir)
        print("score: %.4f" % aggregate)
        last = last_dev_score(history_path)
        if last is None:
            print("gap_vs_dev: n/a (no dev history)")
        else:
            print("gap_vs_dev: %+.4f" % (aggregate - float(last["score"])))
        _append_jsonl(
            history_path,
            {
                "ts": _now_iso(),
                "feature": feature,
                "kind": "probe",
                "score": aggregate,
                "n_cases": len(case_ids),
            },
        )
        return 0

    # dev mode
    contracts = load_contracts(answers)
    outcomes_dir = _resolve_outcomes_dir(args)
    aggregate, per_case, _ = run_scoring(case_ids, contracts, outcomes_dir)
    print("score: %.4f" % aggregate)
    print("cases: %d" % len(case_ids))
    worst = sorted(per_case.items(), key=lambda kv: (kv[1], kv[0]))[:5]
    print("worst:")
    for cid, sc in worst:
        print("  %s %s" % (cid, "PASS" if sc >= 1.0 - 1e-9 else "FAIL"))
    _append_jsonl(
        history_path,
        {
            "ts": _now_iso(),
            "feature": feature,
            "kind": "dev",
            "score": aggregate,
            "n_cases": len(case_ids),
        },
    )
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description="LFD shared scorer")
    ap.add_argument("--feature")
    ap.add_argument("--lfd-root")
    ap.add_argument("--outcomes", default=None)
    ap.add_argument("--holdout", action="store_true")
    ap.add_argument("--probe", default=None, metavar="MAP_JSON")
    ap.add_argument("--self-test", action="store_true")
    # harness/test overrides (used by the selftest; safe to leave at defaults)
    ap.add_argument("--repo-root", default=None)
    ap.add_argument("--state-root", default=DEFAULT_STATE_ROOT)
    args = ap.parse_args(argv)

    if args.self_test:
        from selftest import harness

        return harness.run_all()

    if not args.feature or not args.lfd_root:
        print("EVAL SPEC ERROR: --feature and --lfd-root are required", file=sys.stderr)
        return 2
    if args.holdout and args.probe:
        print("EVAL SPEC ERROR: --holdout and --probe are mutually exclusive", file=sys.stderr)
        return 2

    try:
        return _run(args)
    except (EvalSpecError, lint_core.LintConfigError) as e:
        print("EVAL SPEC ERROR: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
