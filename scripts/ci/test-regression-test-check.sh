#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CHECKER="$REPO_ROOT/scripts/ci/regression-test-check.py"
HOOK="$REPO_ROOT/scripts/commit-msg-regression.sh"
TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

init_repo() {
  local repo="$1"
  mkdir -p "$repo/src"
  git -C "$repo" init -q
  git -C "$repo" config commit.gpgsign false
  git -C "$repo" config core.hooksPath /dev/null
  git -C "$repo" config user.email "regression-check@example.invalid"
  git -C "$repo" config user.name "Regression Check Self-Test"
  printf 'pub fn value() -> i32 { 1 }\n' > "$repo/src/lib.rs"
  git -C "$repo" add .
  git -C "$repo" commit -qm "feat: baseline"
}

run_check() {
  local repo="$1"
  shift
  git -C "$repo" add -A
  python3 "$CHECKER" --repo "$repo" --base HEAD --head INDEX \
    --title "fix: reproduce escaped bug" "$@"
}

expect_pass() {
  local name="$1"
  shift
  local status=0
  "$@" >"$TMP_ROOT/output" 2>&1 || status=$?
  if [[ $status -ne 0 ]]; then
    echo "FAIL: $name unexpectedly failed"
    echo "expected exit 0, got $status"
    cat "$TMP_ROOT/output"
    exit 1
  fi
}

expect_fail() {
  local name="$1"
  local expected="$2"
  shift 2
  local status=0
  "$@" >"$TMP_ROOT/output" 2>&1 || status=$?
  if [[ $status -ne 1 ]]; then
    echo "FAIL: $name returned the wrong status"
    echo "expected exit 1, got $status"
    cat "$TMP_ROOT/output"
    exit 1
  fi
  if ! grep -Fq "$expected" "$TMP_ROOT/output"; then
    echo "FAIL: $name did not report: $expected"
    cat "$TMP_ROOT/output"
    exit 1
  fi
}

# A configuration error (a high-risk entry that resolves to nothing) is not a
# policy verdict about the PR, so it exits 2 through the existing
# infrastructure-error channel rather than 1. Still blocking: any non-zero exit
# fails the workflow step.
expect_status() {
  local name="$1"
  local want="$2"
  local expected="$3"
  shift 3
  local status=0
  "$@" >"$TMP_ROOT/output" 2>&1 || status=$?
  if [[ $status -ne $want ]]; then
    echo "FAIL: $name returned the wrong status"
    echo "expected exit $want, got $status"
    cat "$TMP_ROOT/output"
    exit 1
  fi
  if ! grep -Fq "$expected" "$TMP_ROOT/output"; then
    echo "FAIL: $name did not report: $expected"
    cat "$TMP_ROOT/output"
    exit 1
  fi
}

