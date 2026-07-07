#!/usr/bin/env python3
"""lint_core -- constraint lint for LFD features.

Four checks, all configured from lfd/<feature>/harness/caps.json:
  1. canary scan          -- the dev contract's canary_token must appear
                             nowhere in the repo working tree except
                             lfd/<feature>/harness/answers.dev.json; the
                             tokens in caps.json "holdout_canaries" must
                             appear nowhere except caps.json itself.
  2. capacity caps        -- caps.json "caps": [{name, paths, pattern,
                             max_count, scope?}]; regex match count must be
                             <= max_count. scope defaults to "diff", counting
                             only added git-diff lines from base_ref to the
                             current working tree, plus untracked files as
                             fully-added lines. scope "files" preserves
                             whole-file counting across globbed files.
  3. pin verification     -- lfd/<feature>/harness/pins.json {files:
                             {path: sha256}}; mismatch is RECORDED in dev
                             mode and a VIOLATION in holdout mode.
  4. answer-literal overlap -- string literals >= 8 chars from sealed
                             matcher values must not appear in git-diff
                             added lines (base_ref..HEAD over src/**,
                             crates/**, tests/** minus
                             tests/integration/lfd/profiles), unless the
                             literal also appears in visible case inputs.

Findings are returned as data (never printed to stdout); the detailed
report is written to <state_root>/lint-reports/<feature>-<ts>.txt.
Stdlib only. See lfd/_shared/SCHEMA.md sections 5-6.
"""

import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

DEFAULT_STATE_ROOT = os.environ.get(
    "LFD_STATE_ROOT", os.path.expanduser("~/.ironclaw-lfd")
)
MIN_LITERAL_LEN = 8
DIFF_PATHSPECS = ["src/**", "crates/**", "tests/**", ":!tests/integration/lfd/profiles"]
GREP_EXCLUDES = (
    "target/**",
    "target-lfd/**",
    ".git/**",
    ".codebase-memory/**",
    "__pycache__/**",
)
# Matcher keys whose values are schema structure, not answer content.
LITERAL_SKIP_KEYS = frozenset(["id", "type"])


class LintConfigError(Exception):
    """Broken eval configuration (missing/unreadable answers etc.) --
    a hard error (exit 2), not a scored outcome and not a VOID."""


class Finding(object):
    def __init__(self, check, severity, detail):
        self.check = check
        self.severity = severity  # "violation" | "recorded"
        self.detail = detail

    def __repr__(self):
        return "Finding(%s, %s, %s)" % (self.check, self.severity, self.detail)


class LintResult(object):
    def __init__(self, violations, recorded, pins_ok, runner_hash, report_path):
        self.violations = violations
        self.recorded = recorded
        self.pins_ok = pins_ok
        self.runner_hash = runner_hash  # canonical combined pin hash (or None)
        self.report_path = report_path


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def combined_pin_hash(actual_by_path):
    """Canonical runner hash: sha256 over 'path=sha256' lines, sorted."""
    lines = ["%s=%s" % (p, actual_by_path[p]) for p in sorted(actual_by_path)]
    return hashlib.sha256("\n".join(lines).encode("utf-8")).hexdigest()


def _fs_grep(repo_root, token):
    """Pure-python fallback for non-git roots. Skips .git, mimics git grep -I."""
    hits = []
    needle = token.encode("utf-8")
    root = Path(repo_root)
    for p in sorted(root.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(root).as_posix()
        if any(rel.startswith(prefix.rstrip("*")) for prefix in GREP_EXCLUDES):
            continue
        try:
            data = p.read_bytes()
        except OSError:
            continue
        if b"\0" in data[:8000]:
            continue  # binary
        if needle in data:
            hits.append(rel)
    return hits


def grep_token(repo_root, token):
    """Paths (relative to repo_root) whose content contains `token`.

    Uses `git grep --untracked` (tracked + untracked, .gitignore respected);
    falls back to a filesystem walk when the root is not a git work tree.
    """
    cmd = [
        "git",
        "-C",
        str(repo_root),
        "grep",
        "-I",
        "-l",
        "-F",
        "--untracked",
        "-e",
        token,
        "--",
        ".",
        *[":(exclude)%s" % pattern for pattern in GREP_EXCLUDES],
    ]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True)
    except OSError:
        return sorted(_fs_grep(repo_root, token))
    if r.returncode == 0:
        return sorted(line for line in r.stdout.splitlines() if line)
    if r.returncode == 1:
        return []
    return sorted(_fs_grep(repo_root, token))


