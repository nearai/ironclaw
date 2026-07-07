#!/usr/bin/env python3
"""Selftest harness for the LFD shared scorer (`score_core.py --self-test`).

Builds temporary fake repo roots (git-initialized) from the fixtures in
selftest/fixtures/, drives score_core/probe_core through their real CLIs
(subprocess for exact stdout/exit-code semantics, in-process for engine
edge cases) and prints PASS/FAIL per check. Deterministic, stdlib only,
never touches the real LFD_STATE_ROOT.
"""

import datetime
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures"
SCORER_DIR = HERE.parent
PYTHON = sys.executable or "python3"

if str(SCORER_DIR) not in sys.path:
    sys.path.insert(0, str(SCORER_DIR))

import lint_core  # noqa: E402
import probe_core  # noqa: E402
import score_core  # noqa: E402

CANARY_DEV = "LFDC-mini-selftest-1a2b3c4d"
CANARY_HOLDOUT = "LFDH-mini-selftest-holdout-cafe"


class CheckFailed(Exception):
    pass


def _expect(cond, msg):
    if not cond:
        raise CheckFailed(msg)


def _git(repo, *argv):
    r = subprocess.run(
        ["git", "-C", str(repo)] + list(argv), capture_output=True, text=True
    )
    if r.returncode != 0:
        raise CheckFailed("git %s failed: %s" % (" ".join(argv), r.stderr.strip()))


def _mk_repo(base, name):
    repo = Path(base) / name
    shutil.copytree(FIXTURES / "repo", repo)
    shutil.copytree(FIXTURES / "lfd", repo / "lfd")
    stub_rel = "tests/lfd/runner_stub.rs"
    pins = {"files": {stub_rel: lint_core.sha256_file(repo / stub_rel)}}
    with open(repo / "lfd" / "mini" / "harness" / "pins.json", "w", encoding="utf-8") as f:
        json.dump(pins, f, indent=2, sort_keys=True)
        f.write("\n")
    subprocess.run(
        ["git", "-c", "init.defaultBranch=main", "init", "-q", str(repo)],
        capture_output=True,
        text=True,
        check=True,
    )
    _git(repo, "add", "-A")
    _git(
        repo,
        "-c",
        "user.name=lfd-selftest",
        "-c",
        "user.email=lfd-selftest@localhost",
        "commit",
        "-q",
        "-m",
        "selftest fixture",
    )
    return repo


def _run_cli(script, argv):
    env = dict(os.environ)
    env.pop("LFD_OUT", None)
    return subprocess.run(
        [PYTHON, str(SCORER_DIR / script)] + argv,
        capture_output=True,
        text=True,
        env=env,
    )


def _run_score(repo, state, extra):
    return _run_cli(
        "score_core.py",
        [
            "--feature",
            "mini",
            "--lfd-root",
            str(Path(repo) / "lfd"),
            "--repo-root",
            str(repo),
            "--state-root",
            str(state),
        ]
        + extra,
    )


def _score_from_stdout(stdout):
    for line in stdout.splitlines():
        if line.startswith("score: "):
            return float(line.split(": ", 1)[1])
    raise CheckFailed("no 'score:' line in stdout: %r" % stdout)


# ---------------------------------------------------------------------------
# checks
# ---------------------------------------------------------------------------


def check_good_outcomes(base):
    repo = _mk_repo(base, "repo_good")
    state = Path(base) / "state_good"
    r = _run_score(repo, state, ["--outcomes", str(FIXTURES / "outcomes_good")])
    _expect(r.returncode == 0, "exit %d, stderr: %s" % (r.returncode, r.stderr))
    lines = r.stdout.splitlines()
    _expect(lines[0] == "score: 1.0000", "unexpected score line: %r" % lines[0])
    _expect(lines[1] == "cases: 2", "unexpected cases line: %r" % lines[1])
    _expect(lines[2] == "worst:", "unexpected worst header: %r" % lines[2])
    _expect(
        "mini_001 PASS" in r.stdout and "mini_002 PASS" in r.stdout,
        "worst list missing PASS rows: %r" % r.stdout,
    )
    score = _score_from_stdout(r.stdout)
    _expect(score >= 0.95, "good set scored %.4f < 0.95" % score)
    history = state / "audit" / "mini.dev-history.jsonl"
    _expect(history.is_file(), "dev-history jsonl not written")
    return "score=%.4f" % score


