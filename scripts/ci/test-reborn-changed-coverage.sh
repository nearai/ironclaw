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
  if grep -Fq -e "${needle}" <<<"${CAP_OUT}"; then
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
  if grep -Fq -e "${needle}" "${work}/report.json"; then
    echo "  ok   ${label}"
    passes=$((passes + 1))
  else
    echo "  FAIL ${label}: report missing ${needle}" >&2
    cat "${work}/report.json" >&2
    failures=$((failures + 1))
  fi
}
check_no_report_text() {
  local label="$1" needle="$2"
  if grep -Fq -e "${needle}" "${work}/report.json"; then
    echo "  FAIL ${label}: report unexpectedly contains ${needle}" >&2
    cat "${work}/report.json" >&2
    failures=$((failures + 1))
  else
    echo "  ok   ${label}"
    passes=$((passes + 1))
  fi
}

case_root="${work}/repo"
source_path="crates/ironclaw_demo/src/lib.rs"
mkdir -p "${case_root}/crates/ironclaw_demo/src"
printf '%s\n' 'pub fn classify(value: bool) -> bool {' '    value' '}' >"${case_root}/${source_path}"

# The gate resolves "is this a Reborn production source?" from the crate
# inventory (scripts/ci/lib/crate_tree.py) rather than from a
# `crates/ironclaw_*` pattern, so the fixture has to be a real crate tree:
# a Cargo.toml per crate, and enough of them to clear crate_tree's
# MIN_CRATE_DIRECTORIES discovery floor. Padding the fixture up to the floor is
# deliberate — the alternative (lowering or bypassing the floor for tests) would
# retire the very fail-closed assertion these tests exist to pin.
write_crate_manifest() {
  local crate_dir="$1"
  mkdir -p "${case_root}/${crate_dir}/src"
  printf '[package]\nname = "%s"\n' "$(basename "${crate_dir}")" \
    >"${case_root}/${crate_dir}/Cargo.toml"
}
write_crate_manifest crates/ironclaw_demo
for pad_index in $(seq 1 24); do
  write_crate_manifest "crates/ironclaw_pad${pad_index}"
done

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

echo "▶ restored original changed-line floor"
cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 90.0
branch_percent = 0.0
TOML
threshold_lines=20
: >"${case_root}/${source_path}"
: >"${work}/change.diff"
printf '%s\n' \
  "diff --git a/${source_path} b/${source_path}" \
  "--- /dev/null" \
  "+++ b/${source_path}" \
  "@@ -0,0 +1,${threshold_lines} @@" >>"${work}/change.diff"
for line in $(seq 1 "${threshold_lines}"); do
  printf 'pub fn threshold_line_%s() {}\n' "${line}" >>"${case_root}/${source_path}"
  printf '+pub fn threshold_line_%s() {}\n' "${line}" >>"${work}/change.diff"
done
write_threshold_lcov() {
  local line_18_hits="$1"
  printf 'SF:%s\n' "${case_root}/${source_path}" >"${work}/coverage.lcov"
  for line in $(seq 1 17); do
    printf 'DA:%s,1\n' "${line}" >>"${work}/coverage.lcov"
  done
  printf 'DA:18,%s\nDA:19,0\nDA:20,0\n' "${line_18_hits}" \
    >>"${work}/coverage.lcov"
  for line in $(seq 1 20); do
    printf 'BRDA:%s,0,0,0\n' "${line}" >>"${work}/coverage.lcov"
  done
  printf 'LF:20\nBRF:20\nend_of_record\n' >>"${work}/coverage.lcov"
}

write_threshold_lcov 1
run_gate
check_rc "90% changed lines pass at the original floor" 0
check_text "line floor denominator is reported" "Changed line coverage: 90.00% (18/20)"
check_text "ungated branch coverage remains visible" "Changed branch coverage: 0.00% (0/20)"
check_text "uncovered branch detail remains visible" "${source_path}:1 branch 0/0"
check_report_text "machine report records the 90% line floor" '"threshold_percent": 90.0'
check_report_text "machine report records the zero branch floor" '"branch_threshold_percent": 0.0'

write_threshold_lcov 0
run_gate
check_rc "changed-line coverage below 90% fails" 1
check_text "line-floor failure names the original threshold" "line coverage 85.00% is below 90.0%"

