#!/usr/bin/env bash
set -euo pipefail

# Self-test for scripts/ci/check-wasm-artifact-freshness.py.
#
# A freshness gate that cannot fail is worse than no gate, so every case here
# is about the gate REFUSING: an edited guest source, a missing record, a
# stranded record, a package that lost its artifact, and a tree the gate cannot
# find at all. The happy path is checked last, against the real repository.

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
gate="${repo_root}/scripts/ci/check-wasm-artifact-freshness.py"
crate_tree="${repo_root}/scripts/ci/lib/crate_tree.py"

PASS=0
FAIL=0
tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

assert_rc() {  # label expected actual
  if [ "$2" = "$3" ]; then
    printf 'PASS %s\n' "$1"
    PASS=$((PASS + 1))
  else
    printf 'FAIL %s — expected rc %s, got %s\n' "$1" "$2" "$3" >&2
    FAIL=$((FAIL + 1))
  fi
}

assert_contains() {  # label haystack needle
  case "$2" in
    *"$3"*)
      printf 'PASS %s\n' "$1"
      PASS=$((PASS + 1))
      ;;
    *)
      printf 'FAIL %s — output missing: %s\n' "$1" "$3" >&2
      printf '  got: %s\n' "$2" >&2
      FAIL=$((FAIL + 1))
      ;;
  esac
}

# A fixture repo whose crate inventory clears crate_tree's floor of 20, with a
# support crate and one WASM-shipping package beside it.
make_repo() {  # repo_dir
  local repo="$1" i
  rm -rf "${repo}"
  mkdir -p "${repo}/scripts/ci/lib"
  cp "${gate}" "${repo}/scripts/ci/check-wasm-artifact-freshness.py"
  cp "${crate_tree}" "${repo}/scripts/ci/lib/crate_tree.py"
  for i in $(seq 1 25); do
    mkdir -p "${repo}/crates/ironclaw_filler_${i}"
    printf '[package]\nname = "ironclaw_filler_%s"\n' "${i}" \
      > "${repo}/crates/ironclaw_filler_${i}/Cargo.toml"
  done
  mkdir -p "${repo}/crates/extensions/ironclaw_extension_support"
  printf '[package]\nname = "ironclaw_extension_support"\n' \
    > "${repo}/crates/extensions/ironclaw_extension_support/Cargo.toml"
  mkdir -p "${repo}/crates/extensions/packages/demo/wasm-src/src" \
           "${repo}/crates/extensions/packages/demo/wasm"
  # The guest declares its own workspace, exactly like the real ones, so
  # crate_tree keeps it out of the crate inventory.
  printf '[package]\nname = "demo-tool"\n\n[workspace]\n' \
    > "${repo}/crates/extensions/packages/demo/wasm-src/Cargo.toml"
  printf 'pub fn demo() {}\n' \
    > "${repo}/crates/extensions/packages/demo/wasm-src/src/lib.rs"
  printf 'fake component bytes\n' \
    > "${repo}/crates/extensions/packages/demo/wasm/demo_tool.wasm"
}

run_gate() {  # repo_dir [args...]
  local repo="$1"
  shift
  CAP_RC=0
  CAP_OUT="$(IRONCLAW_REPO_ROOT="${repo}" python3 "${repo}/scripts/ci/check-wasm-artifact-freshness.py" "$@" 2>&1)" || CAP_RC=$?
}

# --------------------------------------------------------------------------
# F1: an unrecorded package refuses (the bootstrap direction).
# --------------------------------------------------------------------------
make_repo "${tmp}/fresh"
run_gate "${tmp}/fresh"
assert_rc       "F1 unrecorded package refuses" 1 "${CAP_RC}"
assert_contains "F1 names the package"          "${CAP_OUT}" "demo: no recorded digest"

# --------------------------------------------------------------------------
# F2: recording then verifying passes.
# --------------------------------------------------------------------------
run_gate "${tmp}/fresh" --update
assert_rc       "F2 --update exits 0"           0 "${CAP_RC}"
run_gate "${tmp}/fresh"
assert_rc       "F2 verify after record"        0 "${CAP_RC}"
assert_contains "F2 reports what it checked"    "${CAP_OUT}" "1 package(s) checked"

# --------------------------------------------------------------------------
# F3: THE POINT OF THE GATE — editing wasm-src without rebuilding fails.
# --------------------------------------------------------------------------
printf 'pub fn demo() { let _ = 1; }\n' \
  > "${tmp}/fresh/crates/extensions/packages/demo/wasm-src/src/lib.rs"
