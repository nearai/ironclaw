#!/usr/bin/env bash
set -euo pipefail

bucket_name="${BUCKET_NAME:?BUCKET_NAME must be set}"
packages_json="${BUCKET_PACKAGES:?BUCKET_PACKAGES must be set}"
test_timeout="${REBORN_CRATE_TEST_TIMEOUT:-28m}"

if ! jq -e 'type == "array" and all(.[]?; type == "string")' >/dev/null 2>&1 <<< "${packages_json}"; then
  echo "BUCKET_PACKAGES must be a JSON array of package-name strings" >&2
  exit 1
fi

packages=()
while IFS= read -r package; do
  packages+=("${package}")
done < <(printf '%s\n' "${packages_json}" | jq -r '.[]')

if [ "${#packages[@]}" -eq 0 ]; then
  echo "Reborn crate bucket '${bucket_name}' has no packages" >&2
  exit 1
fi

features=()
disable_incremental=false
for package in "${packages[@]}"; do
  if [ "${package}" = "ironclaw_reborn_composition" ]; then
    disable_incremental=true
  fi

  feature_flags="$(scripts/ci/package-feature-flags.sh "${package}")"
  if [ -z "${feature_flags}" ]; then
    continue
  fi

  case "${feature_flags}" in
    --features\ *)
      feature_csv="${feature_flags#--features }"
      ;;
    *)
      echo "Unsupported feature flags for ${package}: ${feature_flags}" >&2
      exit 1
      ;;
  esac

  package_features=()
  IFS=',' read -r -a package_features <<< "${feature_csv}"
  for feature in "${package_features[@]}"; do
    if [ -n "${feature}" ]; then
      features+=("${package}/${feature}")
    fi
  done
done

if [ "${disable_incremental}" = "true" ]; then
  echo "Disabling incremental compilation for ironclaw_reborn_composition to keep runner disk usage bounded."
  export CARGO_INCREMENTAL=0
fi

cargo_args=(cargo test)
for package in "${packages[@]}"; do
  cargo_args+=(-p "${package}")
done

if [ "${#features[@]}" -gt 0 ]; then
  feature_arg="$(IFS=,; printf '%s' "${features[*]}")"
  cargo_args+=(--features "${feature_arg}")
fi

cargo_args+=(--all-targets -- --nocapture)

echo "Running Reborn crate bucket: ${bucket_name}"
printf '  - %s\n' "${packages[@]}"
if [ "${#features[@]}" -gt 0 ]; then
  echo "Package-scoped features:"
  printf '  - %s\n' "${features[@]}"
fi

echo "::group::${cargo_args[*]}"
timeout --signal=INT --kill-after=30s "${test_timeout}" "${cargo_args[@]}"
echo "::endgroup::"
