#!/usr/bin/env python3
"""IronClaw development metrics — three tiers.

Tier 1  Flow / speed      (DORA-style: lead time, PR size, merge cadence)
Tier 2  Quality / stability (change-failure proxy, rework, test share)
Tier 3  Codebase health    (leading indicators: composition mass, v1 burndown,
                             file sprawl, abstraction density, boundary coverage)

All numbers are derived from git history + the GitHub API (via `gh`) + the
working tree. No external services beyond those. Safe to run read-only.

Usage:
    python3 scripts/dev_metrics.py [--pr-limit N] [--out FILE]

Everything is a *trend* tool: watch direction, not absolutes.
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
from datetime import datetime, timezone, timedelta

# ----------------------------------------------------------------------------
# shell helpers
# ----------------------------------------------------------------------------


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, check=True
    ).stdout


def try_gh(*args: str) -> str | None:
    try:
        r = subprocess.run(
            ["gh", *args],
            capture_output=True,
            text=True,
            check=False,
            timeout=30,  # gh can block on network/auth failures; treat as unavailable
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if r.returncode != 0:
        return None
    return r.stdout


def sh(cmd: str, ok: tuple[int, ...] = (0,)) -> str:
    """Run a bash pipeline with pipefail; warn (never silently zero) on a
    non-ok exit so a failed find/grep/cat surfaces instead of faking a metric."""
    r = subprocess.run(
        ["bash", "-c", f"set -o pipefail; {cmd}"],
        capture_output=True, text=True,
    )
    if r.returncode not in ok:
        print(f"[warn] metric probe rc={r.returncode}: {cmd[:70]}", file=sys.stderr)
    return r.stdout


def pct(values: list[float], p: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    k = (len(s) - 1) * p
    lo, hi = int(k), min(int(k) + 1, len(s) - 1)
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


# ----------------------------------------------------------------------------
# commit ingestion (whole history)
# ----------------------------------------------------------------------------

CONV_TYPES = {
    "feat", "fix", "test", "docs", "refactor", "perf", "chore",
    "ci", "build", "style", "revert",
}
# types that represent *product change* (denominator for change-failure rate)
PRODUCT_CHANGE = {"feat", "fix", "refactor", "perf"}


def classify_commit(subj: str) -> dict:
    """Pure classification of a commit subject (no git) — unit-testable."""
    # strip scope + breaking marker: feat(reborn)! -> feat
    head = subj.split(":", 1)[0] if ":" in subj else ""
    base = head.split("(", 1)[0].rstrip("!").strip().lower()
    typ = base if base in CONV_TYPES else None
    is_pr = "(#" in subj and subj.rstrip().endswith(")")
    # rework signals: explicit revert, or a fix that undoes/reverts a prior PR
    low = subj.lower()
    is_revert = (
        typ == "revert"
        or low.startswith("revert ")
        or "undoes #" in low
        or "reverts #" in low
    )
    return {"type": typ, "is_pr": is_pr, "is_revert": is_revert}


def load_commits() -> list[dict]:
    # unit-separator delimited: hash \x1f iso-date \x1f subject
    raw = git("log", "--no-merges", "--pretty=format:%H\x1f%cI\x1f%s")
    out = []
    for line in raw.splitlines():
        parts = line.split("\x1f")
        if len(parts) != 3:
            continue
        h, iso, subj = parts
        date = datetime.fromisoformat(iso).astimezone(timezone.utc)
        out.append({"hash": h, "date": date, "subj": subj, **classify_commit(subj)})
    return out


def bucket_by_period(commits: list[dict], now: datetime, days: int) -> dict:
    """Group commits into consecutive `days`-wide windows going back in time."""
    buckets: dict[int, list[dict]] = {}
    for c in commits:
        age = (now - c["date"]).days
        if age < 0:
            age = 0
        buckets.setdefault(age // days, []).append(c)
    return buckets


# ----------------------------------------------------------------------------
# Tier 1 — flow / speed
# ----------------------------------------------------------------------------


def tier1(commits: list[dict], now: datetime, pr_limit: int) -> dict:
    res: dict = {}

    # merge cadence from git (squash-merged PRs carry (#N))
    pr_commits = [c for c in commits if c["is_pr"]]
    for label, d in (("7d", 7), ("30d", 30), ("60d", 60), ("90d", 90)):
        n = sum(1 for c in pr_commits if (now - c["date"]).days < d)
        res[f"prs_merged_{label}"] = n
    res["prs_total_history"] = len(pr_commits)

    # lead time + PR size from GitHub (real open->merge)
    gh_json = try_gh(
        "pr", "list", "--state", "merged", "--limit", str(pr_limit),
        "--json", "number,createdAt,mergedAt,additions,deletions,changedFiles",
    )
    if gh_json:
        try:
            prs = json.loads(gh_json)
        except json.JSONDecodeError:
            prs = []  # gh emitted non-JSON (warning/rate-limit) — treat as no sample
        lead_hours, sizes, files = [], [], []
        for p in prs:
            try:
                c = datetime.fromisoformat(p["createdAt"].replace("Z", "+00:00"))
                m = datetime.fromisoformat(p["mergedAt"].replace("Z", "+00:00"))
            except (KeyError, ValueError):
                continue
            lead_hours.append((m - c).total_seconds() / 3600.0)
            sizes.append((p.get("additions") or 0) + (p.get("deletions") or 0))
            files.append(p.get("changedFiles") or 0)
        res["gh_pr_sample"] = len(prs)
        res["lead_time_h_p50"] = round(pct(lead_hours, 0.5), 1)
        res["lead_time_h_p90"] = round(pct(lead_hours, 0.9), 1)
        res["lead_time_h_max"] = round(max(lead_hours), 1) if lead_hours else 0
        res["pr_lines_p50"] = int(pct(sizes, 0.5))
        res["pr_lines_p90"] = int(pct(sizes, 0.9))
        res["pr_files_p50"] = int(pct(files, 0.5))
        res["pr_files_p90"] = int(pct(files, 0.9))
        # small-PR share — a strong healthy-flow signal
        res["pr_small_share"] = round(
            100 * sum(1 for s in sizes if s <= 200) / len(sizes), 1
        ) if sizes else 0.0
    else:
        res["gh_pr_sample"] = None  # gh unavailable; git cadence still valid

    return res


# ----------------------------------------------------------------------------
# Tier 2 — quality / stability
# ----------------------------------------------------------------------------


def tier2(commits: list[dict], now: datetime) -> dict:
    res: dict = {}
    buckets = bucket_by_period(commits, now, 30)
    per_period = []
    for idx in sorted(buckets):
        cs = buckets[idx]
        counts = {t: 0 for t in CONV_TYPES}
        for c in cs:
            if c["type"]:
                counts[c["type"]] += 1
        product = sum(counts[t] for t in PRODUCT_CHANGE)
        reverts = sum(1 for c in cs if c["is_revert"])
        cfr = round(100 * counts["fix"] / product, 1) if product else 0.0
        fix_feat = round(counts["fix"] / counts["feat"], 2) if counts["feat"] else 0.0
        test_share = round(
            100 * counts["test"] / len(cs), 1
        ) if cs else 0.0
        per_period.append({
            "period": f"-{idx*30}..-{(idx+1)*30}d",
            "commits": len(cs),
            "feat": counts["feat"], "fix": counts["fix"],
            "refactor": counts["refactor"], "test": counts["test"],
            "reverts": reverts,
            "change_failure_pct": cfr,     # fix / (feat+fix+refactor+perf)
            "fix_per_feat": fix_feat,
            "test_commit_pct": test_share,
        })
    res["by_period_30d"] = per_period

    # whole-history rollup
    all_counts = {t: 0 for t in CONV_TYPES}
    for c in commits:
        if c["type"]:
            all_counts[c["type"]] += 1
    prod = sum(all_counts[t] for t in PRODUCT_CHANGE)
    res["overall_change_failure_pct"] = round(100 * all_counts["fix"] / prod, 1) if prod else 0.0
    res["overall_fix_per_feat"] = round(all_counts["fix"] / all_counts["feat"], 2) if all_counts["feat"] else 0.0
    res["overall_reverts"] = sum(1 for c in commits if c["is_revert"])
    res["type_totals"] = all_counts
    return res


# ----------------------------------------------------------------------------
# Tier 3 — codebase health
# ----------------------------------------------------------------------------

COMPOSITION = "crates/ironclaw_reborn_composition/src"
V1_SRC = "src"
CRATES_SRC = "crates"


def tree_bytes(commit: str, path: str, suffix: str = ".rs") -> int:
    """Sum of blob sizes for files under `path` at `commit` (one git call)."""
    try:
        out = git("ls-tree", "-r", "-l", commit, "--", path)
    except subprocess.CalledProcessError:
        return 0
    total = 0
    for line in out.splitlines():
        # <mode> <type> <sha> <size>\t<path>
        meta, _, fpath = line.partition("\t")
        if not fpath.endswith(suffix):
            continue
        cols = meta.split()
        if len(cols) >= 4 and cols[3].isdigit():
            total += int(cols[3])
    return total


def wc_lines(path_glob_cmd: list[str]) -> int:
    out = subprocess.run(path_glob_cmd, capture_output=True, text=True).stdout
    return sum(1 for _ in out.splitlines())


def count_matches(pattern: str, path: str) -> int:
    r = subprocess.run(
        ["grep", "-rEho", pattern, path, "--include=*.rs"],
        capture_output=True, text=True,
    )
    # grep: 0 = matches, 1 = no matches (valid 0), >1 = real error.
    if r.returncode > 1:
        print(f"[warn] grep rc={r.returncode} for pattern {pattern[:40]!r}", file=sys.stderr)
    return len([l for l in r.stdout.splitlines() if l.strip()])


def tier3(now: datetime) -> dict:
    res: dict = {}

    # ---- historical size trend (byte-size index, calibrated to current LOC) ----
    # sample the last commit before each month boundary, back to history start.
    samples = []
    for months_ago in range(0, 7):
        target = now - timedelta(days=30 * months_ago)
        sha = git("rev-list", "-1", f"--before={target.date().isoformat()}", "HEAD").strip()
        if not sha:
            continue
        cdate = git("show", "-s", "--format=%cd", "--date=short", sha).strip()
        samples.append({
            "months_ago": months_ago,
            "date": cdate,
            "composition_bytes": tree_bytes(sha, COMPOSITION),
            "v1_src_bytes": tree_bytes(sha, V1_SRC),
            "crates_bytes": tree_bytes(sha, CRATES_SRC),
        })
    samples.sort(key=lambda s: s["date"])

    # calibrate bytes->LOC using the current working tree (exact)
    cur_comp_lines = _exact_lines(COMPOSITION)
    cur_comp_bytes = tree_bytes("HEAD", COMPOSITION) or 1
    bytes_per_line = cur_comp_bytes / cur_comp_lines if cur_comp_lines else 30.0
    for s in samples:
        s["composition_kloc"] = round(s["composition_bytes"] / bytes_per_line / 1000, 1)
        s["v1_src_kloc"] = round(s["v1_src_bytes"] / bytes_per_line / 1000, 1)
        s["crates_kloc"] = round(s["crates_bytes"] / bytes_per_line / 1000, 1)
        # Byte-share of ALL crate code INCLUDING tests — a coarse historical
        # gauge, NOT the ratchet metric (which is production-LOC over crates/*/src
        # with test files excluded; see composition_share_gate_pct).
        s["composition_byte_share_pct"] = round(
            100 * s["composition_bytes"] / s["crates_bytes"], 1
        ) if s["crates_bytes"] else 0.0
    res["size_trend"] = samples

    # ---- current-state snapshot ----
    import glob, os
    crate_dirs = [d for d in glob.glob("crates/*") if os.path.isdir(d)]
    res["crate_count"] = len(crate_dirs)
    res["composition_kloc_now"] = round(cur_comp_lines / 1000, 1)
    res["v1_src_kloc_now"] = round(_exact_lines(V1_SRC) / 1000, 1)
    res["crates_kloc_now"] = round(_exact_lines(CRATES_SRC) / 1000, 1)

    # Gate-aligned current share — matches scripts/ci/check-composition-budget.sh
    # EXACTLY (production LOC, test-only files excluded, crates/*/src denominator).
    # This is the number the ratchet enforces; the byte-based size_trend below is
    # a DIFFERENT, coarser measurement (see its label).
    comp_prod = _prod_lines(COMPOSITION)
    all_prod = _prod_lines("crates/*/src")
    res["composition_share_gate_pct"] = (
        round(100 * comp_prod / all_prod, 2) if all_prod else 0.0
    )

    traits = count_matches(r"^\s*(pub |pub\(crate\) )?(unsafe )?trait [A-Za-z0-9_]+", "crates")
    # Trait impls incl. generics: `impl Trait for T` AND `impl<T> Trait for T`
    # (optional generic clause with NO required space before `<`).
    impls = count_matches(r"^[[:space:]]*impl(<[^>]*>)?[[:space:]]+[A-Za-z0-9_:<>]+ for ", "crates")
    res["trait_defs"] = traits
    res["trait_impls_for"] = impls
    res["impls_per_trait"] = round(impls / traits, 2) if traits else 0.0

    # Dispatch signals — the "reduce traits & dispatch" companion to the mass
    # metric (#6168 / #4471). Governed scope = composition production code
    # excluding slack/ and extension_host/, matching the dispatch ratchet.
    res["composition_arc_dyn"] = _governed_arc_dyn()
    res["composition_dyn_types"] = _governed_dyn_types()

    # file sprawl vs the 1500 / 3000 rule
    big = _files_over(1500)
    res["files_over_1500"] = len(big)
    res["files_over_3000"] = len([f for f in big if f[0] > 3000])
    res["biggest_files"] = big[:8]

    # boundary-enforcement coverage
    res["boundary_test_count"] = _boundary_tests()
    res["arch_exempt_allows"] = count_matches(r"arch-exempt", "crates")

    return res


# Test-only file pattern — kept in sync with TEST_FILE_RE in
# scripts/ci/check-composition-budget.sh so the gate-aligned share matches.
TEST_FILE_RE = r"(^|/)(tests?\.rs|test_[^/]*\.rs|[^/]*_tests?\.rs)$|/tests/"


def _prod_lines(path_glob: str) -> int:
    """Production LOC of *.rs under path_glob, excluding test-only files —
    the gate's exact numerator/denominator definition."""
    out = sh(
        f"find {path_glob} -name '*.rs' -type f 2>/dev/null "
        f"| {{ grep -vE \"{TEST_FILE_RE}\" || true; }} "
        f"| tr '\\n' '\\0' | xargs -0 cat 2>/dev/null | wc -l"
    )
    try:
        return int(out.strip().split()[0])
    except (ValueError, IndexError):
        return 0


