#!/usr/bin/env bash
#
# Regression tests for scripts/build-wasm-extensions.sh manifest DISCOVERY.
#
# Standalone: bash scripts/ci/test-build-wasm-extensions.sh
#
# Scope is deliberately discovery + the empty-set guards, not WASM builds:
# every fixture manifest declares a host-native runtime kind, which the builder
# reports as SKIP without invoking cargo-component. That keeps these cases
# hermetic (no toolchain, no network) while still driving the real script
# end to end.
#
# Why this file exists: the first-party manifest set used to be keyed to the
# literal crates/ironclaw_first_party_extensions/assets/*/manifest.toml glob.
# Under the target-architecture family move that glob matches nothing even
# though the manifests are still there (docs/reborn/target-architecture/
# CHECKLIST.md WS10, #6963). Discovery now resolves the owning crate by NAME
# through scripts/ci/lib/crate_tree.py, and these cases pin both that it finds
# a moved crate and that an empty set still refuses.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script_under_test="${repo_root}/scripts/build-wasm-extensions.sh"
crate_tree="${repo_root}/scripts/ci/lib/crate_tree.py"

PASS=0
FAIL=0
CAP_OUT=""
CAP_RC=0

capture() { CAP_RC=0; CAP_OUT="$("$@" 2>&1)" || CAP_RC=$?; }

assert_rc() {
    local name="$1" want="$2" got="$3"
    if [ "${got}" -eq "${want}" ]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — expected exit ${want}, got ${got}"; echo "----"; echo "${CAP_OUT}"; echo "----"; fi
}

