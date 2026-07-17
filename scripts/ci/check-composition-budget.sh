#!/usr/bin/env bash
#
# Composition mass ratchet gate.
#
# Fails when ironclaw_reborn_composition's share of production crate code grows
# past the committed ceiling in scripts/ci/composition-budget.toml. See that
# file's header for the rationale and the metric definition.
#
# Usage:
#   scripts/ci/check-composition-budget.sh          # run the gate
#   scripts/ci/check-composition-budget.sh --print   # print observed share only, never fail
#
# Test/override env vars (used by test-check-composition-budget.sh; unset in prod):
#   COMPOSITION_SRC   numerator dir      (default: crates/ironclaw_reborn_composition/src)
#   CRATES_ROOT       denominator root   (default: crates)  -> counts $CRATES_ROOT/*/src/**.rs
#   BUDGET_FILE       budget TOML path   (default: scripts/ci/composition-budget.toml)
#
# Exit codes: 0 = within budget (or dry-run) ; 1 = breach (enforcing) or schema error.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

COMPOSITION_SRC="${COMPOSITION_SRC:-crates/ironclaw_reborn_composition/src}"
CRATES_ROOT="${CRATES_ROOT:-crates}"
BUDGET_FILE="${BUDGET_FILE:-scripts/ci/composition-budget.toml}"

print_only=false
[ "${1:-}" = "--print" ] && print_only=true

# Files that are test-only, excluded from the production-code metric. Matches
# `tests.rs` / `test.rs`, `test_*.rs`, `*_test.rs`, `*_tests.rs`, and anything
# under a `/tests/` directory. Inline `#[cfg(test)]` modules inside otherwise-
# production files are NOT excluded (a line-counter cannot parse them) — a
# documented, symmetric residual that applies to numerator and denominator
# alike, so it does not bias composition's *share*.
TEST_FILE_RE='(^|/)(tests?\.rs|test_[^/]*\.rs|[^/]*_tests?\.rs)$|/tests/'

# --- count LOC of production *.rs under a directory (0 if it does not exist) --
count_loc() {
    local dir="$1"
    [ -d "${dir}" ] || { echo 0; return; }
    find "${dir}" -name '*.rs' -type f 2>/dev/null \
        | { grep -vE "${TEST_FILE_RE}" || true; } \
        | tr '\n' '\0' | xargs -0 cat 2>/dev/null | wc -l | tr -d ' '
}

# --- sum LOC of every crates/*/src tree (the denominator) --------------------
count_denominator() {
    local total=0 d loc
    for d in "${CRATES_ROOT}"/*/src; do
        [ -d "${d}" ] || continue
        loc="$(count_loc "${d}")"
        total=$((total + loc))
    done
    echo "${total}"
}

# Dispatch (Arc<dyn>) sub-metric is scoped to composition PRODUCTION files, but
# EXCLUDES slack/ and extension_host/ — those subtrees are owned by the separate
# channel/extension refactor, so this gate must not trip on or govern their work.
DISPATCH_EXCLUDE_RE='/(slack|extension_host)/'

# --- count Arc<dyn> dispatch sites in governed composition production code ----
count_arc_dyn() {
    [ -d "${COMPOSITION_SRC}" ] || { echo 0; return; }
    # `|| true` on the xargs grep: grep exits 1 (and xargs 123) when there are
    # no Arc<dyn> matches — a valid zero, not a failure — which would otherwise
    # abort the gate under set -e + pipefail.
    find "${COMPOSITION_SRC}" -name '*.rs' -type f 2>/dev/null \
        | { grep -vE "${TEST_FILE_RE}" || true; } \
        | { grep -vE "${DISPATCH_EXCLUDE_RE}" || true; } \
        | tr '\n' '\0' \
        | { xargs -0 grep -ho 'Arc<dyn' 2>/dev/null || true; } | wc -l | tr -d ' '
}

# --- parse one scalar key from the [gate] TOML table -------------------------
# Values are simple: integers, true/false, or "quoted strings". No nested tables.
toml_get() {
    local key="$1"
    # `|| true`: a missing/commented key makes grep exit non-zero, which under
    # `set -e`+`pipefail` would abort BEFORE the schema validation below can
    # emit a clear error. Swallow it so an empty value reaches validation.
    { grep -E "^[[:space:]]*${key}[[:space:]]*=" "${BUDGET_FILE}" || true; } \
        | head -1 \
        | sed -E "s/^[[:space:]]*${key}[[:space:]]*=[[:space:]]*//; s/[[:space:]]*(#.*)?$//; s/^\"//; s/\"$//"
}

fail_schema() { echo "composition-budget: $1" >&2; exit 1; }

[ -f "${BUDGET_FILE}" ] || fail_schema "budget file not found: ${BUDGET_FILE}"

enforce="$(toml_get enforce)"
ceiling_bp="$(toml_get ceiling_bp)"
tolerance_bp="$(toml_get tolerance_bp)"
arc_dyn_ceiling="$(toml_get arc_dyn_ceiling)"
arc_dyn_tolerance="$(toml_get arc_dyn_tolerance)"

# Schema validation — manifest bugs always exit 1, regardless of enforce.
case "${enforce}" in
    true|false) ;;
    *) fail_schema "[gate].enforce must be true or false, got '${enforce:-<missing>}'" ;;
