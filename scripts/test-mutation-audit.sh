#!/usr/bin/env bash
# Self-tests for the mutation-audit harness.
#
# Guardrails are code: a checker that silently does nothing is worse than no
# checker, because it reads as a clean bill of health. These cases pin the
# failure modes that actually bit during development — each one produced a
# wrong, confident answer before it was fixed.
#
# Fast and hermetic: no cargo, no compilation. The expensive end-to-end
# behaviour (a mutant flipping MISSED -> caught) is proven by running
# scripts/mutation-verify-fix.sh against a real crate, not here.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
audit="$repo_root/scripts/mutation-audit.sh"
verify="$repo_root/scripts/mutation-verify-fix.sh"
queue="$repo_root/scripts/ci/mutation_triage_queue.py"

work="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-mutation-selftest.XXXXXX")"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT

failures=0
check() {
  local label="$1"
  shift
  if "$@"; then
    echo "  ok   $label"
  else
    echo "  FAIL $label"
    failures=$((failures + 1))
  fi
}

echo "▶ A. an unscoped audit is refused, not silently empty"
# Without --package, cargo-mutants scopes to the workspace-root package, which
# has no lib/bin, and reports zero mutants. That reads as "nothing to fix".
check "audit without --package exits non-zero" \
  bash -c "! '$audit' 2>/dev/null"
check "audit without --package explains why" \
  bash -c "'$audit' 2>&1 | grep -q 'silently finds zero mutants'"

echo "▶ A2. usage guards work with cargo-mutants absent from PATH"
# Regression: the cargo-mutants presence check originally ran *before* argument
# validation, so on a machine without the tool the unscoped-run guard was
# unreachable and reported the wrong error. These cases passed anyway because
# the author had cargo-mutants installed — the self-test depended on the
# developer's environment, which is exactly what "hermetic" is supposed to rule
# out. A stub PATH with no cargo at all reproduces a clean machine.
bare_path="$work/bare-bin"
mkdir -p "$bare_path"
for tool in bash sed grep python3 mktemp rm dirname cd; do
  src="$(command -v "$tool" 2>/dev/null || true)"
  [ -n "$src" ] && ln -sf "$src" "$bare_path/$tool"
done
check "audit still reports the usage error, not a missing tool" \
  bash -c "PATH='$bare_path' '$audit' 2>&1 | grep -q 'silently finds zero mutants'"
check "verify still reports the usage error, not a missing tool" \
  bash -c "PATH='$bare_path' '$verify' 2>&1 | grep -q 'usage:'"

echo "▶ B. the verify gate refuses incomplete invocations"
check "verify with no args exits non-zero" \
  bash -c "! '$verify' 2>/dev/null"
check "verify without --package exits non-zero" \
  bash -c "! '$verify' 'crates/x/src/y.rs:1:1: replace f with ()' 2>/dev/null"
check "verify rejects a mutant naming a nonexistent file" \
  bash -c "! '$verify' -p some_pkg 'crates/nope/src/gone.rs:1:1: replace f with ()' 2>/dev/null"

echo "▶ C. both scripts refuse to inherit a shared CARGO_TARGET_DIR"
# This one is load-bearing: a shared target dir made the gate report a killed
# mutant as surviving, on identical source. Wrong in both directions.
check "audit warns and unsets CARGO_TARGET_DIR" \
  bash -c "CARGO_TARGET_DIR=/tmp/shared-target '$audit' 2>&1 | grep -q 'ignoring CARGO_TARGET_DIR'"
check "verify warns and unsets CARGO_TARGET_DIR" \
  bash -c "CARGO_TARGET_DIR=/tmp/shared-target '$verify' 2>&1 | grep -q 'ignoring CARGO_TARGET_DIR'"

echo "▶ D. the triage queue reports survivors and scores over viable mutants"
report="$work/report"
mkdir -p "$report"
cat >"$report/missed.txt" <<'EOF'
crates/demo/src/lib.rs:12:5: replace add with ()
EOF
cat >"$report/caught.txt" <<'EOF'
crates/demo/src/lib.rs:20:5: replace sub with ()
crates/demo/src/lib.rs:24:5: replace mul with ()
EOF
cat >"$report/unviable.txt" <<'EOF'
crates/demo/src/lib.rs:30:5: replace thing -> Self with Default::default()
crates/demo/src/lib.rs:34:5: replace other -> Self with Default::default()
crates/demo/src/lib.rs:38:5: replace more -> Self with Default::default()
EOF

python3 "$queue" --report-dir "$report" --output "$work/queue.md" >/dev/null

check "queue counts survivors" \
  grep -q '\*\*1 survivors\*\*' "$work/queue.md"
# The headline number must exclude unviable mutants: they failed to compile and
# say nothing about test strength. Scoring 2/6 instead of 2/3 would understate
# the suite and invite someone to 'fix' non-problems.
check "queue scores over viable mutants only (2/3, not 2/6)" \
  grep -q '\*\*2/3\*\*' "$work/queue.md"
