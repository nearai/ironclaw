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

unrelated="$TMP_ROOT/unrelated"
init_repo "$unrelated"
printf 'pub fn value() -> i32 { 2 }\n' > "$unrelated/src/lib.rs"
expect_pass "unrelated feature skips the gate" run_check "$unrelated" \
  --title "feat: unrelated"

high_risk="$TMP_ROOT/high-risk"
init_repo "$high_risk"
mkdir -p "$high_risk/crates/ironclaw_safety/src"
printf 'pub fn policy() {}\n' > "$high_risk/crates/ironclaw_safety/src/policy.rs"
expect_fail "high-risk feature triggers the gate" \
  "Regression test required (high-risk change; high-risk paths:" \
  run_check "$high_risk" --title "feat: unrelated"

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
mkdir -p "$hook_repo/scripts/ci"
cp "$CHECKER" "$hook_repo/scripts/ci/regression-test-check.py"
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

echo "regression-test-check self-tests passed"