def _collect_strings(value, out, skip_keys):
    if isinstance(value, str):
        out.add(value)
    elif isinstance(value, list):
        for v in value:
            _collect_strings(v, out, skip_keys)
    elif isinstance(value, dict):
        for k, v in value.items():
            if k in skip_keys:
                continue
            _collect_strings(v, out, skip_keys)


def answer_literals(answers):
    """String literals >= MIN_LITERAL_LEN from matcher values."""
    lits = set()
    for contract in answers.get("contracts") or []:
        for key in ("required", "forbidden"):
            for m in contract.get(key) or []:
                _collect_strings(m, lits, LITERAL_SKIP_KEYS)
    return set(l for l in lits if len(l) >= MIN_LITERAL_LEN)


def _load_json(path, what):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        raise LintConfigError("%s not found: %s" % (what, path))
    except (OSError, ValueError) as e:
        raise LintConfigError("%s unreadable (%s): %s" % (what, e, path))


# ---------------------------------------------------------------------------
# the four checks
# ---------------------------------------------------------------------------


def _check_canaries(repo_root, feature, dev_answers, caps, findings):
    allowed_dev = "lfd/%s/harness/answers.dev.json" % feature
    canary = dev_answers.get("canary_token")
    if not canary:
        findings.append(
            Finding("canary", "violation", "answers.dev.json has no canary_token")
        )
    else:
        for hit in grep_token(repo_root, canary):
            if hit != allowed_dev:
                findings.append(
                    Finding(
                        "canary",
                        "violation",
                        "dev canary token found outside answers.dev.json: %s" % hit,
                    )
                )
    allowed_caps = "lfd/%s/harness/caps.json" % feature
    for token in caps.get("holdout_canaries") or []:
        for hit in grep_token(repo_root, token):
            if hit != allowed_caps:
                findings.append(
                    Finding(
                        "canary",
                        "violation",
                        "holdout canary token found in repo: %s" % hit,
                    )
                )


def _matching_files(repo_root, globs):
    paths = set()
    root = Path(repo_root)
    for g in globs:
        for f in sorted(root.glob(g)):
            if f.is_file():
                paths.add(f.relative_to(root).as_posix())
    return paths


def _git_diff_added_lines_worktree(repo_root, base_ref):
    cmd = ["git", "-C", str(repo_root), "diff", str(base_ref), "--"]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr.strip()
    added = []  # (file, line_text)
    current = "<unknown>"
    for line in r.stdout.splitlines():
        if line.startswith("+++ "):
            current = line[4:]
            if current.startswith("b/"):
                current = current[2:]
            continue
        if line.startswith("+"):
            added.append((current, line[1:]))

    cmd = [
        "git",
        "-C",
        str(repo_root),
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
    ]
    r = subprocess.run(cmd, capture_output=True)
    if r.returncode != 0:
        return None, r.stderr.decode("utf-8", errors="ignore").strip()
    root = Path(repo_root)
    for raw in r.stdout.split(b"\0"):
        if not raw:
            continue
        rel = raw.decode("utf-8", errors="surrogateescape")
        path = root / rel
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for line in text.splitlines():
            added.append((rel, line))
    return added, None


def _count_capacity_files(repo_root, pat, globs):
    count = 0
    hits = []
    for g in globs:
        for f in sorted(Path(repo_root).glob(g)):
            if not f.is_file():
                continue
            try:
                text = f.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue
            n = sum(1 for _ in pat.finditer(text))
            if n:
                hits.append("%s:%d" % (f.relative_to(repo_root).as_posix(), n))
            count += n
    return count, hits


def _count_capacity_diff(repo_root, pat, globs, added_lines):
    count = 0
    hit_counts = {}
    matching = _matching_files(repo_root, globs)
    for rel, text in added_lines:
        if rel not in matching:
            continue
        n = sum(1 for _ in pat.finditer(text))
        if n:
            hit_counts[rel] = hit_counts.get(rel, 0) + n
            count += n
    hits = ["%s:%d" % (rel, hit_counts[rel]) for rel in sorted(hit_counts)]
    return count, hits


