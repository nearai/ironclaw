#!/usr/bin/env bash
# Self-tests for the .githooks install story and the pre-push tiers.
# Everything heavy is stubbed inside a throwaway repo pair (worktree + bare
# origin); PATH is REPLACED so real cargo/pytest cannot leak in.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
failures=0

fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

make_repo() {
    work="$(mktemp -d)/repo"
    origin="$(mktemp -d)/origin.git"
    git init -q --bare -b main "$origin"
    git init -q -b main "$work"
    (
        cd "$work"
        git config user.email t@t && git config user.name t
        mkdir -p .githooks scripts/ci bin
        cp "$REPO_ROOT/.githooks/pre-push" .githooks/pre-push
        cp "$REPO_ROOT/.githooks/pre-commit" .githooks/pre-commit
        chmod +x .githooks/pre-push .githooks/pre-commit
        for stub in scripts/preflight-gates.sh scripts/ci/quality_gate.sh \
                    scripts/ci/check-include-str-paths.sh scripts/ci/check-hermetic-env.sh \
                    scripts/ci/reborn-local-coverage-ratchet.sh scripts/check-version-bumps.sh \
                    scripts/pre-commit-safety.sh scripts/ci/changed_workspace_packages.py; do
            mkdir -p "$(dirname "$stub")"
            if [[ "$stub" == *.py ]]; then
                printf '#!/usr/bin/env python3\nimport json\nprint(json.dumps(["a"]))\n' >"$stub"
            else
                printf '#!/usr/bin/env bash\necho "ran ${0##*/}" >>"$HOOK_TEST_LOG"\n' >"$stub"
            fi
            chmod +x "$stub"
        done
        printf '#!/usr/bin/env bash\necho "ran pytest $*" >>"$HOOK_TEST_LOG"\n' >bin/pytest
        chmod +x bin/pytest
        # DEVIATION (plan gap, recorded): the default pre-push tier's
        # changed-package clippy step (fix 7) genuinely shells out to a real
        # `cargo`, which the plan's own self-test sandbox never stubbed. PATH
        # is replaced (not prepended) in run_hook, so without this stub every
        # default-tier assertion fails with "cargo: command not found"
        # (rc=127) rather than testing the hook's own logic. Stubbed the same
        # way scripts/ci/test-preflight-gates.sh already stubs cargo.
        printf '#!/usr/bin/env bash\necho "ran cargo $*" >>"$HOOK_TEST_LOG"\n' >bin/cargo
        chmod +x bin/cargo
        git add -A && git commit -qm init
        git remote add origin "$origin"
        git push -q origin main
        git config core.hooksPath .githooks
    )
    echo "$work"
}

run_hook() { # repo, hook, env...
    local repo="$1" hook="$2"; shift 2
    set +e
    output="$(cd "$repo" && env HOOK_TEST_LOG="$repo/log" \
        PATH="$repo/bin:/usr/bin:/bin" "$@" bash ".githooks/$hook" origin 2>&1)"
    status=$?
    set -e
}

# 1. Default pre-push tier: merge check + preflight + changed-package clippy,
#    NOT the full gauntlet.
repo="$(make_repo)"
run_hook "$repo" pre-push
[ "$status" -eq 0 ] || fail "default pre-push exited $status: $output"
grep -q "ran preflight-gates.sh" "$repo/log" || fail "default tier must run preflight-gates.sh"
grep -q "ran quality_gate.sh" "$repo/log" && fail "default tier must NOT run quality_gate.sh"
grep -qF "merge check" <<<"$output" || fail "default tier must keep the merge-cleanliness check"
grep -qF "changed-package clippy" <<<"$output" || fail "default tier must run the changed-package clippy step (fix 7)"

# 2. IRONCLAW_PREPUSH_FULL=1: full gauntlet runs; Emulate CLI absent is a
#    WARNING and a skip, not a hard failure.
repo="$(make_repo)"
run_hook "$repo" pre-push IRONCLAW_PREPUSH_FULL=1 IRONCLAW_PREPUSH_REBORN_COVERAGE_RATCHET=1
[ "$status" -eq 0 ] || fail "full tier without Emulate CLI exited $status: $output"
grep -q "ran quality_gate.sh" "$repo/log" || fail "full tier must run quality_gate.sh"
grep -q "ran check-include-str-paths.sh" "$repo/log" || fail "full tier keeps static checks"
grep -q "ran reborn-local-coverage-ratchet.sh" "$repo/log" || fail "full tier keeps the ratchet"
grep -qi "warn" <<<"$output" || fail "absent Emulate CLI must print a warning"
grep -q "ran pytest" "$repo/log" && fail "absent Emulate CLI must skip the replay"
grep -qF "#6018" <<<"$output" || fail "full tier must keep the #6018/#5603/#6015 rationale comment (fix 10)"