run_gate "${tmp}/fresh"
assert_rc       "F3 edited guest source fails"  1 "${CAP_RC}"
assert_contains "F3 says why"                   "${CAP_OUT}" "wasm-src changed but wasm/ was not rebuilt"

# --------------------------------------------------------------------------
# F4: adding a file to the guest is a change too (paths feed the digest).
# --------------------------------------------------------------------------
make_repo "${tmp}/added"
run_gate "${tmp}/added" --update
printf 'pub fn extra() {}\n' \
  > "${tmp}/added/crates/extensions/packages/demo/wasm-src/src/extra.rs"
run_gate "${tmp}/added"
assert_rc       "F4 new guest file fails"       1 "${CAP_RC}"

# --------------------------------------------------------------------------
# F5: build residue is not source — a target/ dir must not trip the gate.
# --------------------------------------------------------------------------
make_repo "${tmp}/residue"
run_gate "${tmp}/residue" --update
mkdir -p "${tmp}/residue/crates/extensions/packages/demo/wasm-src/target/release"
printf 'junk\n' > "${tmp}/residue/crates/extensions/packages/demo/wasm-src/target/release/x"
printf 'lock\n' > "${tmp}/residue/crates/extensions/packages/demo/wasm-src/Cargo.lock"
run_gate "${tmp}/residue"
assert_rc       "F5 target/ and Cargo.lock ignored" 0 "${CAP_RC}"

# --------------------------------------------------------------------------
# F6: a stranded record (package gone) refuses rather than passing.
# --------------------------------------------------------------------------
make_repo "${tmp}/stranded"
run_gate "${tmp}/stranded" --update
mkdir -p "${tmp}/stranded/crates/extensions/packages/other/wasm-src/src" \
         "${tmp}/stranded/crates/extensions/packages/other/wasm"
printf '[package]\nname = "other-tool"\n\n[workspace]\n' \
  > "${tmp}/stranded/crates/extensions/packages/other/wasm-src/Cargo.toml"
printf 'pub fn other() {}\n' \
  > "${tmp}/stranded/crates/extensions/packages/other/wasm-src/src/lib.rs"
printf 'bytes\n' > "${tmp}/stranded/crates/extensions/packages/other/wasm/other_tool.wasm"
run_gate "${tmp}/stranded" --update
rm -rf "${tmp}/stranded/crates/extensions/packages/other"
run_gate "${tmp}/stranded"
assert_rc       "F6 stranded record refuses"    1 "${CAP_RC}"
assert_contains "F6 names the stale entry"      "${CAP_OUT}" "other: recorded, but no such package"

# --------------------------------------------------------------------------
# F7: a guest whose artifact was deleted refuses.
# --------------------------------------------------------------------------
make_repo "${tmp}/noartifact"
rm -rf "${tmp}/noartifact/crates/extensions/packages/demo/wasm"
run_gate "${tmp}/noartifact"
assert_rc       "F7 missing artifact refuses"   1 "${CAP_RC}"
assert_contains "F7 says which package"         "${CAP_OUT}" "demo ships \`wasm-src/\` but no"

# --------------------------------------------------------------------------
# F8: the anchor crate is gone — refuse, do not report everything fresh.
# --------------------------------------------------------------------------
make_repo "${tmp}/anchorless"
rm -rf "${tmp}/anchorless/crates/extensions/ironclaw_extension_support"
run_gate "${tmp}/anchorless"
assert_rc       "F8 missing anchor refuses"     1 "${CAP_RC}"
assert_contains "F8 names the anchor"           "${CAP_OUT}" "ironclaw_extension_support"

# --------------------------------------------------------------------------
# F9: no WASM-shipping package at all — refuse rather than pass vacuously.
# --------------------------------------------------------------------------
make_repo "${tmp}/empty"
rm -rf "${tmp}/empty/crates/extensions/packages/demo"
mkdir -p "${tmp}/empty/crates/extensions/packages"
run_gate "${tmp}/empty"
assert_rc       "F9 no guests refuses"          1 "${CAP_RC}"
assert_contains "F9 says discovery is broken"   "${CAP_OUT}" "ships a \`wasm-src/\` guest"

# --------------------------------------------------------------------------
# F10: the real repository is recorded and fresh.
# --------------------------------------------------------------------------
CAP_RC=0
CAP_OUT="$(python3 "${gate}" 2>&1)" || CAP_RC=$?
assert_rc       "F10 real repo is fresh"        0 "${CAP_RC}"
assert_contains "F10 checks all six guests"     "${CAP_OUT}" "6 package(s) checked"

echo ""
echo "wasm artifact freshness tests: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