# Build a repo the crate inventory recognises: a `[workspace]` root manifest
# (the discriminator `regression-test-check.py` uses to decide whether a missing
# crate tree is a broken checkout) plus enough crates to clear
# crate_tree.MIN_CRATE_DIRECTORIES, and every crate the high-risk list names.
#
# `$1` is the repo path; `$2` is the directory crates live under, so the same
# scaffold can be built flat (`crates`) or nested under a family directory
# (`crates/substrates`) — the WS7 layout that made the old literal-prefix list
# silently stop matching.
init_workspace_repo() {
  local repo="$1"
  local crate_root="${2:-crates}"
  local crate index
  mkdir -p "$repo"
  git -C "$repo" init -q
  git -C "$repo" config commit.gpgsign false
  git -C "$repo" config core.hooksPath /dev/null
  git -C "$repo" config user.email "regression-check@example.invalid"
  git -C "$repo" config user.name "Regression Check Self-Test"
  printf '[workspace]\nmembers = []\n' > "$repo/Cargo.toml"

  # Every crate the gate's high-risk and static-asset entries name. Resolution
  # refuses an entry naming a crate the checkout lacks, so this list cannot
  # drift from the gate's: adding an entry for a new crate makes every fixture
  # below fail loudly until the crate appears here.
  for crate in ironclaw_turns ironclaw_processes ironclaw_llm \
    ironclaw_agent_loop ironclaw_safety ironclaw_webui; do
    mkdir -p "$repo/$crate_root/$crate/src"
    printf '[package]\nname = "%s"\n' "$crate" \
      > "$repo/$crate_root/$crate/Cargo.toml"
    printf 'pub fn placeholder() {}\n' > "$repo/$crate_root/$crate/src/lib.rs"
  done
  mkdir -p "$repo/$crate_root/ironclaw_turns/src" \
    "$repo/$crate_root/ironclaw_processes/src/journal_store" \
    "$repo/$crate_root/ironclaw_llm/src" \
    "$repo/$crate_root/ironclaw_agent_loop/src/executor" \
    "$repo/$crate_root/ironclaw_agent_loop/src/state" \
    "$repo/$crate_root/ironclaw_webui/frontend/public"
  printf 'pub fn run() {}\n' \
    | tee "$repo/$crate_root/ironclaw_turns/src/coordinator.rs" \
      "$repo/$crate_root/ironclaw_turns/src/status.rs" \
      "$repo/$crate_root/ironclaw_processes/src/supervisor.rs" \
      "$repo/$crate_root/ironclaw_processes/src/journal_store/mod.rs" \
      "$repo/$crate_root/ironclaw_llm/src/circuit_breaker.rs" \
      "$repo/$crate_root/ironclaw_llm/src/retry.rs" \
      "$repo/$crate_root/ironclaw_llm/src/failover.rs" \
      "$repo/$crate_root/ironclaw_agent_loop/src/executor/mod.rs" \
      "$repo/$crate_root/ironclaw_agent_loop/src/state/mod.rs" >/dev/null
  printf 'asset\n' > "$repo/$crate_root/ironclaw_webui/frontend/public/logo.svg"

  # Padding so discovery clears its fail-closed floor.
  for index in $(seq 1 22); do
    mkdir -p "$repo/$crate_root/ironclaw_pad$index/src"
    printf '[package]\nname = "ironclaw_pad%s"\n' "$index" \
      > "$repo/$crate_root/ironclaw_pad$index/Cargo.toml"
    printf 'pub fn placeholder() {}\n' \
      > "$repo/$crate_root/ironclaw_pad$index/src/lib.rs"
  done

  git -C "$repo" add -A
  git -C "$repo" commit -qm "feat: workspace baseline"
}

unrelated="$TMP_ROOT/unrelated"
init_repo "$unrelated"
printf 'pub fn value() -> i32 { 2 }\n' > "$unrelated/src/lib.rs"
expect_pass "unrelated feature skips the gate" run_check "$unrelated" \
  --title "feat: unrelated"

# --- high-risk detection is inventory-driven, not path-shaped (#6963) --------
# The pair below is the whole point: the same change, in a crate at two
# different depths, must reach the same verdict. The literal-prefix list this
# replaced passed the first and silently skipped the second.

high_risk="$TMP_ROOT/high-risk"
init_workspace_repo "$high_risk"
printf 'pub fn policy() {}\n' > "$high_risk/crates/ironclaw_safety/src/policy.rs"
expect_fail "high-risk feature triggers the gate" \
  "high-risk paths: crates/ironclaw_safety/src/" \
  run_check "$high_risk" --title "feat: unrelated"

high_risk_nested="$TMP_ROOT/high-risk-nested"
init_workspace_repo "$high_risk_nested" crates/substrates
printf 'pub fn policy() {}\n' \
  > "$high_risk_nested/crates/substrates/ironclaw_safety/src/policy.rs"
expect_fail "high-risk change in a family-nested crate triggers the gate" \
  "high-risk paths: crates/substrates/ironclaw_safety/src/" \
  run_check "$high_risk_nested" --title "feat: unrelated"

# A nested tree must not make everything high-risk either: the filter is still
# a filter after the repoint. Dropping the high-risk file leaves NOTES.md as
# the only change the gate sees.
rm -f "$high_risk_nested/crates/substrates/ironclaw_safety/src/policy.rs"
printf 'readme\n' > "$high_risk_nested/NOTES.md"
expect_pass "an unrelated change in a nested tree still skips the gate" \
  run_check "$high_risk_nested" --title "feat: unrelated"