check "queue lists the surviving mutant verbatim" \
  grep -q 'replace add with ()' "$work/queue.md"
check "queue offers the needs-product-decision verdict" \
  grep -q 'needs-product-decision' "$work/queue.md"
check "queue seeds every entry with an unset verdict" \
  grep -q 'verdict: .TODO.' "$work/queue.md"

echo "▶ E. an empty survivor list is reported as clean, not as an error"
empty="$work/empty"
mkdir -p "$empty"
: >"$empty/missed.txt"
: >"$empty/caught.txt"
python3 "$queue" --report-dir "$empty" --output "$work/empty.md" >/dev/null
check "queue states there is nothing to triage" \
  grep -q 'No surviving mutants' "$work/empty.md"

echo "▶ F. a missing report is a loud error, not an empty queue"
check "queue fails when missed.txt is absent" \
  bash -c "! python3 '$queue' --report-dir '$work/absent' --output '$work/x.md' 2>/dev/null"

echo "▶ G. the audit hands the generator the directory cargo-mutants wrote to"
# Sections D-F drive the generator directly with hand-built fixtures, so they
# all passed while every real audit failed at its final step: cargo-mutants
# creates mutants.out *inside* --output, and the script was reading $MUT_OUT
# itself. Testing the helper proved nothing about the caller wiring it — the
# gap .claude/rules/testing.md calls "test through the caller".
#
# A stub cargo reproduces that layout and cargo-mutants' non-recursive creation
# of --output without compiling anything.
stub_bin="$work/stub-bin"
mkdir -p "$stub_bin"
for tool in bash sed grep python3 mktemp rm dirname cat mkdir; do
  src="$(command -v "$tool" 2>/dev/null || true)"
  [ -n "$src" ] && ln -sf "$src" "$stub_bin/$tool"
done
: >"$stub_bin/cargo-mutants"
chmod +x "$stub_bin/cargo-mutants"
cat >"$stub_bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail

# Mimic the behaviours under test: cargo-mutants creates --output with a
# single-level mkdir, then creates mutants.out inside it.
out="."
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -d "$out" ] || mkdir "$out"
mkdir "$out/mutants.out"
echo 'crates/demo/src/lib.rs:12:5: replace add with ()' >"$out/mutants.out/missed.txt"
: >"$out/mutants.out/caught.txt"
STUB
chmod +x "$stub_bin/cargo"

audit_out="$work/missing-parent/audit-out"
check "audit completes and writes the queue where cargo-mutants wrote results" \
  bash -c "PATH='$stub_bin' MUT_OUT='$audit_out' '$audit' -p demo >/dev/null 2>&1 \
    && [ -f '$audit_out/mutants.out/triage-queue.md' ]"
check "the generated queue carries the survivor, not an empty shell" \
  grep -q 'replace add with ()' "$audit_out/mutants.out/triage-queue.md"

echo "▶ H. an audit that tests nothing is an error, not a pass"
# Found in the field: crates/ironclaw_reborn_composition/src/extension_host/
# channel_outbound_targets.rs moved to crates/ironclaw_extension_host/ in #6669.
# The old command line still ran, cargo-mutants filtered to zero mutants, and
# the audit exited 0 with an empty queue — a clean bill of health for a file it
# never opened. cargo-mutants only WARNs in that case, so the script must judge.
check "audit rejects a file path that does not exist" \
  bash -c "! PATH='$stub_bin' MUT_OUT='$work/stale' '$audit' -p demo crates/gone/src/nope.rs 2>/dev/null"
check "audit says the path may have moved" \
  bash -c "PATH='$stub_bin' MUT_OUT='$work/stale' '$audit' -p demo crates/gone/src/nope.rs 2>&1 | grep -q 'may have moved'"

empty_bin="$work/empty-bin"
mkdir -p "$empty_bin"
for tool in bash sed grep python3 mktemp rm dirname cat mkdir; do
  src="$(command -v "$tool" 2>/dev/null || true)"
  [ -n "$src" ] && ln -sf "$src" "$empty_bin/$tool"
done
: >"$empty_bin/cargo-mutants"
chmod +x "$empty_bin/cargo-mutants"
cat >"$empty_bin/cargo" <<'STUB'
#!/usr/bin/env bash
# A run that generated no mutants at all: report files exist but are empty.
out="."
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
mkdir -p "$out/mutants.out"
: >"$out/mutants.out/missed.txt"
: >"$out/mutants.out/caught.txt"
STUB
chmod +x "$empty_bin/cargo"

check "audit rejects a run that produced zero mutants" \
  bash -c "! PATH='$empty_bin' MUT_OUT='$work/zero' '$audit' -p demo 2>/dev/null"
check "audit explains that an empty result is not a passing audit" \
  bash -c "PATH='$empty_bin' MUT_OUT='$work/zero' '$audit' -p demo 2>&1 | grep -q 'not a passing audit'"

echo
if [ "$failures" -eq 0 ]; then
  echo "all mutation-harness self-tests passed"
else
  echo "$failures self-test(s) failed" >&2
  exit 1
fi
