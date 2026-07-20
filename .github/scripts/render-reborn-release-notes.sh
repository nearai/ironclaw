#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: $0 <changelog> <owner/repository> <release-tag> <release-version>" >&2
  exit 2
fi

changelog_path="$1"
repository="$2"
release_tag="$3"
release_version="$4"

if [[ ! -f "$changelog_path" || ! -r "$changelog_path" ]]; then
  echo "changelog is not a readable regular file: $changelog_path" >&2
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

download_rows=(
  "aarch64-apple-darwin|Apple Silicon macOS"
  "x86_64-apple-darwin|Intel macOS"
  "x86_64-pc-windows-msvc|x64 Windows"
  "aarch64-unknown-linux-gnu|ARM64 Linux"
  "x86_64-unknown-linux-gnu|x64 Linux"
  "aarch64-unknown-linux-musl|ARM64 MUSL Linux"
  "x86_64-unknown-linux-musl|x64 MUSL Linux"
)
release_base_url="https://github.com/$repository/releases/download/$release_tag"

printf '## Release Notes\n\n%s\n\n' "$release_notes"
printf '## Download ironclaw %s\n\n' "$release_version"
printf '|  File  | Platform | Checksum |\n'
printf '|--------|----------|----------|\n'
for row in "${download_rows[@]}"; do
  target="${row%%|*}"
  platform="${row#*|}"
  archive="ironclaw-$target.tar.gz"
  printf '| [%s](%s/%s) | %s | [checksum](%s/%s.sha256) |\n' \
    "$archive" \
    "$release_base_url" \
    "$archive" \
    "$platform" \
    "$release_base_url" \
    "$archive"
done
