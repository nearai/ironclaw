#!/usr/bin/env bash
#
# Classify a list of changed paths (one per line on stdin) into the test scopes
# the CI workflows select on: docs_only / has_core_code / has_legacy_tests /
# has_reborn_tests.
#
# Crate paths are normalized to `crates/<crate>/...` BEFORE the case arms run,
# so the arms below stay keyed to crate identity rather than to how deep the
# crate sits under `crates/`. Without that, the target-architecture family move
# (`crates/ironclaw_event_log` -> `crates/substrates/ironclaw_event_log`, PROPOSAL §5)
# makes every crate-scoped arm stop matching: the paths still look like code, so
# they fall through to `is_code_path` and get bucketed legacy-only, and the whole
# Reborn suite is silently skipped on a green PR. See
# docs/internal/reborn/target-architecture/CHECKLIST.md WS10.
#
# Test/override env vars (unset in prod):
#   IRONCLAW_REPO_ROOT  repository root whose crate tree defines the inventory
#                       (default: this script's ../..)
#
# Exit codes: 0 = classified ; 1 = a `crates/` path could not be attributed to
# a crate, or crate discovery failed. Both are refusals, not classifications.

set -euo pipefail

has_core_code=false
docs_only=true
has_legacy_tests=false
has_reborn_tests=false

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="${IRONCLAW_REPO_ROOT:-$(cd "${script_dir}/../.." && pwd)}"

# Fail-closed floor for crate discovery. Mirrors MIN_CRATE_DIRECTORIES in
# scripts/ci/lib/crate_tree.py, which owns the same rule for the Python-side
# gates; test-classify-test-scope.sh pins the two inventories equal.
min_crate_directories=20

# Newline-delimited inventory of directories under `crates/` that own a
# Cargo.toml, at any depth. Nested manifests (ironclaw_safety/fuzz) are kept in
# the list but never win a lookup: resolution walks path segments outward-in
# and stops at the first hit, so the *outermost* crate always owns the path —
# exactly what `crates/<crate>/*` globs did before.
#
# Manifests that declare their own `[workspace]` table are skipped entirely:
# they root a *separate* workspace (the `wasm-src/` guest components), so this
# workspace never builds them and no test run can cover them. `scripts/ci/lib/crate_tree.py` applies the same rule and
# `test-classify-test-scope.sh` pins the two inventories equal.
crate_dirs=""
workspace_root_dirs=""
discover_crate_dirs() {
  local crates_root="${repo_root}/crates"
  if [ ! -d "${crates_root}" ]; then
    echo "classify-test-scope: no crates/ directory under ${repo_root}; refusing to" \
      "classify against an empty crate inventory (CHECKLIST WS10)." >&2
    exit 1
  fi

  local manifest dir relative count
  while IFS= read -r manifest; do
    # `manifest` is relative to crates_root (the find below runs from inside
    # it, see below) — `dir`/`relative` must be rebuilt from it, never from
    # the search root's own absolute prefix. `-not -path '*/.*'` matches
    # against find's PRINTED path: searching from an absolute crates_root
    # would make it also match any dot-component OF THE REPO CHECKOUT PATH
    # itself (e.g. a worktree under `.claude/worktrees/...`), excluding every
    # result. Running `find .` from inside crates_root confines the exclusion
    # to paths actually under crates/, matching what crate_tree.py's
    # `_is_skipped` already does by checking relative parts only.
    relative="${manifest#./}"
    dir="crates/${relative%/Cargo.toml}"
    # `grep -q '^\[workspace\]$'` — line-anchored so `[workspace.dependencies]`
    # in a member manifest is not mistaken for a workspace root.
    if grep -qx '\[workspace\]' "${crates_root}/${relative}" 2>/dev/null; then
      workspace_root_dirs="${workspace_root_dirs}${dir}
"
      continue
    fi
    crate_dirs="${crate_dirs}${dir}
"
  done < <(
    (cd "${crates_root}" && find . -type f -name Cargo.toml \
      -not -path '*/target/*' -not -path '*/.*' 2>/dev/null) | sort
  )

  count="$(printf '%s' "${crate_dirs}" | grep -c . || true)"
  if [ "${count}" -lt "${min_crate_directories}" ]; then
    echo "classify-test-scope: found only ${count} crate director(ies) under" \
      "${crates_root} (floor ${min_crate_directories}). The crate tree moved out from" \
      "under this classifier, or the repo root is wrong. Refusing to bucket every" \
      "crate change as out-of-scope (CHECKLIST WS10)." >&2
    exit 1
  fi
}

newline='
'