printf '%s\n' 'pub fn classify(value: bool) -> bool {' '    value' '}' >"${case_root}/${source_path}"
cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
branch_percent = 100.0
TOML

echo "▶ diff markers inside hunk content are parsed by their first byte"
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- a/crates/ironclaw_demo/src/lib.rs
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -1,1 +1,1 @@
---removed_content
+++added_content
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
BRDA:1,0,0,1
BRDA:1,0,1,1
LF:1
LH:1
BRF:2
BRH:2
end_of_record
EOF
run_gate
check_rc "added content beginning with ++ is counted and removed -- content is skipped" 0
check_text "the marker-like added content contributes one line" "Changed line coverage: 100.00% (1/1)"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- a/crates/ironclaw_demo/src/lib.rs
+++ b/crates/ironclaw_demo/src/lib.rs
@@ malformed @@
+pub fn malformed() {}
DIFF
run_gate
check_rc "a malformed production hunk fails" 1
check_text "malformed hunk failure names its header" "malformed diff hunk header"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,3 @@
+pub fn classify(value: bool) -> bool {
+    value
+}
DIFF

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

cat >"${work}/change.diff" <<'DIFF'
DIFF
run_gate
check_rc "missing branch instrumentation also fails for an empty production diff" 1
check_text "empty production diff cannot bypass BRDA validation" "branch instrumentation is missing"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,3 @@
+pub fn classify(value: bool) -> bool {
+    value
+}
DIFF

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

reexports_path="crates/ironclaw_demo/src/reexports.rs"
printf '%s\n' \
  'pub use crate::service::Service;' \
  'pub(crate) mod support;' >"${case_root}/${reexports_path}"
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/reexports.rs b/crates/ironclaw_demo/src/reexports.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/reexports.rs
@@ -0,0 +1,2 @@
+pub use crate::service::Service;
+pub(crate) mod support;
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
BRDA:1,0,0,1
LF:1
LH:1
BRF:1
BRH:1
end_of_record
EOF
run_gate
check_rc "an uninstrumentable re-export module in a measured crate passes" 0
check_text "re-export-only additions keep an explicit empty denominator" "Changed line coverage: 100.00% (0/0)"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,3 @@
+pub fn classify(value: bool) -> bool {
+    value
+}
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
end_of_record
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
check_rc "an empty SF block for the changed file fails" 1
check_text "empty SF block is reported as uninstrumented" "contain no DA records"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- a/crates/ironclaw_demo/src/lib.rs
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -1,0 +2,1 @@
+const UNMEASURED: bool = true;
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
DA:3,1
BRDA:3,0,0,1
LF:2
LH:2
BRF:1
BRH:1
end_of_record
EOF
run_gate
check_rc "a changed file with no measured changed lines fails" 1
check_text "zero per-file denominator is actionable" "contributed no instrumented lines"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- a/crates/ironclaw_demo/src/lib.rs
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,2 @@
+/// Documents the executable item below.
+use std::fmt::Debug;
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:5,1
DA:6,1
BRDA:6,0,0,1
LF:2
LH:2
BRF:1
BRH:1
end_of_record
EOF
run_gate
check_rc "imports and doc comments outside the executable span pass" 0
check_text "uninstrumentable-only additions keep an explicit empty denominator" "Changed line coverage: 100.00% (0/0)"

cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- a/crates/ironclaw_demo/src/lib.rs
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -1,0 +2,14 @@
+
+/// Documents an item inside the executable span.
+/*
+ * A multiline block comment is not executable.
+ */
+#[derive(
+    Debug,
+    Clone,
+)]
+pub use std::fmt::{
+    Debug,
+    Display,
+};
+}
DIFF
printf '%s\n' \
  'pub fn before() {}' \
  '' \
  '/// Documents an item inside the executable span.' \
  '/*' \
  ' * A multiline block comment is not executable.' \
  ' */' \
  '#[derive(' \
  '    Debug,' \
  '    Clone,' \
  ')]' \
  'pub use std::fmt::{' \
  '    Debug,' \
  '    Display,' \
  '};' \
  '}' \
  'pub fn after() {}' >"${case_root}/${source_path}"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:1,1
DA:16,1
BRDA:16,0,0,1
LF:2
LH:2
BRF:1
BRH:1
end_of_record
EOF
run_gate
check_rc "scaffolding-only additions inside the executable span pass" 0
check_text "in-span scaffolding cannot manufacture a denominator" "Changed line coverage: 100.00% (0/0)"

