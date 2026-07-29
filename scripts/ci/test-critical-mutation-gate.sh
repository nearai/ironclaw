#!/usr/bin/env bash
# Hermetic caller-path tests for the named critical mutation gate.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="${repo_root}/scripts/ci/critical_mutation_gate.py"
work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-critical-mut.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

case_root="${work}/repo"
source_path="crates/ironclaw_demo/src/lib.rs"
watch_path="crates/ironclaw_demo/tests/authorize_contract.rs"
mkdir -p "${case_root}/crates/ironclaw_demo/src" "${work}/bin"
mkdir -p "${case_root}/crates/ironclaw_demo/tests"
printf '%s\n' 'pub fn authorize() -> bool { true }' >"${case_root}/${source_path}"
printf '%s\n' '#[test]' 'fn authorize_contract() {}' >"${case_root}/${watch_path}"
printf '%s\n' "${source_path}" >"${work}/changed.txt"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic critical invariant."
test_args = ["--lib", "authorize_contract"]
watch_paths = ["${watch_path}"]
TOML

cat >"${work}/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" != "mutants" ]; then
  exit 9
fi
shift
if printf '%s\n' "$@" | grep -qx -- '--list'; then
  if [ "${STUB_MODE:-caught}" = "no-list" ]; then
    exit 0
  fi
  if [ "${STUB_MODE:-caught}" = "nearby-list" ]; then
    echo 'crates/ironclaw_demo/src/lib.rs:1:1: replace authorize_helper -> bool with false'
    exit 0
  fi
  echo 'crates/ironclaw_demo/src/lib.rs:1:1: replace authorize -> bool with false'
  exit 0
fi
if [[ " $* " != *" --cargo-test-arg --lib --cargo-test-arg authorize_contract "* ]]; then
  echo "scoped cargo test args were not forwarded" >&2
  exit 8
fi
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [ "${STUB_MODE:-caught}" = "wrong-dir" ]; then
  mkdir -p "${out}"
  : >"${out}/caught.txt"
  exit 0
fi
mkdir -p "${out}/mutants.out"
: >"${out}/mutants.out/caught.txt"
: >"${out}/mutants.out/missed.txt"
: >"${out}/mutants.out/unviable.txt"
: >"${out}/mutants.out/timeout.txt"
mutant='crates/ironclaw_demo/src/lib.rs:1:1: replace authorize -> bool with false'
case "${STUB_MODE:-caught}" in
  caught) echo "${mutant}" >"${out}/mutants.out/caught.txt"; exit 0 ;;
  missed) echo "${mutant}" >"${out}/mutants.out/missed.txt"; exit 2 ;;
  timeout) echo "${mutant}" >"${out}/mutants.out/timeout.txt"; exit 2 ;;
  zero) exit 0 ;;
esac
STUB
chmod +x "${work}/bin/cargo"

passes=0
failures=0
capture() {
  set +e
  CAP_OUT="$(PATH="${work}/bin:${PATH}" STUB_MODE="${1}" python3 "${gate}" \
    --manifest "${work}/manifest.toml" \
    --repo-root "${case_root}" \
    --changed-files "${work}/changed.txt" 2>&1)"
  CAP_RC=$?
  set -e
}
check_rc() {
  local label="$1" expected="$2"
  if [ "${CAP_RC}" -eq "${expected}" ]; then
    echo "  ok   ${label}"
    passes=$((passes + 1))
  else
    echo "  FAIL ${label}: expected ${expected}, got ${CAP_RC}" >&2
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

echo "▶ named critical gate happy path"
capture caught
check_rc "all named mutants caught passes" 0
check_text "pass summary refuses a score" "0 survived; 0 timed out"

echo "▶ survivor and timeout sabotage"
capture missed
check_rc "a surviving named mutant blocks" 1
check_text "survivor is printed verbatim" "replace authorize -> bool with false"
capture timeout
check_rc "a timed-out named mutant blocks" 1
check_text "timeout is not treated as caught" "timed out"

echo "▶ empty discovery and wrong result-directory sabotage"
capture no-list
check_rc "a named function with zero discovered mutants fails" 1
check_text "zero discovery names the stale function" "produced zero mutants"
capture nearby-list
check_rc "a similarly named sibling does not satisfy discovery" 1
check_text "substring discovery failure names the exact function" "authorize"
capture zero
check_rc "an empty result is not a pass" 1
check_text "empty result explains zero mutants" "produced zero mutants"
capture wrong-dir
check_rc "results in the wrong directory fail" 1
check_text "wrong result directory names the missing report" "cargo-mutants result missing"

echo "▶ malformed manifest and stale path fixtures"
cat >"${work}/manifest.toml" <<'TOML'
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "crates/ironclaw_demo/src/lib.rs"
function = "authorize"
TOML
capture caught
check_rc "malformed invariant entry fails" 1
check_text "malformed manifest lists the exact schema" "fields must contain"

cat >"${work}/manifest.toml" <<'TOML'
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "crates/ironclaw_demo/src/moved.rs"
function = "authorize"
rationale = "Synthetic stale path."
TOML
capture caught
check_rc "stale production path fails" 1
check_text "stale path is actionable" "stale critical invariant path"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic stale watch path."
watch_paths = ["crates/ironclaw_demo/tests/moved.rs"]
TOML
capture caught
check_rc "stale watch path fails" 1
check_text "stale watch path is actionable" "stale critical invariant watch path"

echo "▶ guarding-test changes select mutation work"
cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic critical invariant."
test_args = ["--lib", "authorize_contract"]
watch_paths = ["${watch_path}"]
TOML
printf '%s\n' "${watch_path}" >"${work}/changed.txt"
capture missed
check_rc "a guarding-test change reruns its named invariant" 1
check_text "guarding-test selection reaches the survivor verdict" "survived"

echo "▶ unrelated changes select no mutation work"
printf '%s\n' 'docs/readme.md' >"${work}/changed.txt"
capture missed
check_rc "an unrelated file does not run expensive mutation work" 0
check_text "unrelated selection is explicit" "no named invariant source or watch path changed"

echo
if [ "${failures}" -ne 0 ]; then
  echo "${failures} critical-mutation self-test(s) failed" >&2
  exit 1
fi
echo "all ${passes} critical-mutation self-tests passed"