def check_bad_outcomes(base):
    repo = _mk_repo(base, "repo_bad")
    state = Path(base) / "state_bad"
    r = _run_score(repo, state, ["--outcomes", str(FIXTURES / "outcomes_bad")])
    _expect(r.returncode == 0, "exit %d, stderr: %s" % (r.returncode, r.stderr))
    score = _score_from_stdout(r.stdout)
    _expect(score <= 0.30, "bad set scored %.4f > 0.30" % score)
    _expect("mini_001 FAIL" in r.stdout, "mini_001 should be FAIL")
    _expect("mini_002 FAIL" in r.stdout, "mini_002 (error+hard_fail) should be FAIL")
    return "score=%.4f (missing-required + 3 forbidden + error/hard_fail)" % score


def check_unsupported_zero(base):
    repo = _mk_repo(base, "repo_unsupported")
    state = Path(base) / "state_unsupported"
    outcomes = Path(base) / "outcomes_unsupported"
    outcomes.mkdir()
    shutil.copy(
        FIXTURES / "outcomes_good" / "mini_001.outcome.json",
        outcomes / "mini_001.outcome.json",
    )
    shutil.copy(
        FIXTURES / "outcomes_unsupported" / "mini_002.outcome.json",
        outcomes / "mini_002.outcome.json",
    )
    r = _run_score(repo, state, ["--outcomes", str(outcomes)])
    _expect(r.returncode == 0, "exit %d, stderr: %s" % (r.returncode, r.stderr))
    score = _score_from_stdout(r.stdout)
    _expect(abs(score - 0.5) < 1e-9, "expected 0.5000 (1.0 + 0.0)/2, got %.4f" % score)
    _expect("mini_002 FAIL" in r.stdout, "unsupported case should be FAIL")
    return "unsupported case scored 0, aggregate=%.4f" % score


def _expect_void(r):
    _expect(r.returncode == 3, "expected exit 3, got %d" % r.returncode)
    _expect(
        r.stdout == "VOID: constraint violation\n",
        "stdout must be exactly the VOID line, got %r" % r.stdout,
    )
    _expect(r.stderr == "", "stderr must be empty on VOID, got %r" % r.stderr)


def check_void_planted_canary(base):
    repo = _mk_repo(base, "repo_void_canary")
    (repo / "src").mkdir(parents=True, exist_ok=True)
    (repo / "src" / "sneaky.rs").write_text(
        "// smuggled: %s\n" % CANARY_DEV, encoding="utf-8"
    )
    r = _run_score(
        repo,
        Path(base) / "state_void_canary",
        ["--outcomes", str(FIXTURES / "outcomes_good")],
    )
    _expect_void(r)
    return "planted canary in src/ -> exact VOID line, exit 3, nothing else"


def check_void_capacity_cap(base):
    repo = _mk_repo(base, "repo_void_cap")
    (repo / "src").mkdir(parents=True, exist_ok=True)
    (repo / "src" / "branchy.rs").write_text(
        'fn cheat(case_id: &str) -> bool { case_id == "mini_001" }\n',
        encoding="utf-8",
    )
    r = _run_score(
        repo,
        Path(base) / "state_void_cap",
        ["--outcomes", str(FIXTURES / "outcomes_good")],
    )
    _expect_void(r)
    return "untracked added per-case branching over diff cap (max_count 0) -> VOID"