echo "▶ test-only Rust changes do not dilute the production denominator"
test_source="crates/ironclaw_demo/src/tests.rs"
e2e_test_source="crates/ironclaw_demo/src/channel_host/e2e_tests.rs"
printf '%s\n' '#[test]' 'fn helper_test() {}' >"${case_root}/${test_source}"
mkdir -p "$(dirname "${case_root}/${e2e_test_source}")"
printf '%s\n' '#[test]' 'fn e2e_helper_test() {}' >"${case_root}/${e2e_test_source}"
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/tests.rs b/crates/ironclaw_demo/src/tests.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/tests.rs
@@ -0,0 +1,2 @@
+#[test]
+fn helper_test() {}
diff --git a/crates/ironclaw_demo/src/channel_host/e2e_tests.rs b/crates/ironclaw_demo/src/channel_host/e2e_tests.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/channel_host/e2e_tests.rs
@@ -0,0 +1,2 @@
+#[test]
+fn e2e_helper_test() {}
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

echo "▶ cfg(test) span detection ignores braces in comments and literals"
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/ironclaw_demo/src/lib.rs b/crates/ironclaw_demo/src/lib.rs
--- /dev/null
+++ b/crates/ironclaw_demo/src/lib.rs
@@ -0,0 +1,11 @@
+#[cfg(test)]
+mod inline_tests {
+    // A comment brace must not end the module: }
+    const NORMAL: &str = "}";
+    const RAW: &str = r#"}"#;
+    const CHARACTER: char = '}';
+    fn helper() {}
+}
+pub fn production_after_test_module() -> bool {
+    true
+}
DIFF
printf '%s\n' \
  '#[cfg(test)]' \
  'mod inline_tests {' \
  '    // A comment brace must not end the module: }' \
  '    const NORMAL: &str = "}";' \
  '    const RAW: &str = r#"}"#;' \
  "    const CHARACTER: char = '}';" \
  '    fn helper() {}' \
  '}' \
  'pub fn production_after_test_module() -> bool {' \
  '    true' \
  '}' >"${case_root}/${source_path}"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${source_path}
DA:9,1
DA:10,1
DA:11,1
BRDA:10,0,0,1
LF:3
LH:3
BRF:1
BRH:1
end_of_record
EOF
run_gate
check_rc "literal and comment braces do not truncate cfg(test) exclusion" 0
check_text "production after cfg(test) remains in the denominator" "Changed line coverage: 100.00% (3/3)"

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
fixture_git() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
    git -c core.hooksPath=/dev/null -C "${case_root}" "$@"
}
fixture_git init -q --template=
fixture_git add "${source_path}"
fixture_git \
  -c user.name=coverage-test -c user.email=coverage@example.invalid -c commit.gpgsign=false \
  commit -qm baseline
base_commit="$(fixture_git rev-parse HEAD)"
fixture_git mv "${source_path}" "${renamed_path}"
printf '%s\n' 'pub fn renamed_branch(value: bool) -> bool { value }' \
  >>"${case_root}/${renamed_path}"
fixture_git add "${renamed_path}"
fixture_git \
  -c user.name=coverage-test -c user.email=coverage@example.invalid -c commit.gpgsign=false \
  commit -qm renamed
head_commit="$(fixture_git rev-parse HEAD)"
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

echo "▶ the changed-line denominator is computed with the histogram diff algorithm"
# Myers (git's default) anchors greedily. On a deletion-shaped diff it shreds one
# large removal into interleaved -/+ hunks, re-emitting surviving *unchanged* text
# as added lines — which this gate then demands coverage for. Found on #6964, where
# deleting the dead half of llm::reasoning made myers report 907 added lines in a
# file whose real change was 8 (all doc comments and imports, zero executable).
#
# This asserts the invocation rather than re-staging a myers pathology on purpose:
# the pathology depends on git's internal heuristics, so a fixture built around one
# can quietly stop reproducing on a future git and leave a vacuous green test. The
# flag is the actual contract, so pin the flag.
shim_bin="${work}/shim-bin"
mkdir -p "${shim_bin}"
real_git="$(command -v git)"
cat >"${shim_bin}/git" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"${work}/git-argv.log"
exec "${real_git}" "\$@"
EOF
chmod +x "${shim_bin}/git"
: >"${work}/git-argv.log"
PATH="${shim_bin}:${PATH}" python3 "${gate}" \
  --lcov "${work}/coverage.lcov" \
  --manifest "${work}/policy.toml" \
  --base "${base_commit}" \
  --head "${head_commit}" \
  --repo-root "${case_root}" >/dev/null 2>&1 || true
