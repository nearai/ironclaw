#!/usr/bin/env bash
set -euo pipefail

# Static check: every `include_str!("...")` target must (A) exist on disk and
# (B) be present in the Docker build context of any Dockerfile that compiles
# the referencing crate.
#
# Motivation: issue #5603 / the 2026-07-03 Docker outage. Two new
# `include_str!("../../prompts/*.md")` call sites were added in `src/`, but the
# repo-root `prompts/` directory was not `COPY`d into the Dockerfile builder
# stage. The host build passed (the files exist in the repo) while every Docker
# build failed for ~15 consecutive runs with:
#   error: couldn't read `src/hooks/../../prompts/session_summary.md`
# A host-only compile can never catch this class; a static context check can,
# in milliseconds, with no compile.
#
# Runs standalone (`bash scripts/ci/check-include-str-paths.sh`) and from the
# pre-push hook. Whole-repo scan (not delta) — it is cheap and the invariant is
# global.

# Root to scan. Defaults to the git top-level; tests pass an explicit tree.
REPO_ROOT="${1:-$(git rev-parse --show-toplevel)}"

python3 - "$REPO_ROOT" <<'PY'
import re
import sys
from pathlib import Path

repo = Path(sys.argv[1]).resolve()

INCLUDE_RE = re.compile(r'include_str!\(\s*"([^"]+)"\s*\)')
CFG_TEST_RE = re.compile(r'#\[cfg\(test\)\]')
# `COPY <src>... <dest>` but not `COPY --from=<stage> ...` (those pull from a
# previous build stage, not the local context).
COPY_RE = re.compile(r'^\s*COPY\s+(?!--from=)(.+?)\s*$', re.IGNORECASE)
CARGO_BUILD_RE = re.compile(r'cargo\s+build')


def top_segment(rel: str) -> str:
    parts = Path(rel).parts
    return parts[0] if parts else ""


def is_covered(rel, roots):
    """True if `rel` (a repo-relative path) is at or under any COPY root. A
    root is the normalized copied path (e.g. `crates/foo` from
    `COPY crates/foo/ crates/foo/`), so a narrowed crate copy covers that
    crate but not its siblings."""
    p = Path(rel)
    while True:
        if str(p) in roots:
            return True
        if p.parent == p:
            return False
        p = p.parent


def cfg_test_spans(lines):
    """Return a list of (start, end) 0-based line index ranges guarded by
    `#[cfg(test)]`. `cargo build` does not compile these, so include_str!
    targets referenced only from test code are not required in a Docker
    build context. Brace-aware: handles `#[cfg(test)] mod tests { ... }` and
    `#[cfg(test)] fn helper() { ... }`."""
    spans = []
    i = 0
    n = len(lines)
    while i < n:
        if CFG_TEST_RE.search(lines[i]):
            # Find the opening brace of the guarded item. A braceless item
            # (e.g. `#[cfg(test)] use foo;`) terminates at the first `;` — do
            # NOT run forward to the next unrelated `{`, which would swallow
            # real (non-test) code below and hide its include_str! calls.
            j = i
            braceless = False
            while j < n and "{" not in lines[j]:
                if ";" in lines[j]:
                    braceless = True
                    break
                j += 1
            if braceless:
                # Single-line guarded item with no block; span is just it.
                spans.append((i, j))
                i = j + 1
                continue
            if j >= n:
                break
            depth = 0
            k = j
            while k < n:
                depth += lines[k].count("{") - lines[k].count("}")
                if depth <= 0:
                    break
                k += 1
            spans.append((i, k))
            i = k + 1
        else:
            i += 1
    return spans


def in_any_span(idx, spans):
    return any(start <= idx <= end for start, end in spans)


# ── Collect include_str! references from library/binary code ──────────────
# Restrict to src/ and crates/ — the code that actually ships in the binary.
# tests/ and tools/ prompt fixtures are lower-stakes and covered by broad
# `COPY tests/`/`COPY tools/...` lines anyway.
rs_files = []
for base in ("src", "crates"):
    d = repo / base
    if d.is_dir():
        rs_files.extend(sorted(d.rglob("*.rs")))
# The repo-root build.rs is compiled by `cargo build` too (and COPYd into the
# builder stage), so an include_str! it reads is equally a Docker-context risk.
root_build = repo / "build.rs"
if root_build.is_file():
    rs_files.append(root_build)

