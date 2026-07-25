#!/usr/bin/env bash
#
# Local parity runner for the Reborn coverage ratchet that gates
# .github/workflows/reborn-tests.yml. This is intentionally heavy: it runs the
# same instrumented crate-bucket coverage and the same five instrumented
# integration coverage lanes, then merges LCOV and applies the committed
# coverage floor.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script_dir="${repo_root}/scripts/ci"
out_dir="${IRONCLAW_REBORN_LOCAL_COVERAGE_DIR:-${repo_root}/target/reborn-local-coverage-ratchet}"

cd "${repo_root}"

if ! command -v jq >/dev/null 2>&1; then
    echo "ERROR: jq is required for the Reborn local coverage ratchet." >&2
    exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
    cat >&2 <<'EOF'
ERROR: cargo-llvm-cov is required for the Reborn local coverage ratchet.

Install it with:
  cargo install cargo-llvm-cov
EOF
    exit 1
fi

run_cargo_cov_env() {
    env \
        -u ANTHROPIC_API_KEY \
        -u LLM_BACKEND \
        -u LLM_USE_CODEX_AUTH \
        -u OLLAMA_BASE_URL \
        -u OPENAI_API_KEY \
        -u NEARAI_API_KEY \
        -u NEARAI_BASE_URL \
        -u NEARAI_SESSION_TOKEN \
        -u NEARAI_PROVIDER_ID \
        -u NEARAI_MODEL \
        -u IRONCLAW_LLM_PROVIDER \
        -u IRONCLAW_LLM_MODEL \
        IRONCLAW_DISABLE_OS_KEYCHAIN="${IRONCLAW_DISABLE_OS_KEYCHAIN:-1}" \
        CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" \
        CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}" \
        CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}" \
        RUST_MIN_STACK="${RUST_MIN_STACK:-8388608}" \
        "$@"
}

echo "==> reborn coverage ratchet: preparing ${out_dir}"
mkdir -p "${out_dir}/packages" "${out_dir}/buckets" "${out_dir}/lanes"
find "${out_dir}/packages" "${out_dir}/buckets" "${out_dir}/lanes" -type f -name '*.lcov' -delete

allowlist_packages="$(
    cargo metadata --no-deps --format-version 1 \
        | jq -c '
            [
              .packages[]
              | select(
                  (
                    (.name == "ironclaw")
                    or (.name == "ironclaw_runner")
                    or (.name | startswith("ironclaw_reborn"))
                    or (.name | startswith("ironclaw_product"))
                    or (.name == "ironclaw_architecture")
                    or (.name == "ironclaw_slack_extension")
                    or (.name == "ironclaw_telegram_extension")
                    or (.name == "ironclaw_telegram_v2_adapter")
                    or (.name | startswith("ironclaw_webui"))
                  )
                  and (.name != "ironclaw_reborn_integration_tests")
                )
              | .name
            ]
            | unique
          '
)"

closure_packages="$(
    comm -12 \
        <(cargo tree -p ironclaw -e normal,build --prefix none \
            | grep -oE 'ironclaw_[a-z0-9_]+' \
            | sort -u) \
        <(cargo metadata --no-deps --format-version 1 \
            | jq -r '.packages[].name' \
            | sort -u) \
        | jq -R -s -c 'split("\n") | map(select(length > 0))'
)"

packages="$(
    jq -n -c \
        --argjson allowlist "${allowlist_packages}" \
        --argjson closure "${closure_packages}" \
        '$allowlist + $closure | unique'
)"

if [ -z "${packages}" ] || [ "${packages}" = "[]" ]; then
    echo "No Reborn workspace crates discovered" >&2
    exit 1
fi

buckets="$("${script_dir}/reborn-crate-test-buckets.sh" "${packages}")"

echo "==> reborn coverage ratchet: instrumented crate buckets"
while IFS= read -r bucket; do
    bucket_name="$(jq -r '.name' <<<"${bucket}")"
    echo "==> reborn coverage bucket: ${bucket_name}"
    while IFS= read -r package; do
        feature_flags="$("${script_dir}/package-feature-flags.sh" "${package}")"
        package_lcov="${out_dir}/packages/${package}.lcov"
        echo "==> cargo llvm-cov -p ${package} ${feature_flags} test"
        cargo llvm-cov clean --profraw-only

        incremental_env=()
        if [ "${package}" = "ironclaw_reborn_composition" ]; then
            incremental_env=(env CARGO_INCREMENTAL=0)
        fi

        # shellcheck disable=SC2086 # feature_flags intentionally expands to zero or more Cargo args.
        run_cargo_cov_env "${incremental_env[@]}" cargo llvm-cov -p "${package}" ${feature_flags} --all-targets \
            --lcov --output-path "${package_lcov}" \
            test -- --nocapture
    done < <(jq -r '.packages[]' <<<"${bucket}")

    bucket_lcov="${out_dir}/buckets/bucket-${bucket_name}.lcov"
    package_parts=()
    while IFS= read -r package; do
        package_parts+=("${out_dir}/packages/${package}.lcov")
    done < <(jq -r '.packages[]' <<<"${bucket}")
    "${script_dir}/reborn-coverage-merge-lcov.sh" "${bucket_lcov}" "${package_parts[@]}"
done < <(jq -c '.[]' <<<"${buckets}")

echo "==> reborn coverage ratchet: instrumented integration lanes"
for lane in 0 1 2 3 groups; do
    mode="flat-partition"
    lane_index="${lane}"
    if [ "${lane}" = "groups" ]; then
        mode="group"
        lane_index="0"
    fi

    echo "==> reborn coverage lane: ${lane}"
    cargo llvm-cov clean --profraw-only
    run_cargo_cov_env env \
        REBORN_COV_LANE_MODE="${mode}" \
        REBORN_COV_LANE_PARTITIONS=4 \
        REBORN_COV_LANE_INDEX="${lane_index}" \
        REBORN_COV_LANE_TEST_TIMEOUT="${REBORN_COV_LANE_TEST_TIMEOUT:-45m}" \
        "${script_dir}/reborn-coverage-lane-run.sh" "${out_dir}/lanes/part-${lane}.lcov"
done

parts=()
while IFS= read -r path; do
    parts+=("${path}")
done < <(find "${out_dir}/buckets" "${out_dir}/lanes" -type f -name '*.lcov' | LC_ALL=C sort)

if [ "${#parts[@]}" -eq 0 ]; then
    echo "No Reborn coverage LCOV parts were produced" >&2
    exit 1
fi

merged_lcov="${out_dir}/reborn-integration-merged.lcov"
"${script_dir}/reborn-coverage-merge-lcov.sh" "${merged_lcov}" "${parts[@]}"
"${script_dir}/reborn-coverage-summary.sh" "${merged_lcov}" tests/integration/coverage-exemptions.toml
"${script_dir}/reborn-coverage-ratchet.sh" \
    "${merged_lcov}" \
    tests/integration/coverage-exemptions.toml \
    tests/integration/coverage-floor.toml
