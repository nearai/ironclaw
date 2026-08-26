#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
classifier="${script_dir}/classify-test-scope.sh"

assert_scope() {
  local name="$1"
  local files="$2"
  local expected="$3"
  local actual

  actual="$(printf '%s\n' "$files" | "$classifier" | sort)"

  if [ "$actual" != "$expected" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected:\n%s\n' "$expected" >&2
    printf 'Actual:\n%s\n' "$actual" >&2
    exit 1
  fi

  printf 'PASS %s\n' "$name"
}

assert_scope_no_trailing_newline() {
  local name="$1"
  local files="$2"
  local expected="$3"
  local actual

  actual="$(printf '%s' "$files" | "$classifier" | sort)"

  if [ "$actual" != "$expected" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected:\n%s\n' "$expected" >&2
    printf 'Actual:\n%s\n' "$actual" >&2
    exit 1
  fi

  printf 'PASS %s\n' "$name"
}

assert_empty_scope() {
  local expected="$1"
  local actual

  actual="$(printf '' | "$classifier" | sort)"

  if [ "$actual" != "$expected" ]; then
    printf 'FAIL empty input\n' >&2
    printf 'Expected:\n%s\n' "$expected" >&2
    printf 'Actual:\n%s\n' "$actual" >&2
    exit 1
  fi

  printf 'PASS empty input\n'
}

assert_scope \
  "family-level guidance file (direct child of a family dir, owned by no crate)" \
  "crates/app/AGENTS.md" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_scope \
  "reborn binary crate" \
  "crates/ironclaw_cli/src/main.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn product storage crate" \
  "crates/ironclaw_product_storage/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn telegram extension crate" \
  "crates/extensions/packages/telegram/src/channel.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn telegram protocol engine (merged from the v2 adapter crate)" \
  "crates/extensions/packages/telegram/src/render.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn slack package crate" \
  "crates/extensions/packages/slack/src/channel.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn memory-native package crate" \
  "crates/extensions/packages/memory-native/src/service.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn mem0 package crate" \
  "crates/extensions/packages/mem0/src/service.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

# A data-only package owns no crate BY DESIGN — no Cargo.toml, just manifest,
# prompts, schemas and committed wasm. Its data is embedded by
# `ironclaw_extension_support`, so it lights the same lane that crate does.
# This case is not hypothetical: without it the classifier REFUSED on
# `packages/github/manifest.toml` and failed every job that consults it.
assert_scope \
  "data-only package manifest" \
  "crates/extensions/packages/github/manifest.toml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "data-only package prompt asset" \
  "crates/extensions/packages/gmail/prompts/gmail/send.md" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

# A guest component rooting its own workspace: attributable to no crate, but a
# refusal would be wrong — it is excluded by construction, not missing.
assert_scope \
  "wasm-src guest inside a data-only package" \
  "crates/extensions/packages/github/wasm-src/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

# A crate-bearing package resolves through its own Cargo.toml, so its manifest
# rides the crate's arm rather than the package-data arm.
assert_scope \
  "crate-bearing package manifest" \
  "crates/extensions/packages/slack/manifest.toml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn support crate" \
  "crates/ironclaw_outbound/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "named critical product invariant" \
  "crates/ironclaw_assistant/src/run_delivery/observer.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "named critical extension-host invariant" \
  "crates/ironclaw_extension_host/src/channel_outbound_targets.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

# The WS2.4 split: the extension-management product face lives in its own crate
# now, and its arm in `is_reborn_test_path` was added with the split. Without a
# case here a manager-only diff would classify `has_reborn_tests=false` and the
# `reborn-tests` roll-up would pass fast having skipped every Reborn lane — the
# exact failure #6947 records for the stale `crates/ironclaw_product_*/*` arm.
assert_scope \
  "extension-manager product face (WS2.4 split)" \
  "crates/ironclaw_extension_manager/src/lifecycle_product_service.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn root test runner script" \
  "scripts/ci/run-reborn-root-partition.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn group test runner script" \
  "scripts/ci/run-reborn-group-tests.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn root tests and support" \
  "tests/reborn_qa_smoke_scenarios_e2e.rs
tests/integration/support/harness/mod.rs
tests/e2e/scenarios/test_reborn_gateway_smoke.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn qa trace fixture" \
  "tests/fixtures/llm_traces/reborn_qa/routine_health_ping.json" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn e2e scenario" \
  "tests/e2e/scenarios/test_reborn_scope_isolation.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "legacy e2e scenario" \
  "tests/e2e/scenarios/test_live_flow.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_scope \
  "mixed legacy and reborn root tests" \
  "tests/e2e_live.rs
tests/reborn_trace_first_party_tool_coverage.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared manifest" \
  "Cargo.toml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared substrate crate" \
  "crates/ironclaw_host_runtime/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "process lifecycle authority crate" \
  "crates/ironclaw_processes/src/journal_store.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared classifier script" \
  "scripts/ci/classify-test-scope.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared package feature flags script" \
  "scripts/ci/package-feature-flags.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "Reborn crate bucket script" \
  "scripts/ci/reborn-crate-test-buckets.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn crate bucket regression suite" \
  "scripts/ci/test-reborn-crate-test-buckets.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn Responses E2E manifest checker" \
  "scripts/ci/check-reborn-responses-e2e-manifest.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn Responses E2E manifest" \
  "tests/e2e/reborn_responses_e2e_tests.txt" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn coverage manifest" \
  "tests/e2e/reborn_coverage_tests.txt" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn changed coverage, branch export, and critical mutation gates" \
  "scripts/ci/reborn_changed_coverage.py
scripts/ci/check-reborn-branch-coverage-flags.py
scripts/ci/test-check-reborn-branch-coverage-flags.sh
scripts/ci/critical_mutation_gate.py
scripts/ci/test-critical-mutation-gate.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "Reborn QA fixture checker and sabotage suite" \
  "scripts/ci/check-reborn-qa-fixtures.sh
scripts/ci/test-check-reborn-qa-fixtures.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "shared reborn tests workflow" \
  ".github/workflows/reborn-tests.yml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "legacy code style workflow" \
  ".github/workflows/code_style.yml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_scope \
  "docs only" \
  "README.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=false"

assert_empty_scope \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=false"

assert_scope \
  "nested markdown is not docs only" \
  "crates/ironclaw_turn_runner/CLAUDE.md" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn docs only" \
  "docs/internal/reborn/harness/e2e.md" \
  "docs_only=true
has_core_code=false
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "mixed tests and reborn" \
  "tests/e2e/scenarios/test_live_flow.py
crates/ironclaw_composition/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope_no_trailing_newline \
  "final path without trailing newline" \
  "crates/ironclaw_cli/src/main.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage lane-run script" \
  "scripts/ci/reborn-coverage-lane-run.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage merge-lcov script" \
  "scripts/ci/reborn-coverage-merge-lcov.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage summary script" \
  "scripts/ci/reborn-coverage-summary.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage regression suite" \
  "scripts/ci/test-reborn-coverage.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "reborn coverage regression suite, sourced sibling (R-section split)" \
  "scripts/ci/test-reborn-coverage-ratchet-cases.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "WS12 generated gates and sabotage tests are reborn-scoped" \
  "scripts/ci/reborn_changed_coverage.py
scripts/ci/test_reborn_changed_coverage.py
scripts/ci/ws12-suite-shards.toml
scripts/ci/ws12_suite_shards.py
scripts/ci/test_ws12_suite_shards.py
scripts/ci/ws12_workflow_contracts.py
scripts/ci/test_ws12_workflow_contracts.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "WS12 contract modules split into scripts/ci/lib stay reborn-scoped" \
  "scripts/ci/lib/rust_toolchain_contracts.py
scripts/ci/lib/workflow_text.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "test suite boundaries checker script" \
  "scripts/ci/check-test-suite-boundaries.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope \
  "test-classify-test-scope script is itself reborn-scoped" \
  "scripts/ci/test-classify-test-scope.sh" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope \
  "shared coverage lcov lib is reborn-scoped (gemini: PR #5718 comment)" \
  "scripts/ci/lib/reborn_coverage_lcov.py" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

# ---------------------------------------------------------------------------
# Tree-shape independence (CHECKLIST WS10)
#
# The arms above are keyed to crate identity, not to how deep a crate sits
# under crates/. These cases pin that: the same crate classifies the same way
# from a family directory, and a crate tree the classifier cannot read is a
# refusal rather than a silent "nothing in scope".
# ---------------------------------------------------------------------------

# Fixture repo root with the target-architecture layout: crates live one level
# down, inside a family directory.
nested_root="$(mktemp -d)"
trap 'rm -rf "${nested_root}"' EXIT
# `ironclaw_mcp` is the "in neither list" probe: it must classify identically
# nested and flat, which is the whole point — normalization must not invent a
# bucket for a crate that has none. If a future PR adds it to either arm (as
# #6889 did for `ironclaw_extension_host`, which used to fill this role), swap
# in any other crate that appears in neither `is_shared_test_path` nor
# `is_reborn_test_path`.
for crate in ironclaw_host_runtime ironclaw_webui ironclaw_mcp; do
  mkdir -p "${nested_root}/crates/substrates/${crate}/src"
  printf '[package]\nname = "%s"\n' "${crate}" \
    > "${nested_root}/crates/substrates/${crate}/Cargo.toml"
done
# 20 more so the discovery floor is met by a realistic tree, not one fixture.
for i in $(seq 1 20); do
  mkdir -p "${nested_root}/crates/domains/ironclaw_filler_${i}"
  printf '[package]\nname = "ironclaw_filler_%s"\n' "${i}" \
    > "${nested_root}/crates/domains/ironclaw_filler_${i}/Cargo.toml"
done

# ironclaw_extension_support and its sibling packages/ tree, nested one family
# level down too — the fixture package_asset_dir's own WS10 case needs: a
# data-only package (github, no Cargo.toml) whose real location has moved out
# from under the literal `crates/extensions/packages/*` arms in
# is_shared_test_path.
mkdir -p "${nested_root}/crates/substrates/ironclaw_extension_support"
printf '[package]\nname = "ironclaw_extension_support"\n' \
  > "${nested_root}/crates/substrates/ironclaw_extension_support/Cargo.toml"
mkdir -p "${nested_root}/crates/substrates/packages/github"
printf 'id = "github"\n' \
  > "${nested_root}/crates/substrates/packages/github/manifest.toml"

assert_scope_with_root() {
  local name="$1" root="$2" files="$3" expected="$4" actual
  actual="$(printf '%s\n' "$files" | IRONCLAW_REPO_ROOT="$root" "$classifier" | sort)"
  if [ "$actual" != "$expected" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected:\n%s\nActual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
  printf 'PASS %s\n' "$name"
}

assert_refusal_with_root() {
  local name="$1" root="$2" files="$3" needle="$4" err rc
  err="$(printf '%s\n' "$files" | IRONCLAW_REPO_ROOT="$root" "$classifier" 2>&1 >/dev/null)" && rc=0 || rc=$?
  if [ "$rc" -eq 0 ] || [ "${err#*"$needle"}" = "$err" ]; then
    printf 'FAIL %s\n' "$name" >&2
    printf 'Expected exit!=0 and stderr containing: %s\nActual rc=%s stderr:\n%s\n' \
      "$needle" "$rc" "$err" >&2
    exit 1
  fi
  printf 'PASS %s\n' "$name"
}

assert_scope_with_root \
  "nested shared crate still classifies as shared" \
  "${nested_root}" \
  "crates/substrates/ironclaw_host_runtime/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_scope_with_root \
  "nested reborn crate still classifies as reborn-only" \
  "${nested_root}" \
  "crates/substrates/ironclaw_webui/src/handlers.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=false
has_reborn_tests=true"

assert_scope_with_root \
  "nested unlisted crate still classifies as legacy-only (unchanged bucketing)" \
  "${nested_root}" \
  "crates/substrates/ironclaw_mcp/src/lib.rs" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=false"

assert_scope_with_root \
  "nested data-only package manifest still classifies as shared (package_asset_dir normalization)" \
  "${nested_root}" \
  "crates/substrates/packages/github/manifest.toml" \
  "docs_only=false
has_core_code=true
has_legacy_tests=true
has_reborn_tests=true"

assert_refusal_with_root \
  "a crates/ path attributable to no crate is refused, not bucketed" \
  "${nested_root}" \
  "crates/extensions/packages/slack/manifest.toml" \
  "cannot attribute"

assert_refusal_with_root \
  "an unreadable crate tree is refused, not classified as out-of-scope" \
  "${nested_root}/does-not-exist" \
  "crates/ironclaw_webui/src/lib.rs" \
  "no crates/ directory"

empty_tree_root="$(mktemp -d "${nested_root}/empty.XXXXXX")"
mkdir -p "${empty_tree_root}/crates"
assert_refusal_with_root \
  "a crate tree below the discovery floor is refused" \
  "${empty_tree_root}" \
  "crates/ironclaw_webui/src/lib.rs" \
  "crate director"

# The bash inventory in classify-test-scope.sh and the Python one in
# scripts/ci/lib/crate_tree.py are two implementations of one rule. Pin them
# equal so a fix to either cannot leave the other keyed to a stale tree shape.
#
# The classifier is *sourced* rather than re-implemented here: reading its own
# `crate_dirs` and `min_crate_directories` is the only way this file pins the
# shipping rule instead of a third copy of it, which would keep passing while
# the classifier's own `find` expression, prune list, or root drifted. stdin is
# /dev/null so the sourced `while read` loop terminates immediately.
repo_root_for_inventory="$(cd "${script_dir}/../.." && pwd)"
classifier_state="$(
  set -euo pipefail
  # shellcheck disable=SC1090
  source "${classifier}" </dev/null >/dev/null
  printf '%s\n---FLOOR---\n%s\n' "${crate_dirs}" "${min_crate_directories}"
)"
classifier_inventory="$(printf '%s' "${classifier_state}" | sed -n '1,/---FLOOR---/p' | sed '$d' | grep . | sort)"
classifier_floor="$(printf '%s' "${classifier_state}" | sed -n '/---FLOOR---/,$p' | tail -1)"

python_inventory="$(
  python3 "${script_dir}/lib/crate_tree.py" "${repo_root_for_inventory}" | sort
)"
python_floor="$(
  python3 -c "import sys; sys.path.insert(0, '${script_dir}/lib');
import crate_tree; print(crate_tree.MIN_CRATE_DIRECTORIES)"
)"

# The classifier keeps nested manifests in its list (resolution walks outward-in
# and stops at the outermost hit); crate_tree.py prunes them. Compare the
# outermost sets.
classifier_outermost="$(
  printf '%s\n' "${classifier_inventory}" | awk '
    { keep = 1
      for (i = 1; i <= n; i++) if (index($0, kept[i] "/") == 1) { keep = 0; break }
      if (keep) { kept[++n] = $0; print } }'
)"
if [ "${classifier_outermost}" != "${python_inventory}" ]; then
  printf 'FAIL bash and python crate inventories agree\n' >&2
  diff <(printf '%s\n' "${classifier_outermost}") <(printf '%s\n' "${python_inventory}") >&2 || true
  exit 1