capture grep -Fq -- "--diff-algorithm=histogram" "${work}/git-argv.log"
check_rc "the gate pins the histogram diff algorithm when it generates the diff" 0
capture grep -Eq -- "diff .*--unified=0" "${work}/git-argv.log"
check_rc "the gate still generates the diff with zero context" 0

echo "▶ discovery is tree-shape-agnostic and fails closed"
# The WS10 failure mode (docs/internal/reborn/target-architecture/CHECKLIST.md, #6963):
# with the old `crates/ironclaw_*/src/**` keying, every case below reported
# "no Reborn production lines added" and exited 0 — a green gate that measured
# nothing. Coverage numbers here are deliberately identical to the flat-tree
# happy path, because the tree shape must not change what the gate measures.
nested_path="crates/substrates/ironclaw_nested/src/lib.rs"
write_crate_manifest crates/substrates/ironclaw_nested
printf '%s\n' 'pub fn classify(value: bool) -> bool {' '    value' '}' \
  >"${case_root}/${nested_path}"
cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
branch_percent = 100.0
TOML
cat >"${work}/change.diff" <<DIFF
diff --git a/${nested_path} b/${nested_path}
--- /dev/null
+++ b/${nested_path}
@@ -0,0 +1,3 @@
+pub fn classify(value: bool) -> bool {
+    value
+}
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${nested_path}
DA:1,1
DA:2,0
DA:3,1
BRDA:2,0,0,1
BRDA:2,0,1,1
LF:3
LH:2
BRF:2
BRH:2
end_of_record
EOF
run_gate
check_rc "an uncovered line in a family-nested crate still fails the gate" 1
check_text "the nested crate is measured, not skipped" "Changed line coverage: 66.67% (2/3)"
check_text "the nested uncovered line is named" "${nested_path}:2"

# A crate-owned path that is not the crate's own `src/` was outside the
# denominator under the old regex (`crates/<one-segment>/src/…`) and must stay
# outside it: `crates/ironclaw_safety/fuzz/src/main.rs` is the real instance.
# It is attributable — so it must be *excluded*, not *refused*.
fuzz_path="crates/ironclaw_demo/fuzz/src/main.rs"
mkdir -p "$(dirname "${case_root}/${fuzz_path}")"
printf '%s\n' 'fn main() {}' >"${case_root}/${fuzz_path}"
cat >"${work}/change.diff" <<DIFF
diff --git a/${fuzz_path} b/${fuzz_path}
--- /dev/null
+++ b/${fuzz_path}
@@ -0,0 +1,1 @@
+fn main() {}
DIFF
run_gate
check_rc "a nested non-src tree inside a crate stays out of the denominator" 0
check_text "the crate-owned non-src path is excluded, not refused" \
  "no Reborn production lines added"

# The fail-closed half: a `crates/` Rust file no crate owns means the inventory
# and the tree disagree. Falling through to "not production" is precisely how a
# moved tree goes quiet, so it is refused instead.
cat >"${work}/change.diff" <<'DIFF'
diff --git a/crates/not_a_crate/src/lib.rs b/crates/not_a_crate/src/lib.rs
--- /dev/null
+++ b/crates/not_a_crate/src/lib.rs
@@ -0,0 +1,1 @@
+pub fn orphan() {}
DIFF
run_gate
check_rc "an unattributable crates/ Rust file fails closed" 1
check_text "the unattributable path is named" "belongs to no discovered crate"

# ...and the same refusal must reach the mode CI actually runs. `--diff-file`
# hands the gate an un-narrowed diff, but `--base/--head` narrows to per-crate
# `src/` pathspecs BEFORE `parse_diff` ever sees a path — so an unattributable
# file was filtered out of the diff text and the check above could not fire at
# all in production. Verified against this fixture: the gate printed
# "no Reborn production lines added" and exited 0. `screen_unattributable`
# closes that, and this case is the pin: a fail-closed check that cannot fail
# in the mode that matters is not a check (#6963).
orphan_path="crates/not_a_crate/src/lib.rs"
mkdir -p "$(dirname "${case_root}/${orphan_path}")"
printf '%s\n' 'pub fn orphan() {}' >"${case_root}/${orphan_path}"
fixture_git add "${orphan_path}"
fixture_git \
  -c user.name=coverage-test -c user.email=coverage@example.invalid -c commit.gpgsign=false \
  commit -qm orphan