# --- fail closed: an entry that resolves to nothing must be loud -------------
stale_crate="$TMP_ROOT/stale-crate"
init_workspace_repo "$stale_crate"
rm -rf "$stale_crate/crates/ironclaw_turns"
printf 'note\n' > "$stale_crate/note.txt"
expect_status "a high-risk entry naming an absent crate fails loudly" 2 \
  "names a crate this checkout does not have" \
  run_check "$stale_crate" --title "feat: unrelated"

missing_tree="$TMP_ROOT/missing-crate-tree"
init_workspace_repo "$missing_tree"
rm -rf "$missing_tree/crates"
printf 'note\n' > "$missing_tree/note.txt"
expect_status "a workspace checkout with no crate tree fails loudly" 2 \
  "crate discovery cannot run" \
  run_check "$missing_tree" --title "feat: unrelated"

# A checkout with no `[workspace]` manifest is not this workspace, so there is
# no inventory to resolve against. It says so on stderr rather than resolving
# to an empty list in silence.
non_workspace="$TMP_ROOT/non-workspace"
init_repo "$non_workspace"
printf 'pub fn value() -> i32 { 2 }\n' > "$non_workspace/src/lib.rs"
expect_pass "a non-workspace checkout announces that detection is inactive" \
  run_check "$non_workspace" --title "feat: unrelated"
if ! grep -Fq "high-risk path detection is inactive" "$TMP_ROOT/output"; then
  echo "FAIL: the non-workspace skip must be announced, not silent"
  cat "$TMP_ROOT/output"
  exit 1
fi

# Deleting the last file under a high-risk subpath is tolerated for the diff
# that does it: in CI the gate runs from the trusted base checkout, so the PR
# removing the file cannot also remove the entry the base copy still names.
# Without the escape, resolution would exit 2 here and the PR would have no
# in-PR remedy. The change also edits a *surviving* high-risk file so the run
# reaches a verdict, proving resolution tolerated the deleted prefix rather
# than refusing the whole run.
deleted_subpath="$TMP_ROOT/deleted-subpath"
init_workspace_repo "$deleted_subpath"
rm -f "$deleted_subpath/crates/ironclaw_llm/src/retry.rs"
printf 'pub fn trip() {}\n' \
  > "$deleted_subpath/crates/ironclaw_llm/src/circuit_breaker.rs"
expect_fail "deleting a high-risk file is judged, not refused" \
  "high-risk paths: crates/ironclaw_llm/src/circuit_breaker.rs" \
  run_check "$deleted_subpath" --title "feat: unrelated"

if (
  expect_fail "infrastructure error is not a policy rejection" "unused" \
    python3 "$CHECKER" --repo "$unrelated" --base missing-ref --head INDEX \
    --title "fix: infrastructure"
) >/dev/null 2>&1; then
  echo "FAIL: expect_fail accepted infrastructure exit status 2"
  exit 1
fi

meaningful="$TMP_ROOT/meaningful"
init_repo "$meaningful"
mkdir -p "$meaningful/tests"
cat > "$meaningful/tests/regression.rs" <<'EOF'
#[test]
fn preserves_the_reported_result() {
    let actual = crate_under_test();
    assert_eq!(actual, 42, "the escaped failure must stay fixed");
}
EOF
expect_pass "meaningful Rust assertion" run_check "$meaningful"

empty="$TMP_ROOT/empty"
init_repo "$empty"
mkdir -p "$empty/tests"
cat > "$empty/tests/regression.rs" <<'EOF'
#[test]
fn regression() {}
EOF
expect_fail "empty Rust test" "no meaningful changed regression assertion" \
  run_check "$empty"

tautology="$TMP_ROOT/tautology"
init_repo "$tautology"
mkdir -p "$tautology/tests"
cat > "$tautology/tests/test_regression.py" <<'EOF'
def test_regression():
    assert True
EOF
expect_fail "tautological Python assertion" \
  "no meaningful changed regression assertion" run_check "$tautology"

script_python="$TMP_ROOT/script-python"
init_repo "$script_python"
mkdir -p "$script_python/scripts"
cat > "$script_python/scripts/test-regression.py" <<'EOF'
def test_regression():
    assert run_regression() == 42
EOF
expect_pass "repository-native scripts/test-*.py assertion" \
  run_check "$script_python"

self_comparison="$TMP_ROOT/self-comparison"
init_repo "$self_comparison"
mkdir -p "$self_comparison/tests"
cat > "$self_comparison/tests/test_regression.py" <<'EOF'
def test_regression():
    result = run_regression()
    assert result == result
