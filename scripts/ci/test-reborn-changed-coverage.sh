#!/usr/bin/env bash
# Hermetic sabotage tests for the changed-line/changed-branch gate.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="${repo_root}/scripts/ci/reborn_changed_coverage.py"
work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-changed-cov.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

passes=0
failures=0
capture() {
  set +e
  CAP_OUT="$("$@" 2>&1)"
  CAP_RC=$?
  set -e
}
check_rc() {
  local label="$1" expected="$2"
  if [ "${CAP_RC}" -eq "${expected}" ]; then
    echo "  ok   ${label}"
    passes=$((passes + 1))
  else
    echo "  FAIL ${label}: expected rc=${expected}, got ${CAP_RC}" >&2
    printf '%s\n' "${CAP_OUT}" >&2
    failures=$((failures + 1))
  fi
}
check_text() {
  local label="$1" needle="$2"
  if grep -Fq "${needle}" <<<"${CAP_OUT}"; then
    echo "  ok   ${label}"
    passes=$((passes + 1))
  else
    echo "  FAIL ${label}: missing ${needle}" >&2
    printf '%s\n' "${CAP_OUT}" >&2
    failures=$((failures + 1))
  fi
}
check_report_text() {
  local label="$1" needle="$2"
  if grep -Fq "${needle}" "${work}/report.json"; then
    echo "  ok   ${label}"
    passes=$((passes + 1))
  else
    echo "  FAIL ${label}: report missing ${needle}" >&2
    cat "${work}/report.json" >&2
    failures=$((failures + 1))
  fi
}

case_root="${work}/repo"
source_path="crates/ironclaw_demo/src/lib.rs"
mkdir -p "${case_root}/crates/ironclaw_demo/src"
printf '%s\n' 'pub fn classify(value: bool) -> bool {' '    value' '}' >"${case_root}/${source_path}"

cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
branch_percent = 100.0
TOML

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,3 @@
+pub fn classify(value: bool) -> bool {
+    value
+}
DIFF

write_lcov() {
  local line_hits="$1" first_branch="$2" second_branch="$3"
  cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
DA:2,${line_hits}
DA:3,1
BRDA:2,0,0,${first_branch}
BRDA:2,0,1,${second_branch}
LF:3
LH:3
BRF:2
BRH:2
end_of_record
EOF
}

run_gate() {
  capture python3 "${gate}" \
    --lcov "${work}/coverage.lcov" \
    --manifest "${work}/policy.toml" \
    --diff-file "${work}/change.diff" \
    --repo-root "${case_root}" \
    --json "${work}/report.json"
}

echo "▶ changed coverage happy path"
write_lcov 1 1 1
run_gate
check_rc "fully covered changed lines and branches pass" 0
check_text "line denominator is reported" "Changed line coverage: 100.00% (3/3)"
check_text "branch denominator is reported" "Changed branch coverage: 100.00% (2/2)"
check_report_text "machine report preserves the branch denominator" '"instrumented_branches": 2'

echo "▶ line and branch sabotage"
write_lcov 0 1 1
run_gate
check_rc "an uncovered changed line fails" 1
check_text "line sabotage names the exact source line" "${source_path}:2"

write_lcov 1 1 0
run_gate
check_rc "an uncovered changed branch fails" 1
check_text "branch sabotage names the exact branch" "${source_path}:2 branch 0/1"

echo "▶ missing branch instrumentation is loud"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
DA:2,1
DA:3,1
LF:3
LH:3
end_of_record
EOF
run_gate
check_rc "LCOV without BRDA records fails" 1
check_text "missing BRDA explains instrumentation failure" "branch instrumentation is missing"

echo "▶ explicit reviewed exemptions are exact-line only"
cat >"${work}/policy.toml" <<TOML
[policy]
line_percent = 100.0
branch_percent = 100.0

[[exemption]]
path = "${source_path}"
lines = [2]
branch_lines = [2]
owner = "@nearai/testing"
reason = "Synthetic self-test exemption."
issue = "https://github.com/nearai/ironclaw/issues/6524"
review_after = "2099-01-01"
TOML
write_lcov 0 0 0
run_gate
check_rc "an owned exact-line exemption removes only that denominator" 0

echo "▶ malformed and stale exemption fixtures fail"
cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
branch_percent = 100.0

[[exemption]]
path = "crates/ironclaw_demo/src/moved.rs"
lines = [2]
owner = "@nearai/testing"
reason = "Synthetic stale path."
issue = "https://github.com/nearai/ironclaw/issues/6524"
review_after = "2099-01-01"
TOML
write_lcov 1 1 1
run_gate
check_rc "a stale exemption path fails" 1
check_text "stale path is actionable" "names stale path"

cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
TOML
run_gate
check_rc "a malformed policy fails" 1
check_text "malformed policy names exact fields" "[policy] fields must be exactly"

echo "▶ missing production coverage cannot disappear from the denominator"
cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
branch_percent = 100.0
TOML
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/crates/ironclaw_other/src/lib.rs
DA:1,1
BRDA:1,0,0,1
LF:1
LH:1
BRF:1
BRH:1
end_of_record
EOF
run_gate
check_rc "a changed production file absent from LCOV fails" 1
check_text "missing file is named" "changed production files are absent from coverage"
check_report_text "machine report cannot turn a missing file into a pass" '"passed": false'

echo "▶ test-only Rust changes do not dilute the production denominator"
test_source="crates/ironclaw_demo/src/tests.rs"
printf '%s\n' '#[test]' 'fn helper_test() {}' >"${case_root}/${test_source}"
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/tests.rs b/crates/ironclaw_demo/src/tests.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/tests.rs
@@ -0,0 +1,2 @@
+#[test]
+fn helper_test() {}
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- a/crates/ironclaw_demo/src/lib.rs
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -3,0 +4,4 @@
+#[cfg(test)]
+mod inline_tests {
+    fn helper() {}
+}
DIFF
printf '%s\n' \
  'pub fn classify(value: bool) -> bool {' \
  '    value' \
  '}' \
  '#[cfg(test)]' \
  'mod inline_tests {' \
  '    fn helper() {}' \
  '}' >"${case_root}/${source_path}"
run_gate
check_rc "test modules are excluded mechanically" 0
check_text "test-only result is explicit" "no Reborn production lines added"
check_report_text "machine report records the empty production diff" '"changed_product_files": []'

echo "▶ non-test cfg remains in the production denominator"
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,4 @@
+#[cfg(not(test))]
+pub fn production_only(value: bool) -> bool {
+    value
+}
DIFF
printf '%s\n' \
  '#[cfg(not(test))]' \
  'pub fn production_only(value: bool) -> bool {' \
  '    value' \
  '}' >"${case_root}/${source_path}"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
DA:2,1
DA:3,1
DA:4,1
BRDA:3,0,0,1
BRDA:3,0,1,1
LF:4
LH:4
BRF:2
BRH:2
end_of_record
EOF
run_gate
check_rc "cfg(not(test)) production code is gated" 0
check_text "production cfg lines remain counted" "Changed line coverage: 100.00% (4/4)"

echo "▶ renamed production files remain in the denominator"
renamed_path="crates/ironclaw_demo/src/renamed.rs"
printf '%s\n' \
  'pub fn stable_one() -> bool { true }' \
  'pub fn stable_two() -> bool { true }' \
  'pub fn stable_three() -> bool { true }' \
  'pub fn stable_four() -> bool { true }' \
  'pub fn stable_five() -> bool { true }' \
  'pub fn stable_six() -> bool { true }' \
  'pub fn stable_seven() -> bool { true }' \
  'pub fn stable_eight() -> bool { true }' \
  'pub fn stable_nine() -> bool { true }' \
  'pub fn stable_ten() -> bool { true }' >"${case_root}/${source_path}"
git -C "${case_root}" init -q
git -C "${case_root}" add "${source_path}"
git -C "${case_root}" \
  -c user.name=coverage-test -c user.email=coverage@example.invalid \
  commit -qm baseline
base_commit="$(git -C "${case_root}" rev-parse HEAD)"
git -C "${case_root}" mv "${source_path}" "${renamed_path}"
printf '%s\n' 'pub fn renamed_branch(value: bool) -> bool { value }' \
  >>"${case_root}/${renamed_path}"
git -C "${case_root}" add "${renamed_path}"
git -C "${case_root}" \
  -c user.name=coverage-test -c user.email=coverage@example.invalid \
  commit -qm renamed
head_commit="$(git -C "${case_root}" rev-parse HEAD)"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${renamed_path}
DA:11,0
BRDA:11,0,0,1
BRDA:11,0,1,0
LF:1
LH:0
BRF:2
BRH:1
end_of_record
EOF
capture python3 "${gate}" \
  --lcov "${work}/coverage.lcov" \
  --manifest "${work}/policy.toml" \
  --base "${base_commit}" \
  --head "${head_commit}" \
  --repo-root "${case_root}"
check_rc "an uncovered line added during a rename fails" 1
check_text "rename sabotage names the new source path" "${renamed_path}:11"

echo
if [ "${failures}" -ne 0 ]; then
  echo "${failures} changed-coverage self-test(s) failed" >&2
  exit 1
fi
echo "all ${passes} changed-coverage self-tests passed"
