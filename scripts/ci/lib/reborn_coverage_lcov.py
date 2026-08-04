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

import os
import pathlib
import re
import sys
import tomllib

from crate_tree import CrateTreeError, crate_directories, workspace_root_directories


def _default_repo_root():
    """Repository root whose crate tree defines the accounting scope.

    `IRONCLAW_REPO_ROOT` first — the same override
    `scripts/ci/reborn-coverage-merge-lcov.sh` reads, so the self-test can point
    both halves of the pipeline at one fixture tree — then this file's own
    location (`scripts/ci/lib/` -> `../../..`). Not the process CWD: the
    consumers are bash scripts invoked from CI, from the local replay runner,
    and from the self-test, none of which guarantee a working directory.
    """

    override = os.environ.get("IRONCLAW_REPO_ROOT")
    if override:
        return pathlib.Path(override)
    return pathlib.Path(__file__).resolve().parents[3]


def crate_pattern(repo_root=None):
    """Regex matching any workspace crate directory, capturing that directory.

    Anchored on the *discovered* crate inventory rather than on a
    `crates/ironclaw_*` path shape. Three properties the shape-based predecessor
    did not have:

    * **Nested trees resolve.** `crates/extensions/packages/slack/src/…` is a
      crate; a pattern requiring `ironclaw_*` directly under `crates/` matched
      no part of it, so 11 crates (~33.7k instrumented lines) contributed
      nothing to the per-crate table *or* to the global aggregate — the `if
      match:` gate below guards both. Filed as #7083; the shape broke when #7037
      colocated packages under `crates/extensions/`.
    * **A directory basename need not contain `ironclaw`.** Four of the
      colocated package directories are named by extension identity
      (`packages/slack`, `telegram`, `mem0`, `memory-native`, PROPOSAL §5.1), so
      no "smarter" `ironclaw_*` regex can reach them, and a greedy one
      mis-attributes an in-crate `src/ironclaw_*/` module directory to a crate
      that does not exist.
    * **Third-party sources stay out.** Several vendored crates ship their own
      `crates/` subdirectory (`.../wasmtime-46.0.1/crates/wasmtime/src/lib.rs`);
      anchoring on the inventory keeps them out of the denominator. This is the
      same rule, and the same reasoning, as
      `scripts/ci/reborn-coverage-merge-lcov.sh`, which produces the merged lcov
      this module reads — the two disagreeing is what made the hole silent.

    Raises `CrateTreeError` when discovery cannot produce an inventory. Failing
    closed is deliberate: an aggregator that scans nothing must never report a
    coverage percentage (CHECKLIST WS10).
    """

    root = _default_repo_root() if repo_root is None else repo_root
    directories = crate_directories(root)
    _reject_ambiguous_crate_keys(directories)
    # Longest-first so the outermost/nearest owner wins even if the inventory
    # ever stops pruning manifests nested inside a crate.
    alternation = "|".join(
        re.escape(directory)
        for directory in sorted(directories, key=lambda d: (-len(d), d))
    )
    return re.compile(f"(?:^|/)({alternation})/")


def _reject_ambiguous_crate_keys(directories):
    """Refuse an inventory in which two crate directories share a basename.

    `crate_key()` reduces a directory to its basename, so a collision silently
    merges two crates into one coverage bucket and one ratchet floor — a
    *quieter* version of exactly the bug this module was fixed for (#7083),
    since the merged number looks plausible and nothing reports the merge.
    `crate_tree.crate_directory()` already treats an ambiguous basename as
    invalid rather than picking one; this is the same rule applied to the
    aggregation key.

    Unreachable on today's tree (all 65 basenames are distinct) and cheap to
    keep that way. It becomes reachable the moment crates move under family
    directories, which is the next wave.
    """

    seen: dict[str, str] = {}
    collisions: list[str] = []
    for directory in sorted(directories):
        key = crate_key(directory)
        if key in seen:
            collisions.append(f"{key!r}: {seen[key]} and {directory}")
        else:
            seen[key] = directory
    if collisions:
        raise CrateTreeError(
            "crate directories share a basename, which the coverage accounting key "
            "cannot distinguish — two crates would merge into one bucket and one "
            "floor:\n  " + "\n  ".join(collisions) + "\nRename one, or give this "
            "gate a key that survives the collision. Refusing to report a merged "
            "number (docs/reborn/target-architecture/CHECKLIST.md WS10)."
        )


def crate_key(crate_directory):
    """The by-crate accounting key for a discovered crate directory.

    The **directory basename**, which is what `crate_tree.crate_directory()`
    resolves by and what `scripts/ci/classify-test-scope.sh` keys its arms on.
    Chosen over the cargo package name for two reasons: every pre-existing key
    in `tests/integration/coverage-floor.toml` and
    `tests/integration/coverage-exemptions.toml` is already a directory basename
    (they coincide for flat `crates/ironclaw_*/` crates), so no existing floor
    churns; and package names diverge from directories in ways that would make
    worse keys (`crates/ironclaw_reborn_cli` declares `name = "ironclaw"`).

    Basenames are also stable across the family moves still ahead:
    `crates/ironclaw_llm` -> `crates/substrates/ironclaw_llm` keeps the key
    `ironclaw_llm`.
    """

    return crate_directory.rsplit("/", 1)[-1]


def separate_workspace_pattern(repo_root=None):
    """Regex matching any directory under `crates/` that roots a *different*
    cargo workspace.

    Checked before the crate pattern, because "outermost wins" would otherwise
    attribute a guest to its enclosing crate:
    `crates/extensions/packages/slack/wasm-src/` sits inside
    `crates/extensions/packages/slack/`, but `cargo build` here never compiles
    it, so no line in it is coverable and none may enter a denominator. Same
    precedence `scripts/ci/reborn_changed_coverage.py` applies, and the same
    reason `crate_tree.nested_workspace_root()` exists: "excluded by
    construction" is a different answer from "attributable to nothing".

    Returns `None` when the tree has no separate workspace roots at all — an
    empty alternation would compile to a regex matching every path.
    """

    root = _default_repo_root() if repo_root is None else repo_root
    directories = workspace_root_directories(root)
    if not directories:
        return None
    alternation = "|".join(re.escape(directory) for directory in directories)
    return re.compile(f"(?:^|/)(?:{alternation})/")


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


def aggregate(lcov_path, exempt_modules, exempt_crates, repo_root=None):
    try:
        crate_re = crate_pattern(repo_root)
        guest_re = separate_workspace_pattern(repo_root)
    except CrateTreeError as error:
        print(f"reborn-coverage: {error}", file=sys.stderr)
        sys.exit(1)

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
                    in_separate_workspace = bool(
                        guest_re and guest_re.search(current_file)
                    )
                    match = None if in_separate_workspace else crate_re.search(current_file)
                    crate = crate_key(match.group(1)) if match else None
                    if not is_exempt and crate and exempt_crates:
                        is_exempt = crate in exempt_crates
                    if not is_exempt:
                        if crate:
                            bucket = by_crate.setdefault(crate, {"covered": 0, "count": 0})
                            bucket["covered"] += current_covered
                            bucket["count"] += current_count
                            total += current_count
                            hit += current_covered
                current_file = None
                current_covered = None
                current_count = None

    return by_crate, total, hit
