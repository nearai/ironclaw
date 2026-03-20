#!/usr/bin/env bash

load_commit_summary() {
  local range="$1"
  local max_commits="${2:-50}"
  local commit_list overflow

  commit_list="$(git log --oneline --no-merges --reverse "${range}" 2>/dev/null || echo "")"
  if [ -n "${commit_list}" ]; then
    COMMIT_COUNT="$(printf '%s\n' "${commit_list}" | wc -l | tr -d ' ')"
    if [ "${COMMIT_COUNT}" -gt "${max_commits}" ]; then
      COMMIT_MD="$(printf '%s\n' "${commit_list}" | head -n "${max_commits}" | sed 's/^/- /')"
      overflow=$((COMMIT_COUNT - max_commits))
      COMMIT_MD+=$'\n'"- ... and ${overflow} more (see compare view)"
    else
      COMMIT_MD="$(printf '%s\n' "${commit_list}" | sed 's/^/- /')"
    fi
  else
    COMMIT_COUNT=0
    COMMIT_MD="- (no non-merge commits in range)"
  fi
}

replace_marked_section() {
  local body_file="$1"
  local section_file="$2"
  local section_start="$3"
  local section_end="$4"
  local output_file="$5"

  if grep -qF "${section_start}" "${body_file}" && grep -qF "${section_end}" "${body_file}"; then
    awk -v start="${section_start}" -v end="${section_end}" -v replacement_file="${section_file}" '
      BEGIN {
        while ((getline line < replacement_file) > 0) {
          replacement = replacement line ORS
        }
        in_block = 0
      }
      $0 == start {
        printf "%s", replacement
        in_block = 1
        next
      }
      $0 == end {
        in_block = 0
        next
      }
      !in_block {
        print
      }
    ' "${body_file}" > "${output_file}"
  else
    cp "${body_file}" "${output_file}"
    if [ -s "${output_file}" ]; then
      printf '\n\n' >> "${output_file}"
    fi
    cat "${section_file}" >> "${output_file}"
  fi
}
