#!/usr/bin/env bash
# Hermetic caller-path tests for the named critical mutation gate.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="${repo_root}/scripts/ci/critical_mutation_gate.py"
work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-critical-mut.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

# The gate resolves every manifest `package` against the crate inventory
# (scripts/ci/lib/crate_tree.py), which walks `crates/**/Cargo.toml` and refuses
# an inventory below MIN_CRATE_DIRECTORIES. Fixture roots therefore have to be
# real-enough crate trees, not two bare directories — weakening the floor for
# the benefit of the self-test would disarm the fail-closed property the self-
# test exists to prove.
seed_crate_tree() {
  local root="$1" prefix="$2" pad
  for pad in $(seq 1 24); do
    mkdir -p "${root}/${prefix}ironclaw_pad${pad}/src"
    printf '[package]\nname = "ironclaw_pad%s"\n' "${pad}" \
      >"${root}/${prefix}ironclaw_pad${pad}/Cargo.toml"
    printf '%s\n' 'pub fn pad() {}' >"${root}/${prefix}ironclaw_pad${pad}/src/lib.rs"
  done
}

# Writes a demo crate (source + guarding test + manifest) under "$1" at "$2".
seed_demo_crate() {
  local root="$1" prefix="$2" name="$3"
  mkdir -p "${root}/${prefix}${name}/src" "${root}/${prefix}${name}/tests"
  printf '[package]\nname = "%s"\n' "${name}" >"${root}/${prefix}${name}/Cargo.toml"
  printf '%s\n' 'pub fn authorize() -> bool { true }' >"${root}/${prefix}${name}/src/lib.rs"
  printf '%s\n' '#[test]' 'fn authorize_contract() {}' \
    >"${root}/${prefix}${name}/tests/authorize_contract.rs"
}

case_root="${work}/repo"
source_path="crates/ironclaw_demo/src/lib.rs"
watch_path="crates/ironclaw_demo/tests/authorize_contract.rs"
mkdir -p "${work}/bin"
seed_crate_tree "${case_root}" "crates/"
seed_demo_crate "${case_root}" "crates/" ironclaw_demo
# A second real crate, so "this path belongs to another crate" is a fixture
# rather than a hypothetical.
seed_demo_crate "${case_root}" "crates/" ironclaw_sibling
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
    echo '[]'
    exit 0
  fi
  if [ "${STUB_MODE:-caught}" = "malformed-list" ]; then
    echo '{'
    exit 0
  fi
  if [ "${STUB_MODE:-caught}" = "malformed-entry" ]; then
    echo '[{"name":"crates/ironclaw_demo/src/lib.rs:1:1: replace Demo::authorize with ()","function":{}}]'
    exit 0
  fi
  if [ "${STUB_MODE:-caught}" = "nearby-list" ]; then
    echo '[{"name":"crates/ironclaw_demo/src/lib.rs:1:1: replace authorize_helper -> bool with false","function":{"function_name":"Demo::authorize_helper"}}]'
    exit 0
  fi
  echo '[{"name":"crates/ironclaw_demo/src/lib.rs:1:1: replace * with +","function":null},{"name":"crates/ironclaw_demo/src/lib.rs:1:1: replace Demo::authorize with ()","function":{"function_name":"Demo::authorize"}},{"name":"crates/ironclaw_demo/src/lib.rs:2:1: replace Demo::authorize_helper -> bool with false","function":{"function_name":"Demo::authorize_helper"}}]'
  exit 0
fi
if [[ " $* " != *" --cargo-test-arg --lib --cargo-test-arg authorize_contract "* ]]; then
  echo "scoped cargo test args were not forwarded" >&2
  exit 8
fi
out=""
pattern=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2 ;;
    --re) pattern="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [[ "${pattern}" == *"authorize_helper"* ]]; then
  echo "similarly named sibling was included in the critical allowlist" >&2
  exit 7
fi
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
mutant='crates/ironclaw_demo/src/lib.rs:1:1: replace Demo::authorize with ()'
case "${STUB_MODE:-caught}" in
  caught) echo "${mutant}" >"${out}/mutants.out/caught.txt"; exit 0 ;;
  missed) echo "${mutant}" >"${out}/mutants.out/missed.txt"; exit 2 ;;
  timeout) echo "${mutant}" >"${out}/mutants.out/timeout.txt"; exit 3 ;;
  unexpected)
    echo 'crates/ironclaw_demo/src/lib.rs:2:1: replace Demo::authorize_helper -> bool with false' \
      >"${out}/mutants.out/caught.txt"
    exit 0
    ;;
  zero) exit 0 ;;