fi
printf 'PASS bash and python crate inventories agree (%s crate directories)\n' \
  "$(printf '%s\n' "${python_inventory}" | grep -c .)"

if [ "${classifier_floor}" != "${python_floor}" ]; then
  printf 'FAIL bash and python discovery floors agree\n' >&2
  printf 'classifier min_crate_directories=%s crate_tree MIN_CRATE_DIRECTORIES=%s\n' \
    "${classifier_floor}" "${python_floor}" >&2
  exit 1
fi
printf 'PASS bash and python discovery floors agree (%s)\n' "${python_floor}"

# Every `crates/...` pattern in the classifier must match at least one real
# path. This is the check that was missing when the WS6 renames dropped the
# `ironclaw_reborn_` prefix: the arm `crates/ironclaw_reborn_*/*` stopped
# matching anything, and all seven of those crates silently reclassified as
# legacy. A pattern that matches nothing fails OPEN — the classifier keeps
# answering, just wrongly — so only the downstream self-test assertion caught
# it, and only for the one crate that had a fixture. Same shape as
# `sanctioned_paths_all_match_real_files` in reborn_retired_taxonomy.rs: an
# exemption may not outlive the code it exempts.
#
# KNOWN-DEAD, shrink-only. Both predate WS6 and both match nothing today, so
# neither is load-bearing. They are listed rather than repointed because
# repointing changes which tests a change to those crates selects, which is a
# behaviour change and not this PR's business:
#   * crates/ironclaw_extension_support/ -- crate lives at
#     crates/extensions/ironclaw_extension_support (already matched elsewhere
#     by the `*/ironclaw_extension_support` arm).
#   * crates/ironclaw_oauth/ -- no such crate anywhere in the tree.
known_dead_patterns="crates/ironclaw_extension_support/
crates/ironclaw_oauth/"

