# Shared lcov aggregation for the Reborn coverage CI scripts.
#
# Extracted (moved, not duplicated) from reborn-coverage-summary.sh so both
# that script and reborn-coverage-ratchet.sh share ONE lcov-parsing +
# exemption-filtering + by-crate-aggregation implementation. Behavior-
# preserving extraction: reborn-coverage-summary.sh's output is unchanged.
# Regression proof lives in scripts/ci/test-reborn-coverage.sh's M/A/B/C
# sections — they exercise this module transitively through
# reborn-coverage-summary.sh and reborn-coverage-comment.sh, not directly.
#
# Two entry points:
#   load_exemptions(path) -> (exempt_modules, exempt_crates, exemptions)
#   aggregate(lcov_path, exempt_modules, exempt_crates) -> (by_crate, total, hit)

import re
import sys
import tomllib

CRATE_RE = re.compile(r"(?:^|/)crates/(ironclaw_[A-Za-z0-9_]+)/")


def load_exemptions(exemptions_path):
    with open(exemptions_path, "rb") as fh:
        manifest = tomllib.load(fh)

    # Normalized to one `label` field here (path, or "crate: <name>") so
    # callers never branch on module-vs-crate presence.
    exemptions = manifest.get("exemption", [])
    exempt_modules: set[str] = set()
    exempt_crates: set[str] = set()
    for entry in exemptions:
        module = entry.get("module")
        crate_name = entry.get("crate")
        if module and crate_name:
            print(f"malformed exemption entry (exactly one of 'module'/'crate' required, both present): {entry}", file=sys.stderr)
            sys.exit(1)
        if not module and not crate_name:
            print(f"malformed exemption entry (exactly one of 'module'/'crate' required, neither present): {entry}", file=sys.stderr)
            sys.exit(1)
        label = module if module else f"crate: {crate_name}"
        entry["label"] = label
        if not entry.get("reason"):
            print(f"exemption for '{label}' is missing 'reason'", file=sys.stderr)
            sys.exit(1)
        if not entry.get("issue"):
            print(f"exemption for '{label}' is missing 'issue'", file=sys.stderr)
            sys.exit(1)
        if module:
            if not module.startswith("crates/"):
                print(f"exemption module path '{module}' must be repo-relative and start with 'crates/'", file=sys.stderr)
                sys.exit(1)
            exempt_modules.add(module)
        else:
            exempt_crates.add(crate_name)

    return exempt_modules, exempt_crates, exemptions


def aggregate(lcov_path, exempt_modules, exempt_crates):
    by_crate: dict[str, dict[str, int]] = {}
    total = 0
    hit = 0

    current_file = None
    current_covered = None
    current_count = None

    with open(lcov_path, "r", encoding="utf-8") as fh:
        for raw_line in fh:
            line = raw_line.rstrip("\n")
            if line.startswith("SF:"):
                current_file = line[len("SF:"):]
                current_covered = None
                current_count = None
            elif line.startswith("LF:"):
                current_count = int(line[len("LF:"):])
            elif line.startswith("LH:"):
                current_covered = int(line[len("LH:"):])
            elif line == "end_of_record":
                if current_file is not None and current_covered is not None and current_count is not None:
                    # Exempted files are skipped entirely (neither help nor hurt accounting).
                    # Two match kinds share is_exempt: per-file (path suffix/exact) or whole-crate (crate name in exempt_crates).
                    is_exempt = any(current_file.endswith("/" + m) or current_file == m for m in exempt_modules)
                    match = CRATE_RE.search(current_file)
                    if not is_exempt and match and exempt_crates:
                        is_exempt = match.group(1) in exempt_crates
                    if not is_exempt:
                        if match:
                            crate = match.group(1)
                            bucket = by_crate.setdefault(crate, {"covered": 0, "count": 0})
                            bucket["covered"] += current_covered
                            bucket["count"] += current_count
                            total += current_count
                            hit += current_covered
                current_file = None
                current_covered = None
                current_count = None

    return by_crate, total, hit