def check_capacity_diff_ignores_preexisting(base):
    repo = _mk_repo(base, "repo_cap_preexisting")
    (repo / "src").mkdir(parents=True, exist_ok=True)
    (repo / "src" / "branchy.rs").write_text(
        'fn existing(case_id: &str) -> bool { case_id == "mini_001" }\n',
        encoding="utf-8",
    )
    _git(repo, "add", "src/branchy.rs")
    _git(
        repo,
        "-c",
        "user.name=lfd-selftest",
        "-c",
        "user.email=lfd-selftest@localhost",
        "commit",
        "-q",
        "-m",
        "preexisting cap matches",
    )
    r = _run_score(
        repo,
        Path(base) / "state_cap_preexisting",
        ["--outcomes", str(FIXTURES / "outcomes_good")],
    )
    _expect(r.returncode == 0, "exit %d, stderr: %s" % (r.returncode, r.stderr))
    return "committed base_ref matches ignored by default diff scope"


def check_capacity_files_scope_counts_preexisting(base):
    repo = _mk_repo(base, "repo_cap_files_scope")
    (repo / "src").mkdir(parents=True, exist_ok=True)
    (repo / "src" / "branchy.rs").write_text(
        'fn existing(case_id: &str) -> bool { case_id == "mini_001" }\n',
        encoding="utf-8",
    )
    _git(repo, "add", "src/branchy.rs")
    _git(
        repo,
        "-c",
        "user.name=lfd-selftest",
        "-c",
        "user.email=lfd-selftest@localhost",
        "commit",
        "-q",
        "-m",
        "preexisting cap matches",
    )
    caps_path = repo / "lfd" / "mini" / "harness" / "caps.json"
    caps = json.loads(caps_path.read_text(encoding="utf-8"))
    caps["caps"][0]["scope"] = "files"
    caps_path.write_text(
        json.dumps(caps, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    r = _run_score(
        repo,
        Path(base) / "state_cap_files_scope",
        ["--outcomes", str(FIXTURES / "outcomes_good")],
    )
    _expect_void(r)
    return 'scope "files" counts committed whole-file matches -> VOID'


def check_probe_map_identity(base):
    repo = _mk_repo(base, "repo_probe")
    state = Path(base) / "state_probe"
    # 1) dev run to seed dev-history
    r = _run_score(repo, state, ["--outcomes", str(FIXTURES / "outcomes_good")])
    _expect(r.returncode == 0, "dev run failed: %s" % r.stderr)
    dev_score = _score_from_stdout(r.stdout)
    # 2) generate the probe map from dev cases
    probe_out = Path(base) / "probe_out"
    rp = _run_cli(
        "probe_core.py",
        [
            "--feature",
            "mini",
            "--lfd-root",
            str(repo / "lfd"),
            "--out",
            str(probe_out),
        ],
    )
    _expect(rp.returncode == 0, "probe_core failed: %s" % rp.stderr)
    with open(probe_out / "map.json", "r", encoding="utf-8") as f:
        map_data = json.load(f)
    renames = map_data["renames"]
    for expected_token in ("U123AB", "90210441", "jane.doe@example.com", "Janet Q"):
        _expect(expected_token in renames, "map missing rename for %r" % expected_token)
    _expect(map_data["date_shift_days"] == 13, "date_shift_days must be 13")
    perturbed = json.loads((probe_out / "mini_001.json").read_text(encoding="utf-8"))
    original = json.loads(
        (repo / "lfd/mini/eval/dev/cases/mini_001.json").read_text(encoding="utf-8")
    )
    _expect(perturbed["inbound"] != original["inbound"], "probe did not perturb inputs")
    _expect(perturbed["case_id"] == "mini_001", "probe must not rename case ids")
    # 3) simulate a runner pass over the perturbed cases by applying the same
    #    isomorphism to the known-good outcomes
    rename_re = probe_core.compile_rename_regex(renames)
    days = map_data["date_shift_days"]
    outcomes_dir = Path(base) / "outcomes_probe"
    outcomes_dir.mkdir()
    for p in sorted((FIXTURES / "outcomes_good").glob("*.outcome.json")):
        outcome = json.loads(p.read_text(encoding="utf-8"))
        transformed = probe_core.apply_map_to_value(outcome, rename_re, renames, days)
        for key in ("case_id", "status", "meta"):
            transformed[key] = outcome[key]
        with open(outcomes_dir / p.name, "w", encoding="utf-8") as f:
            json.dump(transformed, f, indent=2, sort_keys=True)
    # 4) probe-mode scoring: sealed contracts transformed internally
    r2 = _run_score(
        repo,
        state,
        ["--outcomes", str(outcomes_dir), "--probe", str(probe_out / "map.json")],
    )
    _expect(r2.returncode == 0, "probe scoring failed: %s" % r2.stderr)
    probe_score = _score_from_stdout(r2.stdout)
    _expect(
        abs(probe_score - dev_score) < 1e-9,
        "renamed outcomes must score identically: dev=%.4f probe=%.4f"
        % (dev_score, probe_score),
    )
    _expect(
        "gap_vs_dev: +0.0000" in r2.stdout,
        "expected zero gap line, got %r" % r2.stdout,
    )
    return "dev=%.4f probe=%.4f gap=+0.0000" % (dev_score, probe_score)


def check_ordered_violation(base):
    answers = json.loads(
        (FIXTURES / "lfd/mini/harness/answers.dev.json").read_text(encoding="utf-8")
    )
    contract = next(c for c in answers["contracts"] if c["case_id"] == "mini_001")
    outcome = json.loads(
        (FIXTURES / "outcomes_good/mini_001.outcome.json").read_text(encoding="utf-8")
    )
    _expect(score_core.score_case(contract, outcome) == 1.0, "baseline should be 1.0")
    outcome["egress"][0]["seq"] = 9  # r3 now AFTER r2 -> ordered [r1, r3, r2] broken
    _expect(
        score_core.score_case(contract, outcome) == 0.0,
        "ordered violation must zero the case",
    )
    return "out-of-order satisfied matcher zeroes the case"


def check_unknown_matcher_rejected(base):
    contract = {
        "case_id": "x",
        "required": [{"id": "q1", "type": "quantum_entangle", "weight": 1}],
    }
    try:
        score_core.score_case(contract, {"status": "ran"})
    except score_core.EvalSpecError:
        return "unknown matcher type raises EvalSpecError (broken eval, not agent)"
    raise CheckFailed("unknown matcher type must hard-error")


def check_transcript_wer(base):
    ref = "the quick brown fox jumps over the lazy dog today"
    matcher = {
        "id": "w1",
        "type": "transcript_wer",
        "query": "t",
        "ref": ref,
        "max": 0.15,
        "weight": 1,
    }
    near = {"state": {"t": "The QUICK, brown fox jumps over the lazy dog tonight!"}}
    sat, _ = score_core.eval_matcher(matcher, near)
    wer_near = score_core.word_error_rate(ref, near["state"]["t"])
    _expect(abs(wer_near - 0.1) < 1e-9, "expected WER 0.1, got %.4f" % wer_near)
    _expect(sat, "WER 0.1 <= max 0.15 must satisfy")
    far = {"state": {"t": "a quick red fox leaps over a lazy cat today"}}
    sat2, _ = score_core.eval_matcher(matcher, far)
    wer_far = score_core.word_error_rate(ref, far["state"]["t"])
    _expect(abs(wer_far - 0.5) < 1e-9, "expected WER 0.5, got %.4f" % wer_far)
    _expect(not sat2, "WER 0.5 > max 0.15 must not satisfy")
    obj = {"state": {"t": {"text": near["state"]["t"]}}}
    sat3, _ = score_core.eval_matcher(matcher, obj)
    _expect(sat3, "object state value must fall back to its 'text' field")
    return "WER %.2f satisfied / WER %.2f violated / object.text handled" % (
        wer_near,
        wer_far,
    )


def check_holdout_flow(base):
    repo = _mk_repo(base, "repo_holdout")
    state = Path(base) / "state_holdout"
    holdout_dir = state / "holdout" / "mini"
    shutil.copytree(FIXTURES / "lfd/mini/eval/dev/cases", holdout_dir / "cases")
    answers = json.loads(
        (FIXTURES / "lfd/mini/harness/answers.dev.json").read_text(encoding="utf-8")
    )
    answers["canary_token"] = CANARY_HOLDOUT
    with open(holdout_dir / "answers.holdout.json", "w", encoding="utf-8") as f:
        json.dump(answers, f, indent=2, sort_keys=True)
    r = _run_score(
        repo,
        state,
        ["--holdout", "--outcomes", str(FIXTURES / "outcomes_good")],
    )
    _expect(r.returncode == 0, "holdout run failed (%d): %s" % (r.returncode, r.stderr))
    _expect(
        r.stdout == "1.0000\n",
        "holdout must print ONE aggregate number only, got %r" % r.stdout,
    )
    audit = state / "audit" / "mini.log"
    records = [json.loads(l) for l in audit.read_text(encoding="utf-8").splitlines()]
    _expect(len(records) == 1, "expected 1 audit line, got %d" % len(records))
    rec = records[0]
    _expect(
        set(rec) == {"ts", "feature", "score", "n_cases", "runner_hash_ok"},
        "audit line fields wrong: %r" % rec,
    )
    _expect(rec["n_cases"] == 2 and rec["feature"] == "mini", "audit content wrong")

    # exhaust the 24h window: 2 more recent calls -> 3 total -> 4th call blocked
    now = datetime.datetime.now(datetime.timezone.utc)
    with open(audit, "a", encoding="utf-8") as f:
        for _ in range(2):
            f.write(
                json.dumps(
                    {
                        "ts": (now - datetime.timedelta(hours=1)).isoformat(
                            timespec="seconds"
                        ),
                        "feature": "mini",
                        "score": 1.0,
                        "n_cases": 2,
                        "runner_hash_ok": False,
                    }
                )
                + "\n"
            )
    r2 = _run_score(
        repo,
        state,
        ["--holdout", "--outcomes", str(FIXTURES / "outcomes_good")],
    )
    _expect(r2.returncode == 4, "4th call must exit 4, got %d" % r2.returncode)
    _expect(
        r2.stdout == "HOLDOUT BUDGET EXHAUSTED\n",
        "unexpected budget stdout: %r" % r2.stdout,
    )
    # entries older than the window free the budget again
    stale = now - datetime.timedelta(hours=25)
    lines = []
    for _ in range(3):
        lines.append(
            json.dumps(
                {
                    "ts": stale.isoformat(timespec="seconds"),
                    "feature": "mini",
                    "score": 1.0,
                    "n_cases": 2,
                    "runner_hash_ok": False,
                }
            )
        )
    audit.write_text("\n".join(lines) + "\n", encoding="utf-8")
    _expect(
        score_core.holdout_calls_in_window(audit, now=now) == 0,
        "25h-old calls must not count toward the 24h window",
    )
    return "one number printed, audit line appended, 4th call in 24h blocked"


CHECKS = [
    ("good_outcomes_score_high", check_good_outcomes),
    ("bad_outcomes_score_low", check_bad_outcomes),
    ("unsupported_scores_zero", check_unsupported_zero),
    ("void_on_planted_canary", check_void_planted_canary),
    ("void_on_capacity_cap", check_void_capacity_cap),
    ("capacity_diff_ignores_preexisting", check_capacity_diff_ignores_preexisting),
    (
        "capacity_files_scope_counts_preexisting",
        check_capacity_files_scope_counts_preexisting,
    ),
    ("probe_map_identity", check_probe_map_identity),
    ("ordered_violation_zeroes_case", check_ordered_violation),
    ("unknown_matcher_hard_error", check_unknown_matcher_rejected),
    ("transcript_wer_matcher", check_transcript_wer),
    ("holdout_budget_and_audit", check_holdout_flow),
]


def run_all():
    base = tempfile.mkdtemp(prefix="lfd-scorer-selftest-")
    failed = 0
    try:
        for name, fn in CHECKS:
            try:
                detail = fn(base)
                print("[PASS] %s: %s" % (name, detail))
            except CheckFailed as e:
                failed += 1
                print("[FAIL] %s: %s" % (name, e))
            except Exception as e:  # noqa: BLE001 - selftest must report, not crash
                failed += 1
                print("[FAIL] %s: unexpected %s: %s" % (name, type(e).__name__, e))
    finally:
        shutil.rmtree(base, ignore_errors=True)
    total = len(CHECKS)
    print("SELF-TEST: %d/%d PASS" % (total - failed, total))
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(run_all())