# The arms are keyed to the NORMALIZED spelling `crates/<crate>/<rest>`, which
# is a crate IDENTITY, not a path on disk — `normalize_crate_path` rewrites a
# real `crates/<family>/<crate>/<rest>` onto it. So "does this arm name
# something real?" is answered against the crate inventory, not by globbing the
# filesystem: after the family move (PROPOSAL §5) every live arm would fail a
# bare glob, and repointing the arms instead would break the normalizer's
# contract. The check itself is unchanged in strength — an arm naming a crate
# the inventory cannot resolve is still dead (CHECKLIST WS10).
dead_found=0
while IFS= read -r pattern; do
  [ -n "${pattern}" ] || continue
  case "${known_dead_patterns}" in
    *"${pattern}"*) continue ;;
  esac
  crate_name="${pattern#crates/}"
  crate_name="${crate_name%/}"
  if ! python3 "${script_dir}/lib/crate_tree.py" --directory "${crate_name}" \
      "${repo_root_for_inventory}" >/dev/null 2>&1; then
    printf 'FAIL classifier pattern matches no real path: %s\n' "${pattern}" >&2
    dead_found=1
  fi
done <<PATTERNS
$(grep -oE 'crates/ironclaw_[A-Za-z0-9_]*/' scripts/ci/classify-test-scope.sh | sort -u || true)
PATTERNS