esac
[[ "${ceiling_bp}"      =~ ^[0-9]+$ ]] || fail_schema "[gate].ceiling_bp must be an integer, got '${ceiling_bp:-<missing>}'"
[[ "${tolerance_bp}"    =~ ^[0-9]+$ ]] || fail_schema "[gate].tolerance_bp must be an integer, got '${tolerance_bp:-<missing>}'"
[[ "${arc_dyn_ceiling}"   =~ ^[0-9]+$ ]] || fail_schema "[gate].arc_dyn_ceiling must be an integer, got '${arc_dyn_ceiling:-<missing>}'"
[[ "${arc_dyn_tolerance}" =~ ^[0-9]+$ ]] || fail_schema "[gate].arc_dyn_tolerance must be an integer, got '${arc_dyn_tolerance:-<missing>}'"

comp_loc="$(count_loc "${COMPOSITION_SRC}")"
den_loc="$(count_denominator)"

[ "${den_loc}" -gt 0 ] || fail_schema "denominator LOC is 0 — no crates/*/src trees found under '${CRATES_ROOT}'"

# observed basis points, rounded to nearest (integer math via awk)
observed_bp="$(awk -v c="${comp_loc}" -v d="${den_loc}" 'BEGIN { printf "%d", (10000*c/d)+0.5 }')"

fmt_pct() { awk -v bp="$1" 'BEGIN { printf "%.2f", bp/100 }'; }

arc_dyn="$(count_arc_dyn)"

if [ "${print_only}" = true ]; then
    echo "composition share: $(fmt_pct "${observed_bp}")% (${observed_bp} bp) — ${comp_loc} / ${den_loc} LOC"
    echo "composition dispatch: ${arc_dyn} Arc<dyn> (governed prod, excl slack/extension_host)"
    exit 0
fi

effective_ceiling=$((ceiling_bp + tolerance_bp))
effective_arc_ceiling=$((arc_dyn_ceiling + arc_dyn_tolerance))
breached=0

echo "Composition budget gate: $([ "${enforce}" = true ] && echo ENFORCING || echo DRY-RUN)"

# ---- Metric 1: mass (composition share of production crate code) ----
echo "  [mass] composition src : ${comp_loc} LOC of ${den_loc}  ->  $(fmt_pct "${observed_bp}")% (${observed_bp} bp)"
echo "         ceiling         : $(fmt_pct "${ceiling_bp}")% (tol $(fmt_pct "${tolerance_bp}")pp -> effective $(fmt_pct "${effective_ceiling}")% / ${effective_ceiling} bp)"
if [ "${observed_bp}" -gt "${effective_ceiling}" ]; then
    over=$((observed_bp - effective_ceiling))
    prefix=""; [ "${enforce}" = true ] || prefix="[dry-run, would FAIL] "
    echo "  ${prefix}MASS EXCEEDED: composition is $(fmt_pct "${observed_bp}")% of production crate code," \
         "$(fmt_pct "${over}")pp over the effective ceiling of $(fmt_pct "${effective_ceiling}")%."
    echo "    Move behavior OUT of ironclaw_reborn_composition into an owning crate (charter is"
    echo "    assembly-only). See .claude/skills/ironclaw-reborn-architecture-review (item 2)."
    echo "    If justified, raise ceiling_bp in ${BUDGET_FILE} with a PR rationale."
    breached=1
elif [ "$((ceiling_bp - observed_bp))" -gt 100 ]; then
    echo "  NUDGE: mass is $(fmt_pct "$((ceiling_bp - observed_bp))")pp below ceiling — lower ceiling_bp to lock it in."
fi

# ---- Metric 2: dispatch (Arc<dyn> density in governed production code) ----
echo "  [dispatch] Arc<dyn> (excl slack/extension_host): ${arc_dyn}"
echo "             ceiling: ${arc_dyn_ceiling} (tol ${arc_dyn_tolerance} -> effective ${effective_arc_ceiling})"
if [ "${arc_dyn}" -gt "${effective_arc_ceiling}" ]; then
    over=$((arc_dyn - effective_arc_ceiling))
    prefix=""; [ "${enforce}" = true ] || prefix="[dry-run, would FAIL] "
    echo "  ${prefix}DISPATCH EXCEEDED: ${arc_dyn} Arc<dyn> sites, ${over} over the effective ceiling of ${effective_arc_ceiling}."
    echo "    Prefer concrete types over Arc<dyn> for single-impl seams (concrete-by-default)."
    echo "    If a new dyn boundary is genuinely justified (a real second impl / test-fake seam),"
    echo "    raise arc_dyn_ceiling in ${BUDGET_FILE} with a PR rationale."
    breached=1
elif [ "$((arc_dyn_ceiling - arc_dyn))" -gt 40 ]; then
    echo "  NUDGE: dispatch is $((arc_dyn_ceiling - arc_dyn)) below ceiling — lower arc_dyn_ceiling to lock it in."
fi

echo ""
if [ "${breached}" -eq 1 ]; then
    [ "${enforce}" = true ] && exit 1 || { echo "DRY-RUN: would fail, not enforcing."; exit 0; }
fi
echo "OK: composition within mass + dispatch budget."
exit 0
