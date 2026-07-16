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

# --- count LOC of *.rs under a directory (0 if it does not exist) ------------
count_loc() {
    local dir="$1"
    [ -d "${dir}" ] || { echo 0; return; }
    find "${dir}" -name '*.rs' -type f -print0 2>/dev/null \
        | xargs -0 cat 2>/dev/null | wc -l | tr -d ' '
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

# --- parse one scalar key from the [gate] TOML table -------------------------
# Values are simple: integers, true/false, or "quoted strings". No nested tables.
toml_get() {
    local key="$1"
    grep -E "^[[:space:]]*${key}[[:space:]]*=" "${BUDGET_FILE}" \
        | head -1 \
        | sed -E "s/^[[:space:]]*${key}[[:space:]]*=[[:space:]]*//; s/[[:space:]]*(#.*)?$//; s/^\"//; s/\"$//"
}

fail_schema() { echo "composition-budget: $1" >&2; exit 1; }

[ -f "${BUDGET_FILE}" ] || fail_schema "budget file not found: ${BUDGET_FILE}"

enforce="$(toml_get enforce)"
ceiling_bp="$(toml_get ceiling_bp)"
tolerance_bp="$(toml_get tolerance_bp)"

# Schema validation — manifest bugs always exit 1, regardless of enforce.
case "${enforce}" in
    true|false) ;;
    *) fail_schema "[gate].enforce must be true or false, got '${enforce:-<missing>}'" ;;
esac
[[ "${ceiling_bp}"   =~ ^[0-9]+$ ]] || fail_schema "[gate].ceiling_bp must be an integer, got '${ceiling_bp:-<missing>}'"
[[ "${tolerance_bp}" =~ ^[0-9]+$ ]] || fail_schema "[gate].tolerance_bp must be an integer, got '${tolerance_bp:-<missing>}'"

comp_loc="$(count_loc "${COMPOSITION_SRC}")"
den_loc="$(count_denominator)"

[ "${den_loc}" -gt 0 ] || fail_schema "denominator LOC is 0 — no crates/*/src trees found under '${CRATES_ROOT}'"

# observed basis points, rounded to nearest (integer math via awk)
observed_bp="$(awk -v c="${comp_loc}" -v d="${den_loc}" 'BEGIN { printf "%d", (10000*c/d)+0.5 }')"

fmt_pct() { awk -v bp="$1" 'BEGIN { printf "%.2f", bp/100 }'; }

if [ "${print_only}" = true ]; then
    echo "composition share: $(fmt_pct "${observed_bp}")% (${observed_bp} bp) — ${comp_loc} / ${den_loc} LOC"
    exit 0
fi

effective_ceiling=$((ceiling_bp + tolerance_bp))

echo "Composition budget gate: $([ "${enforce}" = true ] && echo ENFORCING || echo DRY-RUN)"
echo "  composition src : ${comp_loc} LOC"
echo "  all crates src  : ${den_loc} LOC"
echo "  observed share  : $(fmt_pct "${observed_bp}")% (${observed_bp} bp)"
echo "  ceiling         : $(fmt_pct "${ceiling_bp}")% (tolerance $(fmt_pct "${tolerance_bp}")pp -> effective ceiling $(fmt_pct "${effective_ceiling}")% / ${effective_ceiling} bp)"

if [ "${observed_bp}" -gt "${effective_ceiling}" ]; then
    over=$((observed_bp - effective_ceiling))
    prefix=""
    [ "${enforce}" = true ] || prefix="[dry-run, would FAIL] "
    echo ""
    echo "${prefix}BUDGET EXCEEDED: composition is $(fmt_pct "${observed_bp}")% of production crate code," \
         "$(fmt_pct "${over}")pp over the effective ceiling of $(fmt_pct "${effective_ceiling}")%."
    echo "  Move behavior OUT of ironclaw_reborn_composition into an owning crate — the crate's"
    echo "  charter is assembly-only (build_*/with_* wiring). See:"
    echo "    .claude/skills/ironclaw-reborn-architecture-review  (checklist item 2)"
    echo "  Biggest behavior subtrees to carve first: src/slack, src/product_auth, src/extension_host."
    echo "  If this growth is genuinely justified, raise ceiling_bp in ${BUDGET_FILE}"
    echo "  and state the reason in the PR description (a reviewed, one-directional decision)."
    [ "${enforce}" = true ] && exit 1 || exit 0
fi

headroom=$((effective_ceiling - observed_bp))
echo ""
echo "OK: composition share within budget (headroom $(fmt_pct "${headroom}")pp / ${headroom} bp)."

# Down-ratchet nudge: >1pp of accumulated slack means the ceiling should follow
# the improvement down so it can't silently drift back up.
if [ "$((ceiling_bp - observed_bp))" -gt 100 ]; then
    slack=$((ceiling_bp - observed_bp))
    echo "NUDGE: composition is now $(fmt_pct "${slack}")pp below the ceiling — lower ceiling_bp in"
    echo "       ${BUDGET_FILE} to lock in the carve-out and keep the ratchet tight."
fi

exit 0