if [ "${dead_found}" -ne 0 ]; then
  printf 'FAIL a classifier arm names a crate directory that does not exist\n' >&2
  exit 1
fi
printf 'PASS every classifier crates/ pattern matches a real path\n'
# ---------------------------------------------------------------------------
# crate_tree.py --directory and its scripts/ci/crate-dir.sh shell wrapper
# (CHECKLIST WS10, #6963 idiom)
#
# These two are otherwise untested: `--directory` is a new CLI flag on the
# module this file already pins above, and crate-dir.sh is a one-line wrapper
# around it. Pinned here rather than in a new file — this is the file that
# already invokes crate_tree.py as a subprocess and already owns the
# `nested_root` WS10 fixture, so reusing both keeps one home for "does the
# shared crate-resolution machinery survive a family move" instead of a third
# copy of the fixture.
# ---------------------------------------------------------------------------

first_python_crate="$(printf '%s\n' "${python_inventory}" | head -1)"
first_python_crate_name="${first_python_crate##*/}"

directory_flag_output="$(
  python3 "${script_dir}/lib/crate_tree.py" --directory "${first_python_crate_name}" "${repo_root_for_inventory}"
)"
if [ "${directory_flag_output}" != "${first_python_crate}" ]; then
  printf 'FAIL crate_tree.py --directory resolves the same directory as the plain listing\n' >&2
  printf 'expected %s got %s\n' "${first_python_crate}" "${directory_flag_output}" >&2
  exit 1