orphan_commit="$(fixture_git rev-parse HEAD)"
capture python3 "${gate}" \
  --lcov "${work}/coverage.lcov" \
  --manifest "${work}/policy.toml" \
  --base "${head_commit}" \
  --head "${orphan_commit}" \
  --repo-root "${case_root}"
check_rc "an unattributable path is refused through --base/--head too" 1
check_text "the --base/--head refusal names the path" "${orphan_path}"
fixture_git rm -rq "$(dirname "${orphan_path}")"
fixture_git \
  -c user.name=coverage-test -c user.email=coverage@example.invalid -c commit.gpgsign=false \
  commit -qm drop-orphan

# A missing or truncated crate tree cannot read as "nothing changed".
empty_root="${work}/no-crates"
mkdir -p "${empty_root}"
capture python3 "${gate}" \
  --lcov "${work}/coverage.lcov" \
  --manifest "${work}/policy.toml" \
  --diff-file "${work}/change.diff" \
  --repo-root "${empty_root}"
check_rc "a repo root with no crates/ tree fails closed" 1
check_text "missing crate tree is actionable" "crate discovery failed"

short_root="${work}/short-crates"
mkdir -p "${short_root}/crates/ironclaw_lonely/src"
printf '[package]\nname = "ironclaw_lonely"\n' \
  >"${short_root}/crates/ironclaw_lonely/Cargo.toml"
capture python3 "${gate}" \
  --lcov "${work}/coverage.lcov" \
  --manifest "${work}/policy.toml" \
  --diff-file "${work}/change.diff" \
  --repo-root "${short_root}"
check_rc "a crate inventory below the discovery floor fails closed" 1
check_text "short crate tree is actionable" "crate discovery failed"

echo "▶ lines already uncovered at base leave the denominator; nothing else does"
# The rule: a changed line whose pre-image was uncovered at the base commit is
# pre-existing debt, not debt this change introduced. Everything below pins one
# of the four ways a line can relate to base, because the value of the rule is
# entirely in what it refuses to forgive.
pre_crate="crates/ironclaw_preimage"
pre_path="${pre_crate}/src/lib.rs"
write_crate_manifest "${pre_crate}"
# Written here rather than inherited: every assertion below is an exact
# denominator, so an exemption left over from an earlier section would move the
# numbers without failing anything.
cat >"${work}/policy.toml" <<'TOML'
[policy]
line_percent = 100.0
branch_percent = 100.0
TOML
printf '%s\n' \
  'pub fn alpha(value: bool) -> bool {' \
  '    value' \
  '}' \
  'pub fn beta(value: bool) -> bool {' \
  '    !value' \
  '}' >"${case_root}/${pre_path}"

# Two lines modified in place, 1:1 — the rename / rustfmt-re-wrap shape that
# made 135 of PR #7000's 137 flagged lines pre-existing.
cat >"${work}/change.diff" <<DIFF
diff --git a/${pre_path} b/${pre_path}
--- a/${pre_path}
+++ b/${pre_path}
@@ -2 +2 @@
-    old_value
+    value
@@ -5 +5 @@
-    !old_value
+    !value
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,0
DA:5,1
BRDA:5,0,0,1
BRDA:5,0,1,1
LF:2
LH:1
BRF:2
BRH:2
end_of_record
EOF
# Line 2 uncovered at base; line 5 covered at base and still covered.
cat >"${work}/base.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,0
DA:5,1
LF:2
LH:1
end_of_record
EOF
run_gate_base() {
  capture python3 "${gate}" \
    --lcov "${work}/coverage.lcov" \
    --manifest "${work}/policy.toml" \
    --diff-file "${work}/change.diff" \
    --repo-root "${case_root}" \
    --json "${work}/report.json" \
    "$@"
}
run_gate_base --base-lcov "${work}/base.lcov"
check_rc "a line uncovered at base and uncovered now is excluded, not gated" 0
check_text "the subtraction count is reported" \
  "Pre-existing uncovered lines excluded from the denominator: 1"