# Sets OWNING_CRATE_DIR to the crate directory that owns "$1"; returns 1 if the
# path is inside no known crate directory.
OWNING_CRATE_DIR=""
owning_crate_dir() {
  local rest="${1#crates/}" acc="crates" segment
  OWNING_CRATE_DIR=""
  while [ -n "${rest}" ]; do
    segment="${rest%%/*}"
    if [ "${segment}" = "${rest}" ]; then
      rest=""
    else
      rest="${rest#*/}"
    fi
    acc="${acc}/${segment}"
    case "${newline}${crate_dirs}" in
      *"${newline}${acc}${newline}"*)
        OWNING_CRATE_DIR="${acc}"
        return 0
        ;;
    esac
  done
  return 1
}

# True when "$1" is package data under `extensions/packages/`. Anchored on the
# support crate rather than a literal path, so a family move keeps it working:
# `packages/` is that crate's sibling. Sets PACKAGE_ASSET_PACKAGES_DIR to the
# matched `packages/` directory (whatever depth it actually sits at) so the
# caller can normalize the path onto it — see normalize_crate_path below.
PACKAGE_ASSET_PACKAGES_DIR=""
package_asset_dir() {
  local path="$1" support_dir packages_dir
  PACKAGE_ASSET_PACKAGES_DIR=""
  while IFS= read -r support_dir; do
    [ -n "${support_dir}" ] || continue
    case "${support_dir}" in
      */ironclaw_extension_support)
        packages_dir="${support_dir%/*}/packages"
        case "${path}" in
          "${packages_dir}"/*)
            PACKAGE_ASSET_PACKAGES_DIR="${packages_dir}"
            return 0
            ;;
        esac
        ;;
    esac
  done <<EOF
${crate_dirs}
EOF
  return 1
}

# True when "$1" lies inside a directory that roots a separate cargo workspace.
nested_workspace_dir() {
  local path="$1" root
  while IFS= read -r root; do
    [ -n "${root}" ] || continue
    case "${path}" in
      "${root}"/*) return 0 ;;
    esac
  done <<EOF
${workspace_root_dirs}
EOF
  return 1
}

# Sets NORMALIZED_PATH: `crates/<...>/<crate>/<rest>` becomes `crates/<crate>/<rest>`,
# everything else passes through. On today's flat tree every crate directory
# already is `crates/<crate>`, so this is a no-op for every path in the
# repository. Exits 1 (not a subshell — the caller must die too) when a
# `crates/` path cannot be attributed at all.
NORMALIZED_PATH=""
normalize_crate_path() {
  local path="$1" tail
  NORMALIZED_PATH="${path}"

  case "${path}" in
    crates/*) ;;
    *) return 0 ;;
  esac

  tail="${path#crates/}"
  case "${tail}" in
    */*) ;;
    # A file sitting directly in crates/ (AGENTS.md, README.md) belongs to no
    # crate and never did.
    *) return 0 ;;
  esac

  if owning_crate_dir "${path}"; then
    # The crate DIRECTORY basename is the key, not the cargo package name.
    # Usually they are identical; two documented exceptions make the package
    # name the wrong choice here (PROPOSAL §5.1's directory rule):
    # `crates/ironclaw_cli` declares `name = "ironclaw"`, and package
    # directories under `extensions/packages/` are named by extension identity
    # (`packages/slack/` holds `ironclaw_slack_extension`). The arms below are
    # therefore keyed on directory names, and every package directory needs its
    # own arm — a missing one is the silent mis-bucketing this classifier
    # exists to prevent, so `test-classify-test-scope.sh` probes each of them.
    NORMALIZED_PATH="crates/${OWNING_CRATE_DIR##*/}/${path#"${OWNING_CRATE_DIR}/"}"
    return 0
  fi

  # Two shapes under `crates/` own no crate BY DESIGN, and refusing on them
  # would be wrong — they are attributable, just not to a crate:
  #
  #   * a data-only package directory under `extensions/packages/`, which is
  #     manifest + prompts + schemas + committed wasm and deliberately carries
  #     no `Cargo.toml` (PROPOSAL §5). Its data is embedded by
  #     `ironclaw_extension_support`, which is where it used to live, so it
  #     lights the same lane that crate does.
  #   * a separate cargo workspace (the `wasm-src/` guest components).
  #
  # Checked in this order and NOT symmetrically: a package-asset path is
  # rewritten onto the canonical `crates/extensions/packages/<rest>` identity
  # (the same treatment the crate-owned branch above gives its match), while a
  # non-package workspace root passes through unchanged, matching its
  # literal-path arms below. Without the rewrite, a
  # data-only package's path stays literally wherever `packages/` sits
  # (`crates/<family>/packages/...`) after the family move (PROPOSAL §5), the
  # `crates/extensions/packages/*` arms in is_shared_test_path stop matching,
  # and the file falls through to `is_code_path`'s bare `crates/*` arm —
  # bucketing it has_reborn_tests=false where it was true. A `wasm-src/` guest
  # inside the SAME package would be caught by package_asset_dir first (it is
  # also under `packages/`) and gets the identical rewrite, so its own
  # `nested_workspace_dir` membership never needs to be consulted here.
  if package_asset_dir "${path}"; then
    NORMALIZED_PATH="crates/extensions/packages/${path#"${PACKAGE_ASSET_PACKAGES_DIR}/"}"
    return 0
  fi
  if nested_workspace_dir "${path}"; then
    return 0
  fi

  # The crate directory is absent from the checkout — the normal case for a
  # path the diff *deletes*. Fall back to the naming convention so a
  # crate-removal PR classifies exactly as it did before this normalization
  # existed.
  case "${tail}" in
    ironclaw_*/*)
      return 0
      ;;
    */ironclaw_*/*)
      # Drop every family segment ahead of the crate, not just the first, so the
      # fallback is as depth-independent as the tree lookup above it. Anchored on
      # the FIRST `ironclaw_` segment (`%%` keeps the shortest prefix): a greedy
      # match on the last one would fold
      # `crates/f/ironclaw_event_log/src/ironclaw_helper.rs` down to
      # `crates/ironclaw_helper.rs`.
      NORMALIZED_PATH="crates/${tail#"${tail%%/ironclaw_*}"/}"
      return 0
      ;;
  esac

  # A direct child file of a FAMILY directory (`crates/app/AGENTS.md`,
  # `crates/kernel/AGENTS.md`, ...) is attributable but crate-less, exactly like
  # a file sitting directly in `crates/` — the family dirs exist since WS7 and
  # each carries its own guidance file. Recognized structurally: the parent
  # directory is a prefix of at least one discovered crate dir, and the path
  # has no further directory below it. Falls through to the same buckets as
  # `crates/AGENTS.md` (code-adjacent, legacy suite) — conservative, never
  # silent.
  case "${tail}" in
    */*/*) ;;
    */*)
      local family_seg="crates/${tail%%/*}/"
      case "${crate_dirs}" in
        *"${family_seg}"*)
          return 0
          ;;
      esac
      ;;
  esac

  # Default arm: a path under crates/ that resolves to no crate, by tree or by
  # convention. Historically this silently fell through to "legacy tests only",
  # which is how a re-shaped crate tree drops the Reborn suite. Refuse instead.
  echo "classify-test-scope: cannot attribute '${path}' to any crate under" \
    "${repo_root}/crates. Add the crate to the tree, or teach this classifier the" \
    "new path shape — silently bucketing it would skip whichever suite owns it" \
    "(CHECKLIST WS10)." >&2
  exit 1
}

