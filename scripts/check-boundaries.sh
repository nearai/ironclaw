#!/usr/bin/env bash
# Architecture boundary checks for IronClaw.
# Run as: bash scripts/check-boundaries.sh
# Returns non-zero if hard violations are found.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

violations=0

echo "=== Architecture Boundary Checks ==="
echo

# --------------------------------------------------------------------------
# Check 1: Direct database driver usage outside the db layer
# --------------------------------------------------------------------------
# tokio_postgres:: and libsql:: types should only appear in:
#   - src/db/           (the database abstraction layer)
#   - src/workspace/repository.rs (workspace's own DB layer)
#   - src/error.rs      (needs From impls for driver error types)
#   - src/app.rs        (bootstraps/initialises the database)
#   - src/testing.rs    (test infrastructure)
#   - src/cli/          (CLI commands that bootstrap DB connections)
#   - src/setup/        (onboarding wizard bootstraps DB)
#   - src/main.rs       (entry point)
#
# Everything else is a boundary violation -- those modules should go through
# the Database trait, not touch driver types directly.
# --------------------------------------------------------------------------

echo "--- Check 1: Direct database driver usage outside db layer ---"

results=$(grep -rn 'tokio_postgres::\|libsql::' src/ \
    --include='*.rs' \
    | grep -v 'src/db/' \
    | grep -v 'src/workspace/repository.rs' \
    | grep -v 'src/error.rs' \
    | grep -v 'src/app.rs' \
    | grep -v 'src/testing.rs' \
    | grep -v 'src/cli/' \
    | grep -v 'src/setup/' \
    | grep -v 'src/main.rs' \
    | grep -v '^\s*//' \
    | grep -v '//.*tokio_postgres\|//.*libsql' \
    || true)

if [ -n "$results" ]; then
    echo "VIOLATION: Direct database driver usage found outside db layer:"
    echo "$results"
    echo
    count=$(echo "$results" | wc -l | tr -d ' ')
    echo "($count occurrence(s) -- these modules should use the Database trait)"
    violations=$((violations + 1))
else
    echo "OK"
fi
echo

# --------------------------------------------------------------------------
# Check 2: .unwrap() / .expect() in production code (heuristic)
# --------------------------------------------------------------------------
# We cannot perfectly distinguish test vs production code with grep alone
# (test modules span many lines). Instead we:
#   1. Exclude files that are entirely test infrastructure
#   2. Exclude lines that are clearly in test code (assert, #[test], etc.)
#   3. Report a per-file summary so reviewers can focus on the worst files
#
# This is a WARNING, not a hard violation.
# --------------------------------------------------------------------------

echo "--- Check 2: .unwrap() / .expect() / assert!() in production code ---"

# Collect raw matches excluding obvious test-only files and lines.
# Also catches assert!(), assert_eq!(), assert_ne!() but NOT debug_assert variants.
raw_results=$(grep -rnE '\.(unwrap|expect)\(|[^_]assert(_eq|_ne)?!' src/ \
    --include='*.rs' \
    | grep -v 'src/main.rs' \
    | grep -v 'src/testing.rs' \
    | grep -v 'src/setup/' \
    | grep -Ev 'debug_assert|// safety:' \
    || true)

if [ -n "$raw_results" ]; then
    total=$(echo "$raw_results" | wc -l | tr -d ' ')
    echo "WARNING: ~$total .unwrap()/.expect()/assert!() calls found in src/ (excluding main/testing/setup)."
    echo "Many are in test modules; a per-file breakdown helps triage:"
    echo
    # Show per-file counts, sorted by count descending, top 15
    file_counts=$(echo "$raw_results" | cut -d: -f1 | sort | uniq -c | sort -rn)
    echo "$file_counts" | head -15
    fc_total=$(echo "$file_counts" | wc -l | tr -d ' ')
    if [ "$fc_total" -gt 15 ]; then
        echo "    ... and $((fc_total - 15)) more files"
    fi
    echo
    echo "(This is a warning for gradual cleanup, not a blocking violation.)"
    echo "(Many of these are inside #[cfg(test)] modules which is acceptable.)"
else
    echo "OK"
fi
echo

# --------------------------------------------------------------------------
# Check 3: std::env::var reads outside config/bootstrap layers
# --------------------------------------------------------------------------
# Sensitive values should come through Config or the secrets module.
# Direct std::env::var / env::var() reads are allowed in:
#   - src/config/       (the config layer itself)
#   - src/main.rs       (entry point)
#   - src/setup/        (onboarding wizard)
#   - src/testing.rs    (test infrastructure)
#   - src/cli/          (CLI commands that read env for bootstrap)
#   - src/bootstrap.rs  (bootstrap logic)
# --------------------------------------------------------------------------

echo "--- Check 3: Direct env var reads outside config layer ---"

results=$(grep -rn 'std::env::var\|env::var(' src/ \
    --include='*.rs' \
    | grep -v 'src/config/' \
    | grep -v 'src/main.rs' \
    | grep -v 'src/setup/' \
    | grep -v 'src/testing.rs' \
    | grep -v 'src/cli/' \
    | grep -v 'src/bootstrap.rs' \
    | grep -v '#\[cfg(test)\]' \
    | grep -v '#\[test\]' \
    | grep -v 'mod tests' \
    | grep -v 'fn test_' \
    | grep -v '//.*env::var' \
    || true)