check_text "the excluded line names its base pre-image for audit" \
  "${pre_path}:2 (uncovered at base as ${pre_path}:2)"
check_text "only the genuinely measurable line remains in the denominator" \
  "Changed line coverage: 100.00% (1/1)"
check_report_text "the machine report carries the exclusion count" \
  '"preexisting_uncovered_excluded": 1'
check_report_text "the machine report records that base coverage was applied" \
  '"base_coverage_applied": true'

# Uncovered at base but covered now: someone paid off the debt in this change.
# Excluding it would strip a hit from the numerator and shrink the denominator,
# i.e. quietly penalise adding the missing test, so only currently-uncovered
# lines are ever candidates for subtraction.
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,3
DA:5,1
BRDA:5,0,0,1
BRDA:5,0,1,1
LF:2
LH:2
BRF:2
BRH:2
end_of_record
EOF
run_gate_base --base-lcov "${work}/base.lcov"
check_rc "a pre-existing hole this change filled still passes" 0
check_text "the newly covered line stays in the denominator" \
  "Changed line coverage: 100.00% (2/2)"
check_text "a line that is covered now is never subtracted" \
  "Pre-existing uncovered lines excluded from the denominator: 0"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,0
DA:5,1
BRDA:5,0,0,1
BRDA:5,0,1,1
LF:2
LH:1
BRF:2
BRH:2
end_of_record
EOF

# The regression this whole gate exists for: covered before, uncovered now.
cat >"${work}/base.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,1
DA:5,1
LF:2
LH:2
end_of_record
EOF
run_gate_base --base-lcov "${work}/base.lcov"
check_rc "a line covered at base and uncovered now still fails" 1
check_text "the covered-at-base regression is named" "${pre_path}:2"
check_text "nothing is excluded when base coverage says the line was covered" \
  "Pre-existing uncovered lines excluded from the denominator: 0"

# A pure addition has no pre-image, so it can inherit nothing. The base lcov
# deliberately marks the *same line number* uncovered: a gate that keyed on the
# line number rather than the diff's pre-image would wrongly forgive this.
cat >"${work}/change.diff" <<DIFF
diff --git a/${pre_path} b/${pre_path}
--- a/${pre_path}
+++ b/${pre_path}
@@ -1,0 +2 @@
+    value
DIFF
cat >"${work}/base.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,0
DA:5,0
LF:2
LH:0
end_of_record
EOF
run_gate_base --base-lcov "${work}/base.lcov"
check_rc "a genuinely new uncovered line still fails" 1
check_text "the genuinely new line is named" "${pre_path}:2"
check_text "a line with no pre-image is never excluded" \
  "Pre-existing uncovered lines excluded from the denominator: 0"

echo "▶ pre-images resolve across a rename"
moved_from="${pre_crate}/src/moved_from.rs"
moved_to="${pre_crate}/src/moved_to.rs"
printf '%s\n' \
  'pub fn one(value: bool) -> bool {' \
  '    value' \
  '}' \
  'pub fn two(value: bool) -> bool {' \
  '    !value' \
  '}' >"${case_root}/${moved_from}"
fixture_git add "${moved_from}"
fixture_git \
  -c user.name=coverage-test -c user.email=coverage@example.invalid -c commit.gpgsign=false \
  commit -qm preimage-baseline
preimage_base="$(fixture_git rev-parse HEAD)"
fixture_git mv "${moved_from}" "${moved_to}"
printf '%s\n' \
  'pub fn one(value: bool) -> bool {' \
  '    value && true' \
  '}' \
  'pub fn two(value: bool) -> bool {' \
  '    !value || false' \
  '}' >"${case_root}/${moved_to}"
fixture_git add "${moved_to}"
fixture_git \
  -c user.name=coverage-test -c user.email=coverage@example.invalid -c commit.gpgsign=false \
  commit -qm preimage-renamed
