#!/usr/bin/env bash
# Caller-level sabotage tests for the recorded-fixture checker.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
checker="${repo_root}/scripts/ci/check-reborn-qa-fixtures.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-qa-fixtures.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

passes=0
failures=0
capture() {
  set +e
  CAP_OUT="$("$@" 2>&1)"
  CAP_RC=$?
  set -e
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

fixtures="${work}/fixtures"
mkdir -p "${fixtures}"

echo "▶ valid fixture"
printf '%s\n' '{"schema_version": 1, "turns": []}' >"${fixtures}/valid.json"
capture "${checker}" "${fixtures}"
check_rc "a valid object fixture passes" 0

echo "▶ malformed fixture sabotage"
printf '%s\n' '{"schema_version": 1,' >"${fixtures}/malformed.json"
capture "${checker}" "${fixtures}"
check_rc "invalid JSON fails" 1
check_text "malformed file is named" "malformed.json:2"
check_text "parse failure is classified" "malformed JSON fixture"

printf '%s\n' '[]' >"${fixtures}/malformed.json"
capture "${checker}" "${fixtures}"
check_rc "a non-object fixture root fails" 1
check_text "root shape failure is actionable" "root must be an object"

echo "▶ empty discovery sabotage"
rm "${fixtures}/malformed.json" "${fixtures}/valid.json"
capture "${checker}" "${fixtures}"
check_rc "zero fixture discovery fails" 1
check_text "empty discovery names the fixture directory" "no Reborn QA fixture JSON files found"

echo
if [ "${failures}" -ne 0 ]; then
  echo "${failures} QA fixture checker self-test(s) failed" >&2
  exit 1
fi
echo "all ${passes} QA fixture checker self-tests passed"