def _check_capacity(repo_root, caps, findings):
    base_ref = caps.get("base_ref")
    added_lines = None
    for cap in caps.get("caps") or []:
        name = cap.get("name", "<unnamed>")
        try:
            pat = re.compile(cap["pattern"])
            max_count = int(cap["max_count"])
            globs = list(cap["paths"])
            scope = cap.get("scope", "diff")
            if scope not in ("diff", "files"):
                raise ValueError("scope must be 'diff' or 'files'")
        except (KeyError, TypeError, ValueError, re.error) as e:
            findings.append(
                Finding("capacity", "violation", "cap %s malformed: %s" % (name, e))
            )
            continue
        if scope == "files":
            count, hits = _count_capacity_files(repo_root, pat, globs)
        else:
            if not base_ref:
                findings.append(
                    Finding("capacity", "violation", "caps.json missing base_ref")
                )
                continue
            if added_lines is None:
                added_lines, err = _git_diff_added_lines_worktree(repo_root, base_ref)
                if added_lines is None:
                    findings.append(
                        Finding(
                            "capacity",
                            "violation",
                            "git diff %s failed: %s" % (base_ref, err),
                        )
                    )
                    return
            count, hits = _count_capacity_diff(repo_root, pat, globs, added_lines)
        if count > max_count:
            findings.append(
                Finding(
                    "capacity",
                    "violation",
                    "cap %s: %d match(es) > max %d (%s)"
                    % (name, count, max_count, ", ".join(hits)),
                )
            )


def _check_pins(repo_root, harness_dir, holdout, findings):
    severity = "violation" if holdout else "recorded"
    pins_path = harness_dir / "pins.json"
    if not pins_path.is_file():
        findings.append(Finding("pins", severity, "pins.json missing"))
        return False, None
    try:
        pins = _load_json(pins_path, "pins.json")
        files = pins["files"]
    except (LintConfigError, KeyError, TypeError):
        findings.append(Finding("pins", severity, "pins.json malformed"))
        return False, None
    ok = True
    actual_by_path = {}
    for rel in sorted(files):
        expected = files[rel]
        target = Path(repo_root) / rel
        if target.is_file():
            actual = sha256_file(target)
        else:
            actual = "MISSING"
        actual_by_path[rel] = actual
        if actual != expected:
            ok = False
            findings.append(
                Finding("pins", severity, "pin mismatch: %s (pinned %s, actual %s)"
                        % (rel, expected, actual))
            )
    return ok, combined_pin_hash(actual_by_path) if actual_by_path else None


def _git_diff_added_lines(repo_root, base_ref):
    cmd = ["git", "-C", str(repo_root), "diff", "%s..HEAD" % base_ref, "--"]
    cmd += DIFF_PATHSPECS
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr.strip()
    added = []  # (file, line_text)
    current = "<unknown>"
    for line in r.stdout.splitlines():
        if line.startswith("+++ "):
            current = line[4:]
            if current.startswith("b/"):
                current = current[2:]
            continue
        if line.startswith("+"):
            added.append((current, line[1:]))
    return added, None


def _check_literal_overlap(repo_root, caps, answers_list, visible_text, findings):
    base_ref = caps.get("base_ref")
    if not base_ref:
        findings.append(
            Finding("literal_overlap", "violation", "caps.json missing base_ref")
        )
        return
    lits = set()
    for answers in answers_list:
        lits |= answer_literals(answers)
    lits = set(l for l in lits if l not in visible_text)
    if not lits:
        return
    added, err = _git_diff_added_lines(repo_root, base_ref)
    if added is None:
        findings.append(
            Finding(
                "literal_overlap",
                "violation",
                "git diff %s..HEAD failed: %s" % (base_ref, err),
            )
        )
        return
    for lit in sorted(lits):
        for fname, text in added:
            if lit in text:
                findings.append(
                    Finding(
                        "literal_overlap",
                        "violation",
                        "sealed answer literal %r appears in added line (%s): %s"
                        % (lit, fname, text.strip()[:200]),
                    )
                )
                break  # one finding per literal is enough


# ---------------------------------------------------------------------------
# entry point
# ---------------------------------------------------------------------------