if [ -n "$results" ]; then
    count=$(echo "$results" | wc -l | tr -d ' ')
    echo "WARNING: Direct env var reads found outside config layer ($count occurrences):"
    echo "$results"
    echo
    echo "(Review these -- secrets/config should come through Config or the secrets module)"
else
    echo "OK"
fi
echo

# --------------------------------------------------------------------------
# Check 4: Test tier gating — integration tests must use feature flags
# --------------------------------------------------------------------------
# Files in tests/ that connect to PostgreSQL or use DATABASE_URL must be
# gated behind #![cfg(all(feature = "postgres", feature = "integration"))].
# This ensures `cargo test` (no flags) never requires external services.
#
# Heuristic: any test file referencing DATABASE_URL, connect(), PgPool,
# or tokio_postgres should have the cfg gate on the first few lines.
# --------------------------------------------------------------------------

echo "--- Check 4: Test tier gating for integration tests ---"

tier_violations=()
for test_file in tests/*.rs; do
    [ -f "$test_file" ] || continue

    # Check if the file actually connects to a database (imports DB types
    # or calls pool/connect). Mere string references like "DATABASE_URL"
    # in config tests don't count.
    needs_gate=false
    if grep -q 'PgPool\|tokio_postgres::\|create_pool\|\.connect(' "$test_file" 2>/dev/null; then
        needs_gate=true
    fi

    if [ "$needs_gate" = true ]; then
        # Check first 5 lines for the cfg gate
        if ! head -5 "$test_file" | grep -q 'cfg.*feature.*integration' 2>/dev/null; then
            tier_violations+=("  $test_file: needs '#![cfg(all(feature = \"postgres\", feature = \"integration\"))]'")
        fi
    fi
done

if [ ${#tier_violations[@]} -gt 0 ]; then
    echo "VIOLATION: Integration tests missing feature gate:"
    printf '%s\n' "${tier_violations[@]}"
    echo
    echo "(Tests requiring external services must be gated behind the 'integration' feature)"
    violations=$((violations + 1))
else
    echo "OK"
fi
echo

# --------------------------------------------------------------------------
# Check 5: No silent test-skip patterns (try_connect, is_available, etc.)
# --------------------------------------------------------------------------
# Tests must fail loudly when prerequisites are missing, not silently skip.
# The correct approach is feature-flag gating (#![cfg(feature = "integration")]).
# Patterns like try_connect().is_none() { return; } hide broken tests.
# --------------------------------------------------------------------------

echo "--- Check 5: No silent test-skip patterns ---"

skip_results=$(grep -rn 'try_connect\|is_available.*return\|is_none.*return\|is_err.*return.*//.*skip' tests/ \
    --include='*.rs' \
    || true)

if [ -n "$skip_results" ]; then
    echo "VIOLATION: Silent test-skip patterns found (use feature gates instead):"
    echo "$skip_results"
    echo
    violations=$((violations + 1))
else
    echo "OK"
fi
echo

# --------------------------------------------------------------------------
# Check 6: LLM module isolation — no imports from other crate modules
# --------------------------------------------------------------------------
# src/llm/ should only import from:
#   - crate::llm (self-references)
#   - external crates (no crate:: prefix)
# It must NOT import from crate::agent, crate::tools, crate::channels,
# crate::safety, crate::config, crate::bootstrap, crate::cli, crate::db,
# crate::workspace, crate::worker, crate::orchestrator, crate::skills,
# crate::hooks, crate::setup, crate::context, etc.
#
# Test-only imports (crate::testing) are excluded since they don't affect
# the runtime dependency graph and won't exist in the extracted crate.
# --------------------------------------------------------------------------

echo "--- Check 6: LLM module isolation ---"

# Match any `crate::` reference (use-imports AND inline paths) that isn't
# crate::llm or crate::testing.  Filter out comments.
# We strip inline comments (everything after //) with sed before checking,
# so a line like `real_code(crate::foo); // crate::llm` is still caught.
results=$(grep -rn 'crate::' src/llm/ \
    --include='*.rs' \
    | grep -v '^\s*//' \
    | sed 's|//.*||' \
    | grep 'crate::' \
    | grep -v 'crate::llm' \
    | grep -v 'crate::testing' \
    || true)

if [ -n "$results" ]; then
    count=$(echo "$results" | wc -l | tr -d ' ')
    echo "WARNING: src/llm/ has $count reference(s) to modules outside crate::llm:"
    echo "$results"
    echo
    echo "(These are pre-existing; fix them before extracting the crate.)"
    echo "(New 'use crate::' imports are hard violations — see below.)"
    echo
    # Hard-fail only on new `use crate::` imports (easy to avoid in new code).
    use_imports=$(echo "$results" | grep '^[^:]*:.*use crate::' || true)
    if [ -n "$use_imports" ]; then
        echo "HARD VIOLATION: new 'use crate::' imports in src/llm/:"
        echo "$use_imports"
        violations=$((violations + 1))
    fi
else
    echo "OK"
fi
echo

# --------------------------------------------------------------------------
# Summary
# --------------------------------------------------------------------------

echo "=== Summary ==="
if [ "$violations" -gt 0 ]; then
    echo "FAILED: $violations hard violation(s) found"
    exit 1
else
    echo "PASSED: No hard violations found (review warnings above)"
    exit 0
fi