def _governed_pipeline(inner: str) -> str:
    """find composition production files (excl slack/extension_host — the separate
    workstream) piped through `inner` — the exact scope the dispatch ratchet uses."""
    return (
        f"find {COMPOSITION} -name '*.rs' -type f 2>/dev/null "
        f"| {{ grep -vE \"{TEST_FILE_RE}\" || true; }} "
        f"| {{ grep -vE '/(slack|extension_host)/' || true; }} "
        f"| tr '\\n' '\\0' | {{ xargs -0 {inner} 2>/dev/null || true; }}"
    )


def _governed_arc_dyn() -> int:
    out = sh(_governed_pipeline("grep -ho 'Arc<dyn'") + " | wc -l")
    try:
        return int(out.strip().split()[0])
    except (ValueError, IndexError):
        return 0


def _governed_dyn_types() -> int:
    out = sh(_governed_pipeline("grep -hoE 'dyn [A-Za-z_:][A-Za-z0-9_:]*'") + " | sort -u | wc -l")
    try:
        return int(out.strip().split()[0])
    except (ValueError, IndexError):
        return 0


def _exact_lines(path: str) -> int:
    out = sh(f"find {path} -name '*.rs' -type f -print0 | xargs -0 cat 2>/dev/null | wc -l")
    try:
        return int(out.strip().split()[0])
    except (ValueError, IndexError):
        return 0