esac
STUB
chmod +x "${work}/bin/cargo"

passes=0
failures=0
capture_at() {
  local root="$1" mode="$2" manifest="$3" changed="$4"
  set +e
  CAP_OUT="$(PATH="${work}/bin:${PATH}" STUB_MODE="${mode}" python3 "${gate}" \
    --manifest "${manifest}" \
    --repo-root "${root}" \
    --changed-files "${changed}" 2>&1)"
  CAP_RC=$?
  set -e
}
capture() {
  capture_at "${case_root}" "${1}" "${work}/manifest.toml" "${work}/changed.txt"
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
check_text "survivor is printed verbatim" "replace Demo::authorize with ()"
capture timeout
check_rc "a timed-out named mutant blocks" 1
check_text "timeout is not treated as caught" "timed out"

echo "▶ empty discovery and wrong result-directory sabotage"
capture malformed-list
check_rc "malformed cargo-mutants discovery JSON fails" 1
check_text "malformed discovery identifies the JSON contract" "malformed JSON"
capture malformed-entry
check_rc "malformed cargo-mutants function metadata fails" 1
check_text "malformed function metadata identifies the missing field" "function.function_name"
capture no-list
check_rc "a named function with zero discovered mutants fails" 1
check_text "zero discovery names the stale function" "produced zero mutants"
capture nearby-list
check_rc "a similarly named sibling does not satisfy discovery" 1
check_text "substring discovery failure names the exact function" \
  "named critical function produced zero mutants: ${source_path}::authorize"
capture unexpected
check_rc "a result outside the named-mutant allowlist fails" 1
check_text "unexpected result reports an allowlist mismatch" "exact named-mutant allowlist"
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

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic escaping watch path."
watch_paths = ["crates/ironclaw_demo/../outside.rs"]
TOML
capture caught
check_rc "a watch path containing dot-dot fails" 1
check_text "escaping watch path is actionable" "watch path must stay inside"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic out-of-package watch path."
watch_paths = ["crates/ironclaw_other/tests/authorize.rs"]
TOML
capture caught
check_rc "an out-of-package watch path fails" 1
check_text "out-of-package watch path is actionable" "watch path must stay inside"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "First duplicate domain."

[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize_helper"
rationale = "Second duplicate domain."
TOML
capture caught
check_rc "duplicate invariant domains fail" 1
check_text "duplicate domain is actionable" "duplicate critical invariant domain"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "First duplicate function."

[[invariant]]
domain = "authorization-secondary"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Second duplicate function."
TOML
capture caught
check_rc "duplicate path and function entries fail" 1
check_text "duplicate function is actionable" "duplicate critical function"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "Ironclaw Demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic invalid package."
TOML
capture caught
check_rc "invalid package names fail" 1
check_text "invalid package is actionable" "invalid package in invariant"

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

# --- Tree-shape independence (#6963 / CHECKLIST WS10) ------------------------
# The gate used to validate `path` with `^crates/ironclaw_[^/]+/src/.+\.rs$` and
# to confine `watch_paths` to a literal `crates/<package>/`. Both stop matching
# the day crates move into family directories, and every entry then raises
# GateError — this gate blocks CI rather than going quietly green, but it blocks
# all the same. Discovery is now keyed on the crate's name and resolved to
# wherever its Cargo.toml actually is.

echo "▶ nested crate directories validate and select"
nested_root="${work}/nested"
seed_crate_tree "${nested_root}" "crates/substrates/"
seed_demo_crate "${nested_root}" "crates/substrates/" ironclaw_demo
nested_source="crates/substrates/ironclaw_demo/src/lib.rs"
nested_watch="crates/substrates/ironclaw_demo/tests/authorize_contract.rs"
cat >"${work}/nested-manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${nested_source}"
function = "authorize"
rationale = "Synthetic critical invariant under a family directory."
test_args = ["--lib", "authorize_contract"]
watch_paths = ["${nested_watch}"]
TOML
printf '%s\n' "${nested_source}" >"${work}/nested-changed.txt"
capture_at "${nested_root}" caught "${work}/nested-manifest.toml" "${work}/nested-changed.txt"
check_rc "a nested crate's invariant runs instead of erroring" 0
check_text "nested run reaches the same pass summary" "0 survived; 0 timed out"

printf '%s\n' "${nested_watch}" >"${work}/nested-changed.txt"
capture_at "${nested_root}" missed "${work}/nested-manifest.toml" "${work}/nested-changed.txt"
check_rc "a nested guarding-test change still selects its invariant" 1
check_text "nested selection reaches the survivor verdict" "survived"

printf '%s\n' 'docs/readme.md' >"${work}/nested-changed.txt"
capture_at "${nested_root}" missed "${work}/nested-manifest.toml" "${work}/nested-changed.txt"
check_rc "nesting does not make unrelated files select work" 0
check_text "nested unrelated selection is explicit" "no named invariant source or watch path changed"

echo "▶ discovery and package-resolution sabotage"
cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_ghost"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic renamed-away package."
TOML
printf '%s\n' "${source_path}" >"${work}/changed.txt"
capture caught
check_rc "a package absent from the inventory fails" 1
check_text "absent package names the entry and the count" \
  "names package 'ironclaw_ghost', which resolves to 0 crate director(ies)"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "crates/ironclaw_sibling/src/lib.rs"
function = "authorize"
rationale = "Synthetic package and path disagreement."
TOML
capture caught
check_rc "a path owned by another crate fails" 1
check_text "cross-crate path names both crates" \
  "belongs to crate crates/ironclaw_sibling, not to its declared package ironclaw_demo"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${watch_path}"
function = "authorize"
rationale = "Synthetic non-production path."
TOML
capture caught
check_rc "a path outside the crate's src/ fails" 1
check_text "non-production path is actionable" "invalid production path in invariant"

cat >"${work}/manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "docs/notes.rs"
function = "authorize"
rationale = "Synthetic path outside every crate."
TOML
capture caught
check_rc "a path attributable to no crate fails" 1
check_text "unattributable path is actionable" "invalid production path in invariant"

# A half-finished family move leaves the same crate name at two depths. The gate
# must refuse the ambiguity rather than pick one.
duplicate_root="${work}/duplicate"
seed_crate_tree "${duplicate_root}" "crates/"
seed_demo_crate "${duplicate_root}" "crates/" ironclaw_demo
seed_demo_crate "${duplicate_root}" "crates/substrates/" ironclaw_demo
cat >"${work}/duplicate-manifest.toml" <<TOML
[[invariant]]
domain = "authorization"
package = "ironclaw_demo"
path = "${source_path}"
function = "authorize"
rationale = "Synthetic half-finished move."
TOML
capture_at "${duplicate_root}" caught "${work}/duplicate-manifest.toml" "${work}/changed.txt"
check_rc "a crate name at two depths fails" 1
check_text "ambiguous package names the count" "resolves to 2 crate director(ies)"

# The floor in crate_tree.py is the last line of defence: a checkout whose crate
# tree is missing or truncated must never read as "no invariants to check".
empty_root="${work}/no-crates"
mkdir -p "${empty_root}/crates/ironclaw_demo/src"
printf '[package]\nname = "ironclaw_demo"\n' >"${empty_root}/crates/ironclaw_demo/Cargo.toml"
printf '%s\n' 'pub fn authorize() -> bool { true }' >"${empty_root}/${source_path}"
capture_at "${empty_root}" caught "${work}/duplicate-manifest.toml" "${work}/changed.txt"
check_rc "a truncated crate tree fails closed" 1
check_text "truncated tree explains the discovery floor" "crate discovery failed"

echo "▶ the shipped manifest validates against the shipped tree"
# Not hermetic on purpose: this is the pin that catches a [[invariant]] going
# stale against the real repository — a crate renamed out from under an entry,
# or a path moved. Selection over an empty changed-file set exercises the whole
# of load_manifest() without running cargo-mutants.
: >"${work}/no-changes.txt"
set +e
CAP_OUT="$(python3 "${gate}" \
  --manifest "${repo_root}/tests/integration/critical-mutation-functions.toml" \
  --repo-root "${repo_root}" \
  --changed-files "${work}/no-changes.txt" --selection-only 2>&1)"
CAP_RC=$?
set -e
check_rc "the checked-in manifest resolves against the checked-in crate tree" 0
check_text "no-op selection is reported" "false"

echo
if [ "${failures}" -ne 0 ]; then
  echo "${failures} critical-mutation self-test(s) failed" >&2
  exit 1
fi
echo "all ${passes} critical-mutation self-tests passed"