# 3. Full tier WITH an Emulate CLI present still runs the replay.
repo="$(make_repo)"
touch "$repo/emulate-cli.js"
run_hook "$repo" pre-push IRONCLAW_PREPUSH_FULL=1 IRONCLAW_EMULATE_CLI="$repo/emulate-cli.js"
grep -q "ran pytest" "$repo/log" || fail "present Emulate CLI must run the replay"

# 4. A failing default-tier preflight fails the push.
repo="$(make_repo)"
printf '#!/usr/bin/env bash\nexit 1\n' >"$repo/scripts/preflight-gates.sh"
run_hook "$repo" pre-push
[ "$status" -ne 0 ] || fail "failing preflight must fail the hook"

# 5. pre-commit chains pre-commit-safety.sh (the one-install-story fix).
repo="$(make_repo)"
(cd "$repo" && echo x >file && git add file)
set +e
output="$(cd "$repo" && env HOOK_TEST_LOG="$repo/log" \
    PATH="$repo/bin:/usr/bin:/bin" bash .githooks/pre-commit 2>&1)"
status=$?
set -e
[ "$status" -eq 0 ] || fail "pre-commit exited $status: $output"
grep -q "ran pre-commit-safety.sh" "$repo/log" || fail "pre-commit must invoke pre-commit-safety.sh"

# 6. dev-setup.sh pins: one install story, no hook symlinks.
grep -q 'git config core.hooksPath .githooks' "$REPO_ROOT/scripts/dev-setup.sh" \
    || fail "dev-setup.sh must install hooks via git config core.hooksPath .githooks"
grep -qE 'ln -sf .*(pre-push|pre-commit|commit-msg)' "$REPO_ROOT/scripts/dev-setup.sh" \
    && fail "dev-setup.sh must not symlink hooks (worktree-broken install story)"

# 7. WORKTREE CASE (fix 8 — the corrected diagnosis, proven, not asserted).
#    A second worktree of the SAME repo must fire ITS OWN checked-out
#    .githooks content, not the first worktree's, and not require re-running
#    any install step in the second worktree — core.hooksPath is set once,
#    in the shared config, and a relative value resolves per-worktree.
repo="$(make_repo)"
worktree2="$(mktemp -d)/repo-wt2"
git -C "$repo" worktree add -q -b wt2-branch "$worktree2" >/dev/null
# Overwrite ONLY the second worktree's local (uncommitted) .githooks/pre-commit
# with a marker script — worktrees have independent working directories over
# the same object store, so this cannot touch worktree 1's checkout.
printf '#!/usr/bin/env bash\necho "MARKER: worktree2 own hooks fired" >>"$HOOK_TEST_LOG"\n' \
    >"$worktree2/.githooks/pre-commit"
chmod +x "$worktree2/.githooks/pre-commit"
(cd "$worktree2" && echo y >file2 && git add file2)
set +e
wt2_output="$(cd "$worktree2" && env HOOK_TEST_LOG="$repo/log2" \
    PATH="$repo/bin:/usr/bin:/bin" git commit -qm test 2>&1)"
wt2_status=$?
set -e
[ "$wt2_status" -eq 0 ] || fail "worktree2 commit failed: $wt2_output"
grep -q "MARKER: worktree2 own hooks fired" "$repo/log2" \
    || fail "worktree2 must run its OWN .githooks/pre-commit, not worktree1's (core.hooksPath resolves per-worktree — the actual fix, per the corrected diagnosis)"
git -C "$repo" worktree remove -f "$worktree2" >/dev/null 2>&1 || true

if [ "$failures" -gt 0 ]; then
    echo "test-githooks-install: $failures assertion(s) failed" >&2
    exit 1
fi
echo "test-githooks-install: OK"