is_docs_only_path() {
  local path="$1"
  case "$path" in
    docs/*|.github/ISSUE_TEMPLATE/*|.github/pull_request_template.md)
      return 0
      ;;
    *.md)
      case "$path" in
        */*) return 1 ;;
        *) return 0 ;;
      esac
      ;;
    *)
      return 1
      ;;
  esac
}

is_shared_test_path() {
  local path="$1"
  case "$path" in
    Cargo.toml|Cargo.lock|crates/ironclaw_llm/assets/providers.json|Dockerfile)
      return 0
      ;;
    scripts/ci/classify-test-scope.sh|scripts/ci/test-classify-test-scope.sh|scripts/ci/package-feature-flags.sh)
      return 0
      ;;
    .github/workflows/reborn-tests.yml|.github/workflows/reborn-e2e.yml|.github/workflows/nightly-deep-ci.yml)
      return 0
      ;;
    crates/ironclaw_common/*|crates/ironclaw_extension_contracts/*|crates/ironclaw_host_api/*|crates/ironclaw_host_runtime/*|crates/ironclaw_loop_contracts/*|crates/ironclaw_loop_host/*|crates/ironclaw_processes/*|crates/ironclaw_product_contracts/*)
      return 0
      ;;
    crates/ironclaw_filesystem/*|crates/ironclaw_memory/*|crates/ironclaw_event_log/*|crates/ironclaw_event_projections/*|crates/ironclaw_event_streams/*)
      return 0
      ;;
    crates/ironclaw_capabilities/*|crates/ironclaw_secrets/*|crates/ironclaw_network/*|crates/ironclaw_runtime_policy/*)
      return 0
      ;;
    crates/ironclaw_authorization/*|crates/ironclaw_approvals/*|crates/ironclaw_resources/*)
      return 0
      ;;
    crates/ironclaw_auth/*|crates/ironclaw_trust/*|crates/ironclaw_turns/*|crates/ironclaw_agent_loop/*|crates/ironclaw_threads/*)
      return 0
      ;;
    crates/ironclaw_prompt_envelope/*|crates/ironclaw_hooks/*|crates/ironclaw_extension_support/*|crates/ironclaw_llm/*|\
    crates/extensions/packages/*)
      return 0
      ;;
    crates/ironclaw_safety/*|crates/ironclaw_skills/*|crates/ironclaw_oauth/*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_reborn_test_path() {
  local path="$1"
  case "$path" in
    docs/internal/reborn/*|scripts/reborn-e2e-rust.sh|scripts/ci/run-reborn-root-partition.sh|scripts/ci/run-reborn-group-tests.sh|scripts/ci/check-reborn-responses-e2e-manifest.py|tests/reborn_*|tests/integration/*|tests/support/reborn_parity_qa/*|tests/fixtures/llm_traces/reborn_qa/*|tests/e2e/reborn_coverage_tests.txt|tests/e2e/reborn_responses_e2e_tests.txt|tests/e2e/scenarios/test_reborn_*)
      return 0
      ;;
    crates/ironclaw_architecture_tests/*)
      return 0
      ;;
    # The WS6 renames dropped the `ironclaw_reborn_` prefix from all seven
    # crates that carried it, so the `crates/ironclaw_reborn_*/*` glob this
    # arm used to rely on now matches NOTHING and every one of these crates
    # silently reclassified as legacy. Enumerated rather than re-globbed:
    # the new names share no prefix, and a glob that matches nothing fails
    # open (quietly) instead of closed.
    crates/ironclaw_turn_runner/*|\
    crates/ironclaw_cli/*|\
    crates/ironclaw_composition/*|\
    crates/ironclaw_config/*|\
    crates/ironclaw_event_store/*|\
    crates/ironclaw_identity/*|\
    crates/ironclaw_openai_compat/*|\
    crates/ironclaw_trace_commons/*)
      return 0
      ;;
    crates/ironclaw_product_*/*|\
    crates/slack/*|crates/telegram/*|crates/memory-native/*|crates/mem0/*)
      return 0
      ;;
    crates/ironclaw_webui/*)
      return 0
      ;;
    crates/ironclaw_conversations/*|crates/ironclaw_extension_host/*|crates/ironclaw_extension_manager/*|crates/ironclaw_outbound/*|crates/ironclaw_assistant/*|crates/ironclaw_triggers/*)
      return 0
      ;;
    scripts/ci/reborn-coverage-*.sh|scripts/ci/test-reborn-coverage.sh|scripts/ci/test-reborn-coverage-*.sh|scripts/ci/reborn_changed_coverage.py|scripts/ci/test_reborn_changed_coverage.py|scripts/ci/critical_mutation_gate.py|scripts/ci/test-critical-mutation-gate.sh|scripts/ci/check-reborn-branch-coverage-flags.py|scripts/ci/test-check-reborn-branch-coverage-flags.sh|scripts/ci/check-reborn-qa-fixtures.sh|scripts/ci/test-check-reborn-qa-fixtures.sh|scripts/ci/lib/reborn_coverage_lcov.py|scripts/ci/reborn-crate-test-buckets.sh|scripts/ci/test-reborn-crate-test-buckets.sh|scripts/ci/ws12-suite-shards.toml|scripts/ci/ws12_suite_shards.py|scripts/ci/test_ws12_suite_shards.py|scripts/ci/ws12_workflow_contracts.py|scripts/ci/test_ws12_workflow_contracts.py|scripts/ci/lib/rust_toolchain_contracts.py|scripts/ci/lib/workflow_text.py|scripts/ci/check-test-suite-boundaries.sh|scripts/ci/classify-test-scope.sh|scripts/ci/test-classify-test-scope.sh)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_code_path() {
  local path="$1"
  case "$path" in
    crates/*|tests/*|migrations/*)
      return 0
      ;;
    Cargo.toml|Cargo.lock|Dockerfile)
      return 0
      ;;
    scripts/check_no_panics.py|scripts/build-wasm-extensions.sh|scripts/check-version-bumps.sh|scripts/reborn-e2e-rust.sh|scripts/ci/*)
      return 0
      ;;
    .github/workflows/*.yml|.github/actions/install-cargo-component/*|.github/dependabot.yml|.github/labeler.yml)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

discover_crate_dirs

while IFS= read -r raw_path || [ -n "$raw_path" ]; do
  [ -n "$raw_path" ] || continue

  normalize_crate_path "$raw_path"
  path="${NORMALIZED_PATH}"

  if ! is_docs_only_path "$path"; then
    docs_only=false
  fi

  if is_code_path "$path"; then
    has_core_code=true
  fi

  if is_shared_test_path "$path"; then
    has_legacy_tests=true
    has_reborn_tests=true
  elif is_reborn_test_path "$path"; then
    has_reborn_tests=true
  elif is_code_path "$path"; then
    has_legacy_tests=true
  fi
done

cat <<EOF
docs_only=${docs_only}
has_core_code=${has_core_code}
has_legacy_tests=${has_legacy_tests}
has_reborn_tests=${has_reborn_tests}
EOF