EOF
expect_fail "Python self-comparison" \
  "no meaningful changed regression assertion" run_check "$self_comparison"

unittest_tautology="$TMP_ROOT/unittest-tautology"
init_repo "$unittest_tautology"
mkdir -p "$unittest_tautology/tests"
cat > "$unittest_tautology/tests/test_regression.py" <<'EOF'
def test_regression(self):
    result = run_regression()
    self.assertEqual(result, result)
EOF
expect_fail "unittest self-comparison" \
  "no meaningful changed regression assertion" run_check "$unittest_tautology"

inherited_assertion="$TMP_ROOT/inherited-assertion"
init_repo "$inherited_assertion"
mkdir -p "$inherited_assertion/tests"
cat > "$inherited_assertion/tests/regression.rs" <<'EOF'
#[test]
fn existing_contract() {
    assert_eq!(crate_under_test(), 42);
}
EOF
git -C "$inherited_assertion" add tests/regression.rs
git -C "$inherited_assertion" commit -qm "test: add existing contract"
cat >> "$inherited_assertion/tests/regression.rs" <<'EOF'

fn new_setup_without_an_assertion() -> i32 {
    42
}
EOF
expect_fail "setup-only change cannot inherit an old assertion" \
  "no meaningful changed regression assertion" run_check "$inherited_assertion"

index_target="$TMP_ROOT/index-target"
init_repo "$index_target"
cat > "$index_target/src/lib.rs" <<'EOF'
pub fn value() -> i32 { 1 }

#[cfg(test)]
mod tests {
    #[test]
    fn staged_regression() {
        assert_eq!(super::value(), 1);
    }
}
EOF
git -C "$index_target" add src/lib.rs
printf 'pub fn value() -> i32 { 1 }\n' > "$index_target/src/lib.rs"
expect_pass "INDEX detection reads the staged Rust blob" \
  python3 "$CHECKER" --repo "$index_target" --base HEAD --head INDEX \
  --title "fix: staged regression"

revision_target="$TMP_ROOT/revision-target"
init_repo "$revision_target"
cat > "$revision_target/src/lib.rs" <<'EOF'
pub fn value() -> i32 { 1 }

#[cfg(test)]
mod tests {
    #[test]
    fn committed_regression() {
        assert_eq!(super::value(), 1);
    }
}
EOF
git -C "$revision_target" add src/lib.rs
git -C "$revision_target" commit -qm "fix: committed regression"
printf 'pub fn value() -> i32 { 1 }\n' > "$revision_target/src/lib.rs"
expect_pass "revision detection reads the requested Rust blob" \
  python3 "$CHECKER" --repo "$revision_target" --base HEAD^ --head HEAD \
  --title "fix: committed regression"

later_fix="$TMP_ROOT/later-fix"
init_repo "$later_fix"
printf 'pub fn value() -> i32 { 2 }\n' > "$later_fix/src/lib.rs"
git -C "$later_fix" add src/lib.rs
git -C "$later_fix" commit -qm "chore: prepare change"
printf 'pub fn value() -> i32 { 3 }\n' > "$later_fix/src/lib.rs"
git -C "$later_fix" add src/lib.rs
git -C "$later_fix" commit -qm "fix: correct later commit"
expect_fail "fix subject in a later commit triggers the gate" \
  "no meaningful changed regression assertion" \
  python3 "$CHECKER" --repo "$later_fix" --base HEAD~2 --head HEAD \
  --title "chore: neutral pull request title"

comment_only="$TMP_ROOT/comment-only"
init_repo "$comment_only"
mkdir -p "$comment_only/scripts/ci"
cat > "$comment_only/scripts/ci/test-regression.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
# grep -q "fixed" would be the assertion if this test actually ran anything.
true
EOF
expect_fail "comment-only shell assertion" \
  "no meaningful changed regression assertion" run_check "$comment_only"

typescript="$TMP_ROOT/typescript"
init_repo "$typescript"
mkdir -p "$typescript/crates/ironclaw_webui/frontend/src"
cat > "$typescript/crates/ironclaw_webui/frontend/src/regression.test.ts" <<'EOF'
test("keeps the caller-visible result", () => {
  expect(runRegression()).toEqual({ status: "fixed" });
});
EOF
expect_pass "meaningful TypeScript assertion" run_check "$typescript"