assert_contains() {
    local name="$1" hay="$2" needle="$3"
    if [[ "${hay}" == *"${needle}"* ]]; then PASS=$((PASS+1));
    else FAIL=$((FAIL+1)); echo "FAIL: ${name} — output missing: ${needle}"; echo "----"; echo "${hay}"; echo "----"; fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# crate_tree.py's fail-closed floor is MIN_CRATE_DIRECTORIES=20, so a fixture
# that exercises discovery must be a real-enough workspace. Padding crates carry
# only a manifest.
CRATE_FLOOR_PADDING=24

# Build a fixture repo: the script under test + the discovery library + a crates
# tree whose first-party extension crate sits at $2 (repo-relative).
make_repo() {  # dir first_party_crate_dir
    local dir="$1" fp="$2" i name
    rm -rf "${dir}"
    mkdir -p "${dir}/scripts/ci/lib" "${dir}/registry/tools" "${dir}/registry/channels"
    cp "${script_under_test}" "${dir}/scripts/build-wasm-extensions.sh"
    cp "${crate_tree}" "${dir}/scripts/ci/lib/crate_tree.py"
    for i in $(seq 1 "${CRATE_FLOOR_PADDING}"); do
        name="$(printf 'ironclaw_pad%02d' "${i}")"
        mkdir -p "${dir}/crates/${name}"
        printf '[package]\nname = "%s"\n' "${name}" > "${dir}/crates/${name}/Cargo.toml"
    done
    if [ -n "${fp}" ]; then
        mkdir -p "${dir}/${fp}"
        printf '[package]\nname = "ironclaw_first_party_extensions"\n' > "${dir}/${fp}/Cargo.toml"
    fi
}

# A host-native manifest: enumerated by discovery, SKIPped by the builder, so no
# toolchain is needed to prove the manifest was found.
add_manifest() {  # assets_dir extension_id
    mkdir -p "$1/$2"
    printf 'schema_version = "reborn.extension_manifest.v3"\nid = "%s"\n\n[runtime]\nkind = "first_party"\nservice = "stub"\n' \
        "$2" > "$1/$2/manifest.toml"
}

# The script cd's to its own repo root, so each fixture gets its own copy and is
# invoked by path. Command substitution already runs in a subshell, so the cd
# cannot leak; CAP_* stay assigned in this shell.
run_set() {  # repo_dir flag
    CAP_RC=0
    CAP_OUT="$(cd "$1" && bash scripts/build-wasm-extensions.sh "$2" 2>&1)" || CAP_RC=$?
}

run_first_party() {  # repo_dir
    run_set "$1" --first-party
}

# ---------------------------------------------------------------------------
# W1: POSITIVE — flat tree. Discovery resolves the crate and enumerates both
#     manifests.
# ---------------------------------------------------------------------------
make_repo "${tmp}/flat" "crates/ironclaw_first_party_extensions"
add_manifest "${tmp}/flat/crates/ironclaw_first_party_extensions/assets" slack
add_manifest "${tmp}/flat/crates/ironclaw_first_party_extensions/assets" gmail
run_first_party "${tmp}/flat"
assert_rc       "W1 flat tree exits 0"          0 "${CAP_RC}"
assert_contains "W1 flat finds slack"           "${CAP_OUT}" "SKIP first-party/slack"
assert_contains "W1 flat finds gmail"           "${CAP_OUT}" "SKIP first-party/gmail"

# ---------------------------------------------------------------------------
# W2: POSITIVE — the same crate nested under a family directory. This is the
#     case the old glob missed while the manifests were still present.
# ---------------------------------------------------------------------------
make_repo "${tmp}/nested" "crates/substrates/ironclaw_first_party_extensions"
add_manifest "${tmp}/nested/crates/substrates/ironclaw_first_party_extensions/assets" slack
add_manifest "${tmp}/nested/crates/substrates/ironclaw_first_party_extensions/assets" gmail
run_first_party "${tmp}/nested"
assert_rc       "W2 nested tree exits 0"        0 "${CAP_RC}"
assert_contains "W2 nested finds slack"         "${CAP_OUT}" "SKIP first-party/slack"
assert_contains "W2 nested finds gmail"         "${CAP_OUT}" "SKIP first-party/gmail"

# ---------------------------------------------------------------------------
# W3: NEGATIVE — the extensions crate is absent (renamed). Discovery must name
#     the crate it could not resolve instead of building nothing.
# ---------------------------------------------------------------------------
make_repo "${tmp}/absent" ""
run_first_party "${tmp}/absent"
assert_rc       "W3 absent crate exits 1"       1 "${CAP_RC}"
assert_contains "W3 absent crate names it"      "${CAP_OUT}" "expected exactly one crate named 'ironclaw_first_party_extensions'"

# ---------------------------------------------------------------------------
# W4: NEGATIVE — crate resolves but carries no manifests. Pins the empty-set
#     guard that already existed (build_manifest_set's "no manifests found").
# ---------------------------------------------------------------------------
make_repo "${tmp}/empty" "crates/ironclaw_first_party_extensions"
mkdir -p "${tmp}/empty/crates/ironclaw_first_party_extensions/assets"
run_first_party "${tmp}/empty"
assert_rc       "W4 empty assets exits 1"       1 "${CAP_RC}"
assert_contains "W4 empty assets refuses"       "${CAP_OUT}" "no manifests found"

# ---------------------------------------------------------------------------
# W5: NEGATIVE — a below-floor crate inventory must refuse rather than silently
#     resolving nothing.
# ---------------------------------------------------------------------------
rm -rf "${tmp}/thin"
mkdir -p "${tmp}/thin/scripts/ci/lib" "${tmp}/thin/crates/ironclaw_first_party_extensions/assets"
cp "${script_under_test}" "${tmp}/thin/scripts/build-wasm-extensions.sh"
cp "${crate_tree}" "${tmp}/thin/scripts/ci/lib/crate_tree.py"
printf '[package]\nname = "ironclaw_first_party_extensions"\n' \
    > "${tmp}/thin/crates/ironclaw_first_party_extensions/Cargo.toml"
run_first_party "${tmp}/thin"
assert_rc       "W5 thin inventory exits 1"     1 "${CAP_RC}"
assert_contains "W5 thin inventory refuses"     "${CAP_OUT}" "crate discovery failed"

# ---------------------------------------------------------------------------
# W6: NEGATIVE — the registry tool set has the same empty-set guard. Pins that
#     the other two manifest sets also refuse rather than exiting 0.
# ---------------------------------------------------------------------------
make_repo "${tmp}/notools" "crates/ironclaw_first_party_extensions"
run_set "${tmp}/notools" --tools
assert_rc       "W6 empty registry/tools exits 1" 1 "${CAP_RC}"
assert_contains "W6 empty registry/tools refuses" "${CAP_OUT}" "no manifests found"

echo ""
echo "build-wasm-extensions discovery tests: ${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
