#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 5 ]]; then
  echo "usage: $0 <changelog> <targets-tsv> <owner/repository> <release-tag> <release-version>" >&2
  exit 2
fi

changelog_path="$1"
targets_path="$2"
repository="$3"
release_tag="$4"
release_version="$5"

if [[ ! -f "$changelog_path" || ! -r "$changelog_path" ]]; then
  echo "changelog is not a readable regular file: $changelog_path" >&2
  exit 1
fi
if [[ ! -f "$targets_path" || ! -r "$targets_path" ]]; then
  echo "release targets are not a readable regular file: $targets_path" >&2
  exit 1
fi
if [[ ! "$repository" =~ ^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$ ]]; then
  echo "repository must use the owner/name form: $repository" >&2
  exit 1
fi
if [[ "$release_tag" != "ironclaw-v$release_version" ]]; then
  echo "release tag $release_tag does not match version $release_version" >&2
  exit 1
fi

extract_changelog_section() {
  local section="$1"
  awk -v section="$section" '
    function is_section_header(line, name, marker, suffix) {
      marker = "## [" name "]"
      if (substr(line, 1, length(marker)) != marker) {
        return 0
      }
      suffix = substr(line, length(marker) + 1, 1)
      return suffix == "" || suffix == " " || suffix == "("
    }

    !active && is_section_header($0, section) {
      active = 1
      next
    }
    active && /^## \[/ {
      exit
    }
    active {
      if ($0 ~ /[^[:space:]]/) {
        for (i = 0; i < pending_blank_lines; i++) {
          print ""
        }
        pending_blank_lines = 0
        print
        emitted = 1
      } else if (emitted) {
        pending_blank_lines++
      }
    }
  ' "$changelog_path"
}

# A prepared version section always takes precedence. Prereleases commonly keep
# their notes under Unreleased, matching the legacy cargo-dist release body.
release_notes="$(extract_changelog_section "$release_version")"
release_core="${release_version%%+*}"
if [[ -z "$release_notes" && "$release_core" == *-* ]]; then
  release_notes="$(extract_changelog_section Unreleased)"
fi
if [[ -z "$release_notes" ]]; then
  echo "CHANGELOG.md has no publishable notes for [$release_version]" >&2
  exit 1
fi

download_targets=()
download_platforms=()
seen_targets="|"
target_count=0
while IFS= read -r target_row || [[ -n "$target_row" ]]; do
  if [[ "$target_row" != *$'\t'* || "${target_row#*$'\t'}" == *$'\t'* ]]; then
    echo "release target row must contain exactly one tab: $target_row" >&2
    exit 1
  fi
  target="${target_row%%$'\t'*}"
  platform="${target_row#*$'\t'}"
  if [[ ! "$target" =~ ^[0-9A-Za-z_.-]+$ \
    || -z "$platform" \
    || "$platform" == *"|"* \
    || "$platform" == *$'\r'* ]]; then
    echo "invalid release target row: $target" >&2
    exit 1
  fi
  if [[ "$seen_targets" == *"|$target|"* ]]; then
    echo "duplicate release target: $target" >&2
    exit 1
  fi
  seen_targets+="$target|"
  download_targets+=("$target")
  download_platforms+=("$platform")
  target_count=$((target_count + 1))
done < "$targets_path"
if [[ "$target_count" -ne 7 ]]; then
  echo "release target manifest must contain exactly seven targets" >&2
  exit 1
fi

release_base_url="https://github.com/$repository/releases/download/$release_tag"

printf '## Release Notes\n\n%s\n\n' "$release_notes"
printf '## Download ironclaw %s\n\n' "$release_version"
printf '|  File  | Platform | Checksum |\n'
printf '|--------|----------|----------|\n'
for index in "${!download_targets[@]}"; do
  target="${download_targets[$index]}"
  platform="${download_platforms[$index]}"
  archive="ironclaw-$target.tar.gz"
  printf '| [%s](%s/%s) | %s | [checksum](%s/%s.sha256) |\n' \
    "$archive" \
    "$release_base_url" \
    "$archive" \
    "$platform" \
    "$release_base_url" \
    "$archive"
done
