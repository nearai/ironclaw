#!/usr/bin/env bash
set -euo pipefail
# Delta lint: only fail on clippy warnings/errors that touch changed lines.
# Compares the current branch against the merge base with the upstream default branch.

CLIPPY_OUT=""
DIFF_OUT=""
CLIPPY_STDERR=""

cleanup() {
    [ -n "$CLIPPY_OUT" ] && rm -f "$CLIPPY_OUT"
    [ -n "$DIFF_OUT" ] && rm -f "$DIFF_OUT"
    [ -n "$CLIPPY_STDERR" ] && rm -f "$CLIPPY_STDERR"
}
trap cleanup EXIT

# Verify python3 is available (needed for diagnostic filtering)
if ! command -v python3 &>/dev/null; then
    echo "ERROR: python3 is required for delta lint but not found"
    exit 1
fi

# Accept optional remote name argument; default to dynamic detection
REMOTE="${1:-}"

# Determine the upstream base ref dynamically
BASE_REF=""
if [ -n "$REMOTE" ]; then
    # Use the provided remote name
    if [ -z "$BASE_REF" ]; then
        BASE_REF=$(git symbolic-ref "refs/remotes/$REMOTE/HEAD" 2>/dev/null | sed 's|refs/remotes/||' || true)
    fi
    if [ -z "$BASE_REF" ] && git rev-parse --verify "$REMOTE/main" &>/dev/null; then
        BASE_REF="$REMOTE/main"
    fi
    if [ -z "$BASE_REF" ] && git rev-parse --verify "$REMOTE/master" &>/dev/null; then
        BASE_REF="$REMOTE/master"
    fi
else
    # Try the remote HEAD symbolic ref (works for any default branch name)
    if [ -z "$BASE_REF" ]; then
        BASE_REF=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's|refs/remotes/||' || true)
    fi
    # Fall back to common default branch names
    if [ -z "$BASE_REF" ] && git rev-parse --verify origin/main &>/dev/null; then
        BASE_REF="origin/main"
    fi
    if [ -z "$BASE_REF" ] && git rev-parse --verify origin/master &>/dev/null; then
        BASE_REF="origin/master"
    fi
fi
if [ -z "$BASE_REF" ]; then
    echo "WARNING: could not determine upstream base branch, skipping delta lint"
    exit 0
fi

# Compute merge base
BASE=$(git merge-base "$BASE_REF" HEAD 2>/dev/null) || {
    echo "WARNING: git merge-base failed for $BASE_REF, skipping delta lint"
    exit 0
}

# Find changed .rs files
CHANGED_RS=$(git diff --name-only "$BASE" -- '*.rs' || true)
if [ -z "$CHANGED_RS" ]; then
    echo "==> delta lint: no .rs files changed, skipping"
    exit 0
fi

echo "==> delta lint: checking changed lines since $(echo "$BASE" | head -c 10)..."

# Extract unified-0 diff for changed line ranges
DIFF_OUT=$(mktemp "${TMPDIR:-/tmp}/ironclaw-diff.XXXXXX")
git diff --unified=0 "$BASE" -- '*.rs' > "$DIFF_OUT"

# Run clippy with JSON output (stderr shows compilation progress/errors)
CLIPPY_OUT=$(mktemp "${TMPDIR:-/tmp}/ironclaw-clippy.XXXXXX")
CLIPPY_STDERR=$(mktemp "${TMPDIR:-/tmp}/ironclaw-clippy-err.XXXXXX")
cargo clippy --locked --all-targets --message-format=json > "$CLIPPY_OUT" 2>"$CLIPPY_STDERR" || true

# Show compilation errors if clippy produced no JSON output
if [ ! -s "$CLIPPY_OUT" ] && [ -s "$CLIPPY_STDERR" ]; then
    echo "ERROR: clippy failed to produce output. Compilation errors:"
    cat "$CLIPPY_STDERR"
    exit 1
fi

# Get repo root for path normalization in Python
REPO_ROOT="$(git rev-parse --show-toplevel)"

# Filter clippy diagnostics against changed line ranges
python3 - "$DIFF_OUT" "$CLIPPY_OUT" "$REPO_ROOT" <<'PYEOF'
import json
import re
import sys
import os

def parse_diff(diff_path):
    """Parse unified-0 diff to extract {file: [[start, end], ...]} changed ranges."""
    changed = {}
    current_file = None
    with open(diff_path) as f:
        for line in f:
            # Match +++ b/path/to/file.rs or +++ /dev/null (deletion)
            if line.startswith('+++ /dev/null'):
                current_file = None
                continue
            m = re.match(r'^\+\+\+ b/(.+)$', line)
            if m:
                current_file = m.group(1)
                if current_file not in changed:
                    changed[current_file] = []
                continue
            # Match @@ hunk headers: @@ -old,count +new,count @@
            m = re.match(r'^@@ .+ \+(\d+)(?:,(\d+))? @@', line)
            if m and current_file:
                start = int(m.group(1))
                count = int(m.group(2)) if m.group(2) is not None else 1
                if count == 0:
                    continue
                end = start + count - 1
                changed[current_file].append([start, end])
    return changed

def normalize_path(path, repo_root):
    """Normalize absolute path to relative (from repo root)."""
    if os.path.isabs(path):
        if path.startswith(repo_root):
            return os.path.relpath(path, repo_root)
    return path

def in_changed_range(file_path, line_start, line_end, changed_ranges, repo_root):
    """Check if file:[line_start, line_end] overlaps any changed range."""
    rel = normalize_path(file_path, repo_root)
    ranges = changed_ranges.get(rel)
    if not ranges:
        return False
    return any(start <= line_end and line_start <= end for start, end in ranges)

def main():
    diff_path = sys.argv[1]
    clippy_path = sys.argv[2]
    repo_root = sys.argv[3]

    changed_ranges = parse_diff(diff_path)

    blocking = []
    baseline = []

    with open(clippy_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue

            if msg.get("reason") != "compiler-message":
                continue

            cm = msg.get("message", {})
            level = cm.get("level", "")
            if level not in ("warning", "error"):
                continue

            rendered = cm.get("rendered", "").strip()

            # Errors are always blocking regardless of location
            if level == "error":
                blocking.append(rendered)
                continue

            # For warnings, only block if they overlap changed lines
            spans = cm.get("spans", [])
            primary = None
            for s in spans:
                if s.get("is_primary"):
                    primary = s
                    break
            if not primary:
                if spans:
                    primary = spans[0]
                else:
                    baseline.append(rendered)
                    continue

            file_name = primary.get("file_name", "")
            line_start = primary.get("line_start", 0)
            line_end = primary.get("line_end", line_start)

            if in_changed_range(file_name, line_start, line_end, changed_ranges, repo_root):
                blocking.append(rendered)
            else:
                baseline.append(rendered)

    if baseline:
        print(f"\n--- Baseline warnings (not in changed lines, informational) [{len(baseline)}] ---")
        for w in baseline[:10]:
            print(w)
        if len(baseline) > 10:
            print(f"  ... and {len(baseline) - 10} more")

    if blocking:
        print(f"\n*** BLOCKING: {len(blocking)} issue(s) in changed lines ***")
        for w in blocking:
            print(w)
        sys.exit(1)
    else:
        print("\n==> delta lint: passed (no issues in changed lines)")
        sys.exit(0)

if __name__ == "__main__":
    main()
PYEOF