fi
printf 'PASS crate_tree.py --directory resolves the same directory as the plain listing (%s)\n' \
  "${first_python_crate_name}"

nested_directory_output="$(
  python3 "${script_dir}/lib/crate_tree.py" --directory ironclaw_webui "${nested_root}"
)"
if [ "${nested_directory_output}" != "crates/substrates/ironclaw_webui" ]; then
  printf 'FAIL crate_tree.py --directory resolves a family-nested crate\n' >&2
  printf 'got %s\n' "${nested_directory_output}" >&2
  exit 1
fi
printf 'PASS crate_tree.py --directory resolves a family-nested crate (crates/substrates/ironclaw_webui)\n'

missing_directory_error="$(
  python3 "${script_dir}/lib/crate_tree.py" --directory "ironclaw_ws10_probe_missing" "${repo_root_for_inventory}" 2>&1 >/dev/null
)" && missing_directory_rc=0 || missing_directory_rc=$?
if [ "${missing_directory_rc}" -eq 0 ] || [ "${missing_directory_error#*"found 0"}" = "${missing_directory_error}" ]; then
  printf 'FAIL crate_tree.py --directory refuses an absent crate name\n' >&2
  printf 'rc=%s output=%s\n' "${missing_directory_rc}" "${missing_directory_error}" >&2
  exit 1