typescript_tautology="$TMP_ROOT/typescript-tautology"
init_repo "$typescript_tautology"
mkdir -p "$typescript_tautology/crates/ironclaw_webui/frontend/src"
cat > "$typescript_tautology/crates/ironclaw_webui/frontend/src/regression.test.ts" <<'EOF'
test("does not prove the regression", () => {
  const result = runRegression();
  assert(true);
  strictEqual(result, result);
});
EOF
expect_fail "tautological TypeScript assertion" \
  "no meaningful changed regression assertion" run_check "$typescript_tautology"

typescript_node_assert="$TMP_ROOT/typescript-node-assert"
init_repo "$typescript_node_assert"
mkdir -p "$typescript_node_assert/crates/ironclaw_webui/frontend/src"
while IFS='|' read -r method assertion; do
  cat > "$typescript_node_assert/crates/ironclaw_webui/frontend/src/regression.test.ts" <<EOF
import assert from "node:assert/strict";

test("recognizes node:assert methods", () => {
  $assertion
});
EOF
  expect_pass "meaningful node:assert $method assertion" \
    run_check "$typescript_node_assert"
done <<'EOF'
equal|assert.equal(actualStatus(), "fixed");
deepEqual|assert.deepEqual(actualResult(), { status: "fixed" });
ok|assert.ok(wasFixed());
match|assert.match(actualMessage(), /fixed/);
notEqual|assert.notEqual(actualStatus(), "broken");
strictEqual|assert.strictEqual(actualCount(), 1);
deepStrictEqual|assert.deepStrictEqual(actualItems(), ["fixed"]);
EOF

typescript_node_assert_tautology="$TMP_ROOT/typescript-node-assert-tautology"
init_repo "$typescript_node_assert_tautology"
mkdir -p "$typescript_node_assert_tautology/crates/ironclaw_webui/frontend/src"
cat > "$typescript_node_assert_tautology/crates/ironclaw_webui/frontend/src/regression.test.ts" <<'EOF'
import assert from "node:assert/strict";

test("does not accept tautological node:assert methods", () => {
  const result = runRegression();
  assert.equal(result, result);
  assert.deepEqual(result, result);
  assert.equal(actualStatus(), actualStatus());
  assert.deepEqual(
    buildResult({ status: "fixed", count: 1 }),
    buildResult({ status: "fixed", count: 1 }),
  );
  assert.ok(true);
  assert.ok(1);
  assert.fail();
  assert(true);
  // assert.equal(result.status, "fixed");
});
EOF
expect_fail "tautological node:assert TypeScript assertions" \
  "no meaningful changed regression assertion" \
  run_check "$typescript_node_assert_tautology"

shell_test="$TMP_ROOT/shell"
init_repo "$shell_test"
mkdir -p "$shell_test/scripts/ci"
cat > "$shell_test/scripts/ci/test-regression.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if command_under_test | grep -q "fixed"; then
  exit 0
fi
exit 1
EOF
expect_pass "meaningful shell assertion" run_check "$shell_test"

shell_guard="$TMP_ROOT/shell-guard"
init_repo "$shell_guard"
mkdir -p "$shell_guard/scripts/ci"
cat > "$shell_guard/scripts/ci/test-regression.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
fixture="${TMPDIR:-/tmp}/fixture"
if [[ -d "$fixture" ]]; then
  fixture="$fixture/existing"
fi
EOF
expect_fail "shell setup guard is not an assertion" \
  "no meaningful changed regression assertion" run_check "$shell_guard"

bare_marker="$TMP_ROOT/bare-marker"
init_repo "$bare_marker"
printf 'pub fn value() -> i32 { 2 }\n' > "$bare_marker/src/lib.rs"
expect_fail "bare skip marker" "explicit impossibility reason" \
  run_check "$bare_marker" --commit-bodies "[skip-regression-check]"

reasoned_marker="$TMP_ROOT/reasoned-marker"
init_repo "$reasoned_marker"
printf 'pub fn value() -> i32 { 2 }\n' > "$reasoned_marker/src/lib.rs"
expect_fail "unreviewed reasoned commit exemption" "non-author approving review" \
  run_check "$reasoned_marker" \
  --commit-bodies \
  "[skip-regression-check: deterministic reproduction is impossible because the failure depends on provider hardware unavailable in hermetic CI]"