preimage_head="$(fixture_git rev-parse HEAD)"
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${moved_to}
DA:2,0
DA:5,0
BRDA:2,0,0,1
BRDA:2,0,1,1
LF:2
LH:0
BRF:2
BRH:2
end_of_record
EOF
# At the OLD path: line 2 covered, line 5 uncovered.
cat >"${work}/base.lcov" <<EOF
SF:${case_root}/${moved_from}
DA:2,1
DA:5,0
LF:2
LH:1
end_of_record
EOF
capture python3 "${gate}" \
  --lcov "${work}/coverage.lcov" \
  --manifest "${work}/policy.toml" \
  --base "${preimage_base}" \
  --head "${preimage_head}" \
  --repo-root "${case_root}" \
  --json "${work}/report.json" \
  --base-lcov "${work}/base.lcov"
check_rc "a renamed file's covered-at-base line still gates" 1
check_text "the renamed covered-at-base line is named" "${moved_to}:2"
check_report_text "the covered-at-base line is in the machine report's holes" \
  "\"${moved_to}:2\""
# The uncovered-at-base sibling must be excluded, not merely absent because the
# rename lost its pre-image: the next assertion pins that it resolved to the old
# path, so the two together separate "forgiven" from "never seen".
check_no_report_text "the renamed uncovered-at-base line is not gated" \
  "\"${moved_to}:5\""
check_text "the rename's pre-image is resolved to the old path" \
  "${moved_to}:5 (uncovered at base as ${moved_from}:5)"

echo "▶ unobtainable base coverage falls back to counting every changed line"
# The fallback is the whole safety argument: the subtraction may only ever come
# from coverage the gate positively read. Every failure mode below must land on
# *current* behaviour — stricter, never looser — and say so out loud.
cat >"${work}/change.diff" <<DIFF
diff --git a/${pre_path} b/${pre_path}
--- a/${pre_path}
+++ b/${pre_path}
@@ -2 +2 @@
-    old_value
+    value
@@ -5 +5 @@
-    !old_value
+    !value
DIFF
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,0
DA:5,1
BRDA:5,0,0,1
BRDA:5,0,1,1
LF:2
LH:1
BRF:2
BRH:2
end_of_record
EOF
run_gate_base --base-lcov "${work}/absent.lcov"
check_rc "a missing base lcov counts every changed line" 1
check_text "the missing base lcov is announced, not swallowed" "Base coverage: NOT APPLIED"
check_text "the fallback explains itself" "--base-lcov not found"
check_text "the fallback says every changed line counts" \
  "every changed line counts, including lines that were already uncovered"
check_text "the previously excluded line is back in the denominator" "${pre_path}:2"
check_report_text "the machine report records the fallback" \
  '"base_coverage_applied": false'

printf '%s\n' 'TN:' 'SF:/nowhere.rs' 'end_of_record' >"${work}/empty.lcov"
run_gate_base --base-lcov "${work}/empty.lcov"
check_rc "a base lcov with no DA records counts every changed line" 1
check_text "an lcov that measured nothing is refused as base coverage" \
  "contains no DA records"

run_gate_base
check_rc "no base coverage requested is the strict denominator" 1
check_text "the un-requested case is still announced" "Base coverage: NOT APPLIED"
check_text "the subtraction count is reported even when nothing is subtracted" \
  "Pre-existing uncovered lines excluded from the denominator: 0"

# Two sources of base coverage is an operator error, not a precedence question:
# silently preferring one would make which lcov was consulted unknowable.
run_gate_base --base-lcov "${work}/base.lcov" --fetch-base-coverage
check_rc "asking for two base-coverage sources is refused outright" 1
check_text "the conflicting flags are named" \
  "--base-lcov cannot be combined with --fetch-base-coverage"

echo "▶ the artifact lookup is exercised, not just the local-file shortcut"
# `--fetch-base-coverage` is the mode CI runs. Testing only `--base-lcov` would
# leave the whole resolution path — the one that can silently stop working —
# unexercised, which is the "a check that cannot fail is not a check" trap.
fake_gh="${work}/fake-gh"
mkdir -p "${fake_gh}"
cat >"${fake_gh}/gh" <<EOF
#!/usr/bin/env bash
url="\$2"
case "\${url}" in
  *"/actions/workflows/reborn-tests.yml/runs"*) cat "${work}/gh-runs.json" ;;
  *"/actions/runs/"*"/artifacts"*) cat "${work}/gh-artifacts.json" ;;
  *"/actions/artifacts/"*"/zip") cat "${work}/gh-artifact.zip" ;;
  *) echo "unexpected gh call: \${url}" >&2; exit 9 ;;
