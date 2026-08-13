#!/usr/bin/env bash
#
# Composition mass ratchet gate.
#
# Fails when ironclaw_composition breaches any of the three committed
# bounds in scripts/ci/composition-budget.toml:
#   [mass]     its SHARE of production crate code            (relative)
#   [abs]      its ABSOLUTE production LOC                   (relative to nothing)
#   [dispatch] its Arc<dyn> site count in governed prod code
# See that file's header for each metric's rationale and definition. The
# absolute bound is the binding one: the share metric's denominator is every
# other crate's production code, so feature inflow elsewhere improves
# composition's score while composition itself grows (#7151).
#
# Usage:
#   scripts/ci/check-composition-budget.sh          # run the gate
#   scripts/ci/check-composition-budget.sh --print   # print observed share only, never fail
#
# Both the numerator and the denominator are resolved from the crate inventory
# (scripts/ci/lib/crate_tree.py — the outermost owner of each crates/**/Cargo.toml),
# not from the flat `crates/ironclaw_*` shape. The numerator crate is found by
# NAME at whatever depth its Cargo.toml sits, so the target-architecture family
# move (crates/<family>/ironclaw_*, PROPOSAL §5) does not take this ratchet dark.
# Both failure directions are gated below: an empty denominator and a zero-LOC
# numerator are errors, never a 0.00% "pass" (docs/internal/reborn/target-architecture/
# CHECKLIST.md WS10, #6963).
#
# Test/override env vars (used by test-check-composition-budget.sh; unset in prod):
#   COMPOSITION_SRC   numerator dir      (default: discovered from COMPOSITION_CRATE)
#   COMPOSITION_CRATE numerator crate    (default: ironclaw_composition)
#   CRATES_ROOT       crates directory   (default: crates) -> its parent is the
#                     discovery root; the basename must be `crates`
#   BUDGET_FILE       budget TOML path   (default: scripts/ci/composition-budget.toml)
#
# Exit codes: 0 = within budget (or dry-run) ; 1 = breach (enforcing) or schema error.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

CRATES_ROOT="${CRATES_ROOT:-crates}"
COMPOSITION_CRATE="${COMPOSITION_CRATE:-ironclaw_composition}"
BUDGET_FILE="${BUDGET_FILE:-scripts/ci/composition-budget.toml}"

print_only=false
[ "${1:-}" = "--print" ] && print_only=true

fail_schema() { echo "composition-budget: $1" >&2; exit 1; }

# --- crate inventory (tree-shape-agnostic, fail-closed) ----------------------
# crate_tree.py emits crate directories relative to a REPO root, so the
# discovery root is CRATES_ROOT's parent. Requiring the basename to be `crates`
# keeps that derivation honest instead of silently discovering the wrong tree.
if [ "$(basename "${CRATES_ROOT}")" != "crates" ]; then
    fail_schema "CRATES_ROOT must be a directory named 'crates', got '${CRATES_ROOT}'"
fi
discovery_root="$(dirname "${CRATES_ROOT}")"

# crate_tree.py exits non-zero (and explains itself) on a missing crates/ tree
# or an inventory below its floor. Propagate that rather than measuring nothing.
# stdout and stderr are captured separately: merging them with `2>&1` would fold
# any Python warning into the inventory and turn a benign diagnostic into a
# bogus crate directory in the denominator.
discovery_error="$(mktemp)"
trap 'rm -f "${discovery_error}"' EXIT
if ! CRATE_DIRS="$(python3 "${repo_root}/scripts/ci/lib/crate_tree.py" "${discovery_root}" 2>"${discovery_error}")"; then
    fail_schema "crate discovery failed under '${discovery_root}': $(cat "${discovery_error}")"
fi
[ -n "${CRATE_DIRS}" ] || fail_schema "crate inventory is empty under '${discovery_root}'"

# --- resolve the numerator crate BY NAME, at any depth -----------------------
if [ -z "${COMPOSITION_SRC:-}" ]; then
    composition_matches="$(printf '%s\n' "${CRATE_DIRS}" \
        | awk -F/ -v want="${COMPOSITION_CRATE}" '$NF == want')"
    composition_count="$(printf '%s' "${composition_matches}" | grep -c . || true)"
    if [ "${composition_count}" -ne 1 ]; then
        fail_schema "expected exactly one crate directory named '${COMPOSITION_CRATE}' under \
'${CRATES_ROOT}', found ${composition_count}. If the crate was renamed or moved, repoint this \
gate (COMPOSITION_CRATE) in the same PR rather than letting the ratchet measure an empty tree."
    fi
    COMPOSITION_SRC="${discovery_root}/${composition_matches}/src"