fi
printf 'PASS crate_tree.py --directory refuses an absent crate name\n'

# Two crates sharing a basename in different families: `crate_directory`'s own
# "found N, expected 1" refusal, exercised through the CLI flag. Appended to
# nested_root at the very end, after every other assertion that depends on
# its crate SET has already run.
mkdir -p "${nested_root}/crates/substrates/ironclaw_ambiguous_probe" \
  "${nested_root}/crates/domains/ironclaw_ambiguous_probe"
printf '[package]\nname = "ironclaw_ambiguous_probe"\n' \
  > "${nested_root}/crates/substrates/ironclaw_ambiguous_probe/Cargo.toml"
printf '[package]\nname = "ironclaw_ambiguous_probe"\n' \
  > "${nested_root}/crates/domains/ironclaw_ambiguous_probe/Cargo.toml"
ambiguous_directory_error="$(
  python3 "${script_dir}/lib/crate_tree.py" --directory "ironclaw_ambiguous_probe" "${nested_root}" 2>&1 >/dev/null
)" && ambiguous_directory_rc=0 || ambiguous_directory_rc=$?
if [ "${ambiguous_directory_rc}" -eq 0 ] || [ "${ambiguous_directory_error#*"found 2"}" = "${ambiguous_directory_error}" ]; then
  printf 'FAIL crate_tree.py --directory refuses an ambiguous crate name\n' >&2
  printf 'rc=%s output=%s\n' "${ambiguous_directory_rc}" "${ambiguous_directory_error}" >&2
  exit 1
fi
printf 'PASS crate_tree.py --directory refuses an ambiguous crate name\n'

# scripts/ci/crate-dir.sh: a thin exec wrapper, pinned for correctness
# (matches crate_tree.py --directory) and for propagating a non-zero exit.
crate_dir_sh="${script_dir}/crate-dir.sh"
crate_dir_sh_output="$("${crate_dir_sh}" "${first_python_crate_name}" "${repo_root_for_inventory}")"
if [ "${crate_dir_sh_output}" != "${first_python_crate}" ]; then
  printf 'FAIL crate-dir.sh resolves the same directory as crate_tree.py --directory\n' >&2
  printf 'expected %s got %s\n' "${first_python_crate}" "${crate_dir_sh_output}" >&2
  exit 1
fi
printf 'PASS crate-dir.sh resolves the same directory as crate_tree.py --directory (%s)\n' \
  "${first_python_crate_name}"

if "${crate_dir_sh}" "ironclaw_ws10_probe_missing" "${repo_root_for_inventory}" >/dev/null 2>&1; then
  printf 'FAIL crate-dir.sh refuses an absent crate name\n' >&2
  exit 1
fi
printf 'PASS crate-dir.sh refuses an absent crate name\n'