expect_pass "reviewed reasoned commit exemption" run_check "$reasoned_marker" \
  --author "author" --approving-reviewers "author,reviewer" \
  --commit-bodies \
  "[skip-regression-check: deterministic reproduction is impossible because the failure depends on provider hardware unavailable in hermetic CI]"

wrapped_marker="$TMP_ROOT/wrapped-marker"
init_repo "$wrapped_marker"
printf 'pub fn value() -> i32 { 2 }\n' > "$wrapped_marker/src/lib.rs"
expect_pass "reviewed marker reason may wrap across lines" \
  run_check "$wrapped_marker" --author "author" \
  --approving-reviewers "reviewer" \
  --commit-bodies "$(printf '%s\n%s' \
    '[skip-regression-check: deterministic reproduction is impossible because' \
    'the failure depends on provider hardware unavailable in hermetic CI]')"

label_no_reason="$TMP_ROOT/label-no-reason"
init_repo "$label_no_reason"
printf 'pub fn value() -> i32 { 2 }\n' > "$label_no_reason/src/lib.rs"
expect_fail "label without reason" "explicit impossibility reason" \
  run_check "$label_no_reason" --labels "skip-regression-check" \
  --approving-reviewers "reviewer" --author "author"

label_no_review="$TMP_ROOT/label-no-review"
init_repo "$label_no_review"
printf 'pub fn value() -> i32 { 2 }\n' > "$label_no_review/src/lib.rs"
expect_fail "label without independent approval" \
  "non-author approving review" run_check "$label_no_review" \
  --labels "skip-regression-check" \
  --body "Regression-test exemption: deterministic reproduction is impossible because the production provider cannot be represented hermetically" \
  --author "author"

label_approved="$TMP_ROOT/label-approved"
init_repo "$label_approved"
printf 'pub fn value() -> i32 { 2 }\n' > "$label_approved/src/lib.rs"
expect_pass "reasoned independently approved label" run_check "$label_approved" \
  --labels "skip-regression-check" \
  --body "Regression-test exemption: deterministic reproduction is impossible because the production provider cannot be represented hermetically" \
  --author "author" --approving-reviewers "author,reviewer"

hook_repo="$TMP_ROOT/hook"
init_repo "$hook_repo"
mkdir -p "$hook_repo/scripts/ci/lib"
cp "$CHECKER" "$hook_repo/scripts/ci/regression-test-check.py"
# The gate resolves high-risk paths through the crate inventory, so its sibling
# library travels with it wherever it is deployed.
cp "$REPO_ROOT/scripts/ci/lib/crate_tree.py" "$hook_repo/scripts/ci/lib/"
printf 'pub fn value() -> i32 { 2 }\n' > "$hook_repo/src/lib.rs"
git -C "$hook_repo" add src/lib.rs
printf 'fix: escaped result\n' > "$hook_repo/COMMIT_MSG"
expect_fail "commit hook blocks a fix without regression evidence" \
  "no meaningful changed regression assertion" \
  env GIT_DIR="$hook_repo/.git" GIT_WORK_TREE="$hook_repo" \
  bash "$HOOK" "$hook_repo/COMMIT_MSG"
printf '%s\n' \
  'fix: escaped result' \
  '' \
  '[skip-regression-check: deterministic reproduction is impossible because the failure depends on provider hardware unavailable in hermetic CI]' \
  > "$hook_repo/COMMIT_MSG"
expect_pass "commit hook permits a reasoned marker pending PR review" \
  env GIT_DIR="$hook_repo/.git" GIT_WORK_TREE="$hook_repo" \
  bash "$HOOK" "$hook_repo/COMMIT_MSG"

missing_checker_repo="$TMP_ROOT/missing-checker-hook"
init_repo "$missing_checker_repo"
printf 'fix: checker unavailable\n' > "$missing_checker_repo/COMMIT_MSG"
expect_pass "commit hook defers to CI when the checker is unavailable" \
  env GIT_DIR="$missing_checker_repo/.git" GIT_WORK_TREE="$missing_checker_repo" \
  bash "$HOOK" "$missing_checker_repo/COMMIT_MSG"

echo "regression-test-check self-tests passed"
