#!/usr/bin/env python3
"""Unit tests for the pure logic in dev_metrics.py.

Covers the parsing/classification/aggregation/rendering that carried the
review-flagged bugs (conventional-commit classification, percentile math,
change-failure bucketing, Markdown rendering, and the test-file regex kept in
sync with the ratchet gate). Runs with no git/network — imports the module and
exercises functions directly.

    python3 scripts/test_dev_metrics.py     # standalone
    pytest scripts/test_dev_metrics.py      # or via pytest
"""

import os
import re
import sys
from datetime import datetime, timezone, timedelta

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dev_metrics as dm  # noqa: E402


def test_classify_commit_types():
    assert dm.classify_commit("feat(reborn): x (#10)")["type"] == "feat"
    assert dm.classify_commit("fix: y")["type"] == "fix"
    assert dm.classify_commit("feat(reborn)!: breaking (#11)")["type"] == "feat"
    assert dm.classify_commit("chore(ci): z")["type"] == "chore"
    assert dm.classify_commit("random prose commit")["type"] is None


def test_classify_commit_pr_detection():
    assert dm.classify_commit("fix: y (#6130)")["is_pr"] is True
    assert dm.classify_commit("fix: y")["is_pr"] is False
    # trailing text after the paren means it is not a squash-merge subject
    assert dm.classify_commit("fix: y (#6130) follow")["is_pr"] is False


def test_classify_commit_revert_signals():
    assert dm.classify_commit("revert: bad change")["is_revert"] is True
    assert dm.classify_commit("Revert \"feat: x\"")["is_revert"] is True
    assert dm.classify_commit("fix: undoes #5902 loop")["is_revert"] is True
    assert dm.classify_commit("fix: reverts #10")["is_revert"] is True
    assert dm.classify_commit("feat: normal")["is_revert"] is False


def test_pct_percentiles():
    vals = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    assert dm.pct([], 0.5) == 0.0
    assert dm.pct([42], 0.9) == 42
    assert abs(dm.pct(vals, 0.5) - 5.5) < 1e-9
    assert dm.pct(vals, 1.0) == 10
    assert dm.pct(vals, 0.0) == 1


def _commit(subj, days_ago, now):
    return {"hash": "h", "date": now - timedelta(days=days_ago),
            "subj": subj, **dm.classify_commit(subj)}


def test_tier2_change_failure_math():
    now = datetime(2026, 7, 16, tzinfo=timezone.utc)
    commits = [
        _commit("feat: a (#1)", 1, now),
        _commit("feat: b (#2)", 2, now),
        _commit("fix: c (#3)", 3, now),
        _commit("fix: d (#4)", 4, now),
        _commit("fix: e (#5)", 5, now),
        _commit("refactor: f (#6)", 6, now),
    ]
    t2 = dm.tier2(commits, now)
    # fix / (feat+fix+refactor+perf) = 3 / (2+3+1+0) = 50.0%
    assert t2["overall_change_failure_pct"] == 50.0
    assert t2["overall_fix_per_feat"] == 1.5
    assert t2["type_totals"]["fix"] == 3
    # all within one 30-day window
    assert t2["by_period_30d"][0]["change_failure_pct"] == 50.0


def test_tier2_revert_count():
    now = datetime(2026, 7, 16, tzinfo=timezone.utc)
    commits = [
        _commit("fix: undoes #99", 1, now),
        _commit("revert: nope", 2, now),
        _commit("feat: fine", 3, now),
    ]
    assert dm.tier2(commits, now)["overall_reverts"] == 2


def test_render_md_smoke():
    now = datetime(2026, 7, 16, tzinfo=timezone.utc)
    t1 = {"prs_merged_7d": 1, "prs_merged_30d": 2, "prs_merged_60d": 3,
          "prs_merged_90d": 4, "prs_total_history": 5, "gh_pr_sample": None}
    t2 = dm.tier2([_commit("fix: x (#1)", 1, now)], now)
    t3 = {"crate_count": 69, "composition_share_gate_pct": 23.98,
          "composition_kloc_now": 156.4, "v1_src_kloc_now": 290.0,
          "crates_kloc_now": 652.0, "trait_defs": 369, "trait_impls_for": 2882,
          "impls_per_trait": 7.81, "composition_arc_dyn": 1093,
          "composition_dyn_types": 259, "files_over_1500": 164, "files_over_3000": 56,
          "boundary_test_count": 34, "arch_exempt_allows": 101,
          "size_trend": [{"date": "2026-07-16", "composition_kloc": 156.4,
                          "composition_byte_share_pct": 20.2, "v1_src_kloc": 268.5,
                          "crates_kloc": 973.2}],
          "biggest_files": [(17371, "a.rs")]}
    md = dm.render_md(t1, t2, t3, now)
    assert "Tier 1 — Flow / Speed" in md
    assert "composition share (ratchet metric)" in md
    assert "23.98%" in md
    # dispatch signal must render
    assert "composition Arc<dyn> (governed, ratchet)" in md
    assert "1093" in md
    # the byte trend must be explicitly disclaimed as not the ratchet metric
    assert "NOT the ratchet metric" in md


def test_test_file_regex_matches_gate():
    # Kept in sync with TEST_FILE_RE in check-composition-budget.sh.
    rx = re.compile(dm.TEST_FILE_RE)
    for p in ["crates/x/src/tests.rs", "crates/x/src/foo_tests.rs",
              "crates/x/src/foo_test.rs", "crates/x/src/test_support.rs",
              "crates/x/src/runtime/tests/a.rs"]:
        assert rx.search(p), f"should match test file: {p}"
    for p in ["crates/x/src/lib.rs", "crates/x/src/runtime.rs",
              "crates/x/src/greatest.rs"]:
        assert not rx.search(p), f"should NOT match prod file: {p}"


def test_impl_regex_counts_generics():
    # The tier3 impl-density regex must count both plain and generic impls.
    rx = re.compile(r"^[[:space:]]*impl(<[^>]*>)?[[:space:]]+[A-Za-z0-9_:<>]+ for ".replace(
        "[[:space:]]", r"\s"))
    assert rx.search("impl Foo for Bar {")
    assert rx.search("impl<T> Foo for Bar<T> {")
    assert rx.search("    impl<T: Clone> Foo for Bar {")


def main() -> int:
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    passed = failed = 0
    for t in tests:
        try:
            t()
            passed += 1
        except AssertionError as e:
            failed += 1
            print(f"FAIL: {t.__name__}: {e}")
        except Exception as e:  # noqa: BLE001
            failed += 1
            print(f"ERROR: {t.__name__}: {type(e).__name__}: {e}")
    print(f"\ndev_metrics unit tests: {passed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