def _write_report(state_root, feature, findings, holdout):
    reports_dir = Path(state_root) / "lint-reports"
    try:
        reports_dir.mkdir(parents=True, exist_ok=True)
        ts = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        path = reports_dir / ("%s-%s.txt" % (feature, ts))
        n = 1
        while path.exists():
            path = reports_dir / ("%s-%s-%d.txt" % (feature, ts, n))
            n += 1
        lines = [
            "LFD lint report",
            "feature: %s" % feature,
            "mode: %s" % ("holdout" if holdout else "dev"),
            "generated: %s"
            % datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
            "",
        ]
        if not findings:
            lines.append("clean: no findings")
        for f in findings:
            lines.append("[%s] (%s) %s" % (f.severity.upper(), f.check, f.detail))
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return str(path)
    except OSError:
        return None  # report is best-effort; findings still returned as data


def run_lint(
    feature,
    lfd_root,
    repo_root=None,
    state_root=DEFAULT_STATE_ROOT,
    holdout=False,
    holdout_answers_path=None,
    write_report=True,
):
    """Run all lint checks. Returns LintResult; raises LintConfigError on a
    broken eval setup (missing answers/cases -- exit 2 territory)."""
    lfd_root = Path(lfd_root).resolve()
    repo_root = Path(repo_root).resolve() if repo_root else lfd_root.parent
    harness_dir = lfd_root / feature / "harness"

    findings = []

    caps_path = harness_dir / "caps.json"
    caps = {}
    if not caps_path.is_file():
        # Anti-tamper: a missing lint config is itself a violation, otherwise
        # deleting caps.json would disable every check.
        findings.append(Finding("config", "violation", "caps.json missing"))
    else:
        try:
            caps = _load_json(caps_path, "caps.json")
        except LintConfigError as e:
            findings.append(Finding("config", "violation", str(e)))
            caps = {}
        for key in ("base_ref", "caps", "holdout_canaries"):
            if caps and key not in caps:
                findings.append(
                    Finding("config", "violation", "caps.json missing key %r" % key)
                )

    dev_answers_path = harness_dir / "answers.dev.json"
    dev_answers = _load_json(dev_answers_path, "answers.dev.json")

    answers_list = [dev_answers]
    if holdout and holdout_answers_path is not None:
        holdout_answers = _load_json(holdout_answers_path, "answers.holdout.json")
        answers_list.append(holdout_answers)
        token = holdout_answers.get("canary_token")
        if token:
            # the token may legitimately be listed (token only, no answers)
            # in caps.json "holdout_canaries"
            allowed_caps = "lfd/%s/harness/caps.json" % feature
            for hit in grep_token(repo_root, token):
                if hit != allowed_caps:
                    findings.append(
                        Finding(
                            "canary",
                            "violation",
                            "holdout answers canary found in repo: %s" % hit,
                        )
                    )

    # visible case inputs (raw file text) -- literals present here are fair game
    cases_dir = lfd_root / feature / "eval" / "dev" / "cases"
    visible_parts = []
    if cases_dir.is_dir():
        for p in sorted(cases_dir.glob("*.json")):
            try:
                visible_parts.append(p.read_text(encoding="utf-8"))
            except OSError:
                pass
    visible_text = "\n".join(visible_parts)

    _check_canaries(repo_root, feature, dev_answers, caps, findings)
    _check_capacity(repo_root, caps, findings)
    pins_ok, runner_hash = _check_pins(repo_root, harness_dir, holdout, findings)
    _check_literal_overlap(repo_root, caps, answers_list, visible_text, findings)

    report_path = _write_report(state_root, feature, findings, holdout) if write_report else None

    violations = [f for f in findings if f.severity == "violation"]
    recorded = [f for f in findings if f.severity == "recorded"]
    return LintResult(violations, recorded, pins_ok, runner_hash, report_path)


def main(argv=None):
    import argparse

    ap = argparse.ArgumentParser(description="LFD constraint lint")
    ap.add_argument("--feature", required=True)
    ap.add_argument("--lfd-root", required=True)
    ap.add_argument("--repo-root", default=None)
    ap.add_argument("--state-root", default=DEFAULT_STATE_ROOT)
    ap.add_argument("--holdout", action="store_true")
    args = ap.parse_args(argv)
    try:
        result = run_lint(
            args.feature,
            args.lfd_root,
            repo_root=args.repo_root,
            state_root=args.state_root,
            holdout=args.holdout,
        )
    except LintConfigError as e:
        print("EVAL SPEC ERROR: %s" % e, file=sys.stderr)
        return 2
    if result.violations:
        print("VOID: constraint violation")
        return 3
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