missing = []          # (referencing_file, raw_path)
outside = []          # (referencing_file, raw_path) — resolves outside the repo
refs = []             # (referencing_relpath, target_relpath)
for f in rs_files:
    try:
        text = f.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        continue
    if "include_str!" not in text:
        continue
    f_rel = f.relative_to(repo)
    lines = text.splitlines()
    spans = cfg_test_spans(lines) if "#[cfg(test)]" in text else []
    # Precompute character offset → line index for span filtering.
    for m in INCLUDE_RE.finditer(text):
        line_idx = text.count("\n", 0, m.start())
        if in_any_span(line_idx, spans):
            # Test-only include_str! — not compiled by `cargo build`.
            continue
        raw = m.group(1)
        target = (f.parent / raw).resolve()
        if not target.exists():
            missing.append((f_rel, raw))
            continue
        try:
            target_rel = target.relative_to(repo)
        except ValueError:
            # Resolves outside the repo: it exists on the host but can never be
            # inside a Docker build context, so the image build would fail.
            outside.append((f_rel, raw))
            continue
        refs.append((f_rel, target_rel))


# ── Parse Dockerfiles that build the binary from local source ─────────────
def parse_dockerfile(text):
    roots = set()
    full_copy = False
    builds = False
    for line in text.splitlines():
        if CARGO_BUILD_RE.search(line):
            builds = True
        m = COPY_RE.match(line)
        if not m:
            continue
        tokens = [t for t in m.group(1).split() if t]
        if len(tokens) < 2:
            continue
        srcs = tokens[:-1]  # last token is the destination
        for s in srcs:
            if s.startswith("--"):  # --chown=, --chmod=, etc.
                continue
            if s in (".", "./"):
                full_copy = True
                continue
            # Store the full normalized copied path (not just its top segment)
            # so a narrowed copy like `COPY crates/foo/` covers crates/foo but
            # not sibling crates.
            roots.add(str(Path(s)))
    return roots, full_copy, builds


dockerfiles = sorted(repo.glob("Dockerfile*"))
coverage_errors = []  # (dockerfile, referencing_file, target, missing_root)
for df in dockerfiles:
    try:
        text = df.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        continue
    roots, full_copy, builds = parse_dockerfile(text)
    if not builds or full_copy:
        # Doesn't compile from local source, or copies the whole context
        # (`COPY . .`) — every include_str! target is present by construction.
        continue
    df_rel = df.relative_to(repo)
    for ref_file, target in refs:
        # Only enforce for referencing files this Dockerfile actually copies
        # (i.e. actually compiles). A Dockerfile that never copies `src/`
        # (e.g. Dockerfile.reborn) does not compile `src/hooks/*.rs`, so a
        # repo-root `prompts/` reference from there is irrelevant to it.
        if not is_covered(ref_file, roots):
            continue
        if not is_covered(target, roots):
            coverage_errors.append((df_rel, ref_file, target, top_segment(str(target))))


# ── Report ────────────────────────────────────────────────────────────────
failed = False

if missing:
    failed = True
    print("✗ include_str!() targets that do not exist on disk:")
    for ref_file, raw in missing:
        print(f"    {ref_file}: include_str!(\"{raw}\") — file not found")
    print()

if outside:
    failed = True
    print("✗ include_str!() targets that resolve outside the build context "
          "(exist on host, but no Docker COPY can ever include them):")
    for ref_file, raw in outside:
        print(f"    {ref_file}: include_str!(\"{raw}\") — resolves outside the repo")
    print()

if coverage_errors:
    failed = True
    print("✗ include_str!() targets missing from a Dockerfile build context:")
    # Group by (dockerfile, missing_root) so the fix is obvious.
    seen = set()
    for df_rel, ref_file, target, missing_root in coverage_errors:
        key = (str(df_rel), missing_root)
        if key in seen:
            continue
        seen.add(key)
        # A top-level file (e.g. `providers.json`) is copied without a
        # trailing slash; a directory root gets `dir/ dir/`.
        is_file = str(target) == missing_root
        copy_hint = (f"COPY {missing_root} {missing_root}" if is_file
                     else f"COPY {missing_root}/ {missing_root}/")
        print(f"    {df_rel}: compiles code that does "
              f"include_str!(\"…/{target}\") but never COPYs `{missing_root}`.")
        print(f"        e.g. {ref_file} → {target}")
        print(f"        Fix: add `{copy_hint}` to the "
              f"builder stage of {df_rel} (before the `cargo build`).")
    print()

if failed:
    print("include_str! path check failed. See CLAUDE.md 'Prompt templates live "
          "in files' and issue #6018.")
    print("Bypass (not recommended): git push --no-verify")
    sys.exit(1)

print("include_str! path + Docker-COPY coverage: OK "
      f"({len(refs)} references across {len(dockerfiles)} Dockerfile(s))")
PY