fi

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

# --- sum LOC of every discovered crate's src tree (the denominator) ----------
# Keyed to the crate inventory rather than to `crates/*/src`, so a crate keeps
# counting after it moves into a family directory. Equivalent on the flat tree:
# every direct child of crates/ owns a Cargo.toml, and nested manifests
# (ironclaw_safety/fuzz, the assets/*/wasm-src guests) are attributed to their
# enclosing crate by the inventory exactly as `crates/*/src` skipped them.
count_denominator() {
    local total=0 rel loc
    while IFS= read -r rel; do
        [ -n "${rel}" ] || continue
        loc="$(count_loc "${discovery_root}/${rel}/src")"
        total=$((total + loc))
    done <<EOF
${CRATE_DIRS}
EOF
    echo "${total}"
}

# Dispatch (Arc<dyn>) sub-metric is scoped to composition PRODUCTION files, but
# EXCLUDES slack/, telegram/ and extension_host/ — those subtrees are owned by
# the separate channel/extension refactor, so this gate must not trip on or
# govern their work. (telegram/ holds only the thin telegram_host_beta wiring
# layer; the host domain itself lives in crates/extensions/packages/telegram.)
DISPATCH_EXCLUDE_RE='/(slack|telegram|extension_host)/'

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

[ -f "${BUDGET_FILE}" ] || fail_schema "budget file not found: ${BUDGET_FILE}"

enforce="$(toml_get enforce)"
ceiling_bp="$(toml_get ceiling_bp)"
tolerance_bp="$(toml_get tolerance_bp)"
arc_dyn_ceiling="$(toml_get arc_dyn_ceiling)"
arc_dyn_tolerance="$(toml_get arc_dyn_tolerance)"
loc_ceiling="$(toml_get loc_ceiling)"
loc_tolerance="$(toml_get loc_tolerance)"
loc_nudge_slack="$(toml_get loc_nudge_slack)"

# Schema validation — manifest bugs always exit 1, regardless of enforce.
case "${enforce}" in
    true|false) ;;
    *) fail_schema "[gate].enforce must be true or false, got '${enforce:-<missing>}'" ;;
esac
[[ "${ceiling_bp}"      =~ ^[0-9]+$ ]] || fail_schema "[gate].ceiling_bp must be an integer, got '${ceiling_bp:-<missing>}'"
[[ "${tolerance_bp}"    =~ ^[0-9]+$ ]] || fail_schema "[gate].tolerance_bp must be an integer, got '${tolerance_bp:-<missing>}'"
[[ "${arc_dyn_ceiling}"   =~ ^[0-9]+$ ]] || fail_schema "[gate].arc_dyn_ceiling must be an integer, got '${arc_dyn_ceiling:-<missing>}'"
[[ "${arc_dyn_tolerance}" =~ ^[0-9]+$ ]] || fail_schema "[gate].arc_dyn_tolerance must be an integer, got '${arc_dyn_tolerance:-<missing>}'"
# The absolute-mass keys are REQUIRED, not optional-with-a-default: a default
# would let the binding metric be disarmed by deleting three lines, which is the
# quiet way a gate joins the ones that check nothing (#7151).
[[ "${loc_ceiling}"      =~ ^[0-9]+$ ]] || fail_schema "[gate].loc_ceiling must be an integer, got '${loc_ceiling:-<missing>}'"
[[ "${loc_tolerance}"    =~ ^[0-9]+$ ]] || fail_schema "[gate].loc_tolerance must be an integer, got '${loc_tolerance:-<missing>}'"
[[ "${loc_nudge_slack}"  =~ ^[0-9]+$ ]] || fail_schema "[gate].loc_nudge_slack must be an integer, got '${loc_nudge_slack:-<missing>}'"
# A zero ceiling is unreachable by real code and would make the gate fire on
# every commit; a zero-or-absurd ceiling is far likelier to be a typo or a
# stubbed-out disarm than a genuine bound.
[ "${loc_ceiling}" -gt 0 ] || fail_schema "[gate].loc_ceiling must be greater than 0 — a zero absolute ceiling is a disarmed gate, not a bound"