def _files_over(threshold: int) -> list:
    out = sh("find crates -name '*.rs' -type f -exec wc -l {} + "
             "| awk '$2!=\"total\"{print $1\" \"$2}' | sort -rn")
    result = []
    for line in out.splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2 and parts[0].isdigit() and int(parts[0]) > threshold:
            result.append((int(parts[0]), parts[1]))
    return result


def _boundary_tests() -> int:
    f = "crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs"
    # grep -c exits 1 when zero matches — that is a valid 0, not a probe failure.
    out = sh(f"grep -cE '#\\[test\\]' {f} 2>/dev/null", ok=(0, 1))
    try:
        return int(out.strip() or 0)
    except ValueError:
        return 0


# ----------------------------------------------------------------------------
# render
# ----------------------------------------------------------------------------


def render_md(t1, t2, t3, now) -> str:
    L = []
    L.append(f"# IronClaw Development Metrics — {now.date().isoformat()}\n")
    L.append("_Trend tool: read direction, not absolutes. Derived from git + GitHub + working tree._\n")

    # Tier 1
    L.append("## Tier 1 — Flow / Speed\n")
    L.append("| metric | value |")
    L.append("|---|---|")
    L.append(f"| PRs merged (7d / 30d / 60d / 90d) | {t1['prs_merged_7d']} / {t1['prs_merged_30d']} / {t1['prs_merged_60d']} / {t1['prs_merged_90d']} |")
    L.append(f"| PRs merged (all history) | {t1['prs_total_history']} |")
    if t1.get("gh_pr_sample"):
        L.append(f"| Lead time open→merge  p50 / p90 / max (h) | {t1['lead_time_h_p50']} / {t1['lead_time_h_p90']} / {t1['lead_time_h_max']} |")
        L.append(f"| PR size lines  p50 / p90 | {t1['pr_lines_p50']} / {t1['pr_lines_p90']} |")
        L.append(f"| PR files changed  p50 / p90 | {t1['pr_files_p50']} / {t1['pr_files_p90']} |")
        L.append(f"| Small-PR share (≤200 lines) | {t1['pr_small_share']}% |")
        L.append(f"| _(GitHub sample size)_ | {t1['gh_pr_sample']} PRs |")
    else:
        L.append("| Lead time / PR size | _gh unavailable — skipped_ |")
    L.append("")

    # Tier 2
    L.append("## Tier 2 — Quality / Stability\n")
    L.append(f"- **Overall change-failure proxy** (fix / feat+fix+refactor+perf): **{t2['overall_change_failure_pct']}%**")
    L.append(f"- **Overall fix : feat ratio**: **{t2['overall_fix_per_feat']}**")
    L.append(f"- **Reverts / undo-PRs (rework)**: **{t2['overall_reverts']}**")
    tt = t2["type_totals"]
    L.append(f"- Commit-type totals: feat {tt['feat']}, fix {tt['fix']}, refactor {tt['refactor']}, test {tt['test']}, perf {tt['perf']}, docs {tt['docs']}, chore {tt['chore']}\n")
    L.append("### By 30-day period (newest first)\n")
    L.append("| window | commits | feat | fix | refactor | test | reverts | change-fail % | fix/feat | test-commit % |")
    L.append("|---|---|---|---|---|---|---|---|---|---|")
    for p in t2["by_period_30d"]:
        L.append(f"| {p['period']} | {p['commits']} | {p['feat']} | {p['fix']} | {p['refactor']} | {p['test']} | {p['reverts']} | {p['change_failure_pct']} | {p['fix_per_feat']} | {p['test_commit_pct']} |")
    L.append("")

    # Tier 3
    L.append("## Tier 3 — Codebase Health (leading indicators)\n")
    L.append("| metric | value |")
    L.append("|---|---|")
    L.append(f"| Workspace crates | {t3['crate_count']} |")
    L.append(f"| **composition share (ratchet metric)** | **{t3['composition_share_gate_pct']}%** |")
    L.append(f"| composition KLOC (now) | {t3['composition_kloc_now']} |")
    L.append(f"| v1 `src/` KLOC (now) | {t3['v1_src_kloc_now']} |")
    L.append(f"| all `crates/` KLOC (now) | {t3['crates_kloc_now']} |")
    L.append(f"| trait defs / impls / impls-per-trait | {t3['trait_defs']} / {t3['trait_impls_for']} / {t3['impls_per_trait']} |")
    L.append(f"| **composition Arc<dyn> (governed, ratchet)** | **{t3.get('composition_arc_dyn', '-')}** |")
    L.append(f"| composition distinct dyn traits (governed) | {t3.get('composition_dyn_types', '-')} |")
    L.append(f"| files > 1500 / > 3000 lines | {t3['files_over_1500']} / {t3['files_over_3000']} |")
    L.append(f"| boundary tests | {t3['boundary_test_count']} |")
    L.append(f"| arch-exempt allows | {t3['arch_exempt_allows']} |")
    L.append("")
    L.append("### Size trend — composition mass & v1 burndown (calibrated KLOC est.)\n")
    L.append("_Byte-based over ALL crate code incl. tests — a coarse historical gauge, "
             "NOT the ratchet metric above (which is production-LOC, test files excluded)._\n")
    L.append("| date | composition KLOC | comp byte-% (incl tests) | v1 src KLOC | all crates KLOC |")
    L.append("|---|---|---|---|---|")
    for s in t3["size_trend"]:
        L.append(f"| {s['date']} | {s['composition_kloc']} | {s['composition_byte_share_pct']}% | {s['v1_src_kloc']} | {s['crates_kloc']} |")
    L.append("")
    L.append("### Biggest files (sprawl watch)\n")
    for n, f in t3["biggest_files"]:
        L.append(f"- {n} — `{f}`")
    L.append("")
    return "\n".join(L)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pr-limit", type=int, default=500,
                    help="how many recent merged PRs to sample from GitHub")
    ap.add_argument("--out", default=None, help="write markdown report to FILE")
    ap.add_argument("--json", dest="json_out", default=None,
                    help="write raw metrics JSON to FILE")
    args = ap.parse_args()

    now = datetime.now(timezone.utc)
    commits = load_commits()
    if not commits:
        print("no commits found", file=sys.stderr)
        return 1

    print(f"analyzing {len(commits)} commits since {commits[-1]['date'].date()} ...",
          file=sys.stderr)
    t1 = tier1(commits, now, args.pr_limit)
    t2 = tier2(commits, now)
    print("computing codebase-health trend (git history samples) ...", file=sys.stderr)
    t3 = tier3(now)

    md = render_md(t1, t2, t3, now)
    print(md)

    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(md)
        print(f"\n[written] {args.out}", file=sys.stderr)
    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as f:
            json.dump({"tier1": t1, "tier2": t2, "tier3": t3,
                       "generated": now.isoformat()}, f, indent=2)
        print(f"[written] {args.json_out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