esac
EOF
chmod +x "${fake_gh}/gh"
cat >"${work}/base.lcov" <<EOF
SF:${case_root}/${pre_path}
DA:2,0
DA:5,1
LF:2
LH:1
end_of_record
EOF
python3 - "${work}" <<'PY'
import pathlib, sys, zipfile
work = pathlib.Path(sys.argv[1])
with zipfile.ZipFile(work / "gh-artifact.zip", "w") as archive:
    archive.write(work / "base.lcov", "reborn-integration-merged.lcov")
PY
printf '%s\n' '{"workflow_runs": [{"id": 4242, "event": "merge_group", "status": "completed"}]}' \
  >"${work}/gh-runs.json"
printf '%s\n' '{"artifacts": [{"id": 77, "name": "reborn-integration-coverage-merged", "expired": false}]}' \
  >"${work}/gh-artifacts.json"
run_gate_fetch() {
  capture env PATH="${fake_gh}:${PATH}" GITHUB_REPOSITORY=nearai/ironclaw \
    python3 "${gate}" \
    --lcov "${work}/coverage.lcov" \
    --manifest "${work}/policy.toml" \
    --base "${preimage_base}" \
    --head "${preimage_head}" \
    --repo-root "${case_root}" \
    --json "${work}/report.json" \
    --fetch-base-coverage
}
# Drive the real diff/rename path against the fetched artifact, so the fetch is
# proved end to end rather than only up to the download.
cat >"${work}/coverage.lcov" <<EOF
SF:${case_root}/${moved_to}
DA:2,1
DA:5,0
BRDA:2,0,0,1
BRDA:2,0,1,1
LF:2
LH:1
BRF:2
BRH:2
end_of_record
EOF
cat >"${work}/base.lcov" <<EOF
SF:${case_root}/${moved_from}
DA:2,1
DA:5,0
LF:2
LH:1
end_of_record
EOF
python3 - "${work}" <<'PY'
import pathlib, sys, zipfile
work = pathlib.Path(sys.argv[1])
with zipfile.ZipFile(work / "gh-artifact.zip", "w") as archive:
    archive.write(work / "base.lcov", "reborn-integration-merged.lcov")
PY
run_gate_fetch
check_rc "a downloaded base artifact excludes the pre-existing line" 0
check_text "the fetched run is named for audit" "run 4242"
check_text "the fetched artifact drives the same subtraction" \
  "Pre-existing uncovered lines excluded from the denominator: 1"

printf '%s\n' '{"workflow_runs": []}' >"${work}/gh-runs.json"
run_gate_fetch
check_rc "no run for the base commit counts every changed line" 1
check_text "the empty run list is explained" "no completed reborn-tests.yml run exists"

printf '%s\n' '{"workflow_runs": [{"id": 4242, "event": "push", "status": "completed"}]}' \
  >"${work}/gh-runs.json"
printf '%s\n' '{"artifacts": [{"id": 77, "name": "reborn-integration-coverage-merged", "expired": true}]}' \
  >"${work}/gh-artifacts.json"
run_gate_fetch
check_rc "an expired artifact counts every changed line" 1
check_text "artifact expiry is explained" "no unexpired"

printf '%s\n' '{"artifacts": [{"id": 77, "name": "reborn-integration-coverage-merged", "expired": false}]}' \
  >"${work}/gh-artifacts.json"
printf '%s' 'not a zip' >"${work}/gh-artifact.zip"
run_gate_fetch
check_rc "an unreadable artifact counts every changed line" 1
check_text "a corrupt artifact is explained" "unreadable"

cat >"${fake_gh}/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh: HTTP 403" >&2
exit 1
EOF
chmod +x "${fake_gh}/gh"
run_gate_fetch
check_rc "an API failure counts every changed line" 1
check_text "the API failure is surfaced verbatim" "HTTP 403"
check_text "an API failure never silently subtracts" \
  "Pre-existing uncovered lines excluded from the denominator: 0"

rm -f "${fake_gh}/gh"
run_gate_fetch
check_rc "a missing gh binary counts every changed line" 1
check_text "the missing binary is explained" "Base coverage: NOT APPLIED"

echo
if [ "${failures}" -ne 0 ]; then
  echo "${failures} changed-coverage self-test(s) failed" >&2
  exit 1
fi
echo "all ${passes} changed-coverage self-tests passed"