comp_loc="$(count_loc "${COMPOSITION_SRC}")"
den_loc="$(count_denominator)"

[ "${den_loc}" -gt 0 ] || fail_schema "denominator LOC is 0 — no crate src trees found under '${CRATES_ROOT}'"

# A zero numerator is the silent half of this gate's failure mode, and it is the
# one a partial family move actually produces: move only the composition crate
# and the denominator stays healthy, so the gate used to print
# "0.00% (0 bp) ... OK: composition within mass + dispatch budget" and exit 0 —
# a ratchet reporting perfect health while measuring nothing. Discovery makes
# that unreachable by construction; this guard is the backstop that also covers
# an explicit COMPOSITION_SRC override pointing somewhere empty.
[ "${comp_loc}" -gt 0 ] || fail_schema "composition LOC is 0 — '${COMPOSITION_SRC}' holds no \
production .rs files. The ratchet cannot pass by measuring nothing; repoint COMPOSITION_CRATE \
(or COMPOSITION_SRC) in the same PR that moved or renamed the crate."

# observed basis points, rounded to nearest (integer math via awk)
observed_bp="$(awk -v c="${comp_loc}" -v d="${den_loc}" 'BEGIN { printf "%d", (10000*c/d)+0.5 }')"

fmt_pct() { awk -v bp="$1" 'BEGIN { printf "%.2f", bp/100 }'; }

arc_dyn="$(count_arc_dyn)"

if [ "${print_only}" = true ]; then
    echo "composition share: $(fmt_pct "${observed_bp}")% (${observed_bp} bp) — ${comp_loc} / ${den_loc} LOC"
    echo "composition absolute: ${comp_loc} LOC (production src)"
    echo "composition dispatch: ${arc_dyn} Arc<dyn> (governed prod, excl slack/extension_host)"
    exit 0
fi

effective_ceiling=$((ceiling_bp + tolerance_bp))
effective_arc_ceiling=$((arc_dyn_ceiling + arc_dyn_tolerance))
effective_loc_ceiling=$((loc_ceiling + loc_tolerance))
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
    echo "    Move behavior OUT of ironclaw_composition into an owning crate (charter is"
    echo "    assembly-only). See .claude/skills/ironclaw-reborn-architecture-review (item 2)."
    echo "    If justified, raise ceiling_bp in ${BUDGET_FILE} with a PR rationale."
    breached=1
elif [ "$((ceiling_bp - observed_bp))" -gt 100 ]; then
    echo "  NUDGE: mass is $(fmt_pct "$((ceiling_bp - observed_bp))")pp below ceiling — lower ceiling_bp to lock it in."
fi

# ---- Metric 1b: ABSOLUTE mass (production LOC, no denominator) ----
# The binding bound. Metric 1's denominator is every other crate's production
# code, so feature inflow elsewhere improves composition's share while
# composition itself grows; this one cannot be moved by anyone else's work.
echo "  [abs] composition src  : ${comp_loc} LOC"
echo "         ceiling         : ${loc_ceiling} (tol ${loc_tolerance} -> effective ${effective_loc_ceiling})"
if [ "${comp_loc}" -gt "${effective_loc_ceiling}" ]; then
    over=$((comp_loc - effective_loc_ceiling))
    prefix=""; [ "${enforce}" = true ] || prefix="[dry-run, would FAIL] "
    echo "  ${prefix}ABSOLUTE MASS EXCEEDED: composition holds ${comp_loc} production LOC, ${over} over the"
    echo "    effective ceiling of ${effective_loc_ceiling}. Composition's charter is service-graph ASSEMBLY;"
    echo "    behavior belongs in an owning crate. Note the share metric above may still look"
    echo "    healthy — it improves whenever the rest of the workspace grows, which is exactly"
    echo "    why this absolute bound exists (#7151)."
    echo "    If the growth is justified, raise loc_ceiling in ${BUDGET_FILE} with a PR rationale."
    breached=1
elif [ "$((loc_ceiling - comp_loc))" -gt "${loc_nudge_slack}" ]; then
    echo "  NUDGE: absolute mass is $((loc_ceiling - comp_loc)) LOC below ceiling — lower loc_ceiling to lock it in (re-ratchet at every wave close)."
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
