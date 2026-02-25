#!/usr/bin/env bash
# Quality gate — run before shipping changes.
# See: https://github.com/nearai/ironclaw/issues/338
#
# Usage:
#   ./scripts/quality-gate.sh          # full suite
#   ./scripts/quality-gate.sh --quick  # skip coverage and mutation testing
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

QUICK=false
[[ "${1:-}" == "--quick" ]] && QUICK=true

passed=0
failed=0

step() {
    printf "\n${YELLOW}--- %s ---${NC}\n" "$1"
}

pass() {
    printf "${GREEN}PASS${NC}: %s\n" "$1"
    ((passed++))
}

fail() {
    printf "${RED}FAIL${NC}: %s\n" "$1"
    ((failed++))
}

# 1. Format check
step "cargo fmt"
if cargo fmt --all -- --check 2>/dev/null; then
    pass "formatting"
else
    fail "formatting — run 'cargo fmt'"
fi

# 2. Clippy (includes cognitive_complexity + too_many_lines via clippy.toml)
step "cargo clippy"
if cargo clippy --all --benches --tests --examples --all-features -- -D warnings 2>&1; then
    pass "clippy (zero warnings)"
else
    fail "clippy — fix all warnings"
fi

# 3. Tests
step "cargo test"
if cargo test --all-features 2>&1; then
    pass "tests"
else
    fail "tests"
fi

# 4. Mutex unwrap check (no .lock().unwrap() in production code)
step "mutex unwrap check"
# Scan each .rs file, only checking lines BEFORE '#[cfg(test)]' boundary.
# This excludes test modules at the bottom of files. recording.rs is test-only.
MUTEX_UNWRAPS=""
while IFS= read -r rs_file; do
    # Find the line number where #[cfg(test)] starts (0 if absent)
    TEST_LINE=$(grep -n '^#\[cfg(test)\]' "$rs_file" | head -1 | cut -d: -f1)
    TEST_LINE=${TEST_LINE:-99999}
    # Check production portion for .lock().unwrap() (single-line and multi-line)
    HITS=$(head -n "$((TEST_LINE - 1))" "$rs_file" 2>/dev/null \
        | grep -n '\.lock()\.unwrap()' || true)
    if [[ -n "$HITS" ]]; then
        while IFS= read -r hit; do
            MUTEX_UNWRAPS+="${rs_file}:${hit}"$'\n'
        done <<< "$HITS"
    fi
done < <(find src/ -name '*.rs' ! -name 'recording.rs')
MUTEX_UNWRAPS=$(echo "$MUTEX_UNWRAPS" | sed '/^$/d')
if [[ -z "$MUTEX_UNWRAPS" ]]; then
    pass "no .lock().unwrap() in production code"
else
    echo "$MUTEX_UNWRAPS"
    fail "found .lock().unwrap() in production code — use if let Ok()"
fi

# 5. Feature isolation
step "feature isolation"
if cargo check --no-default-features --features libsql 2>&1; then
    pass "feature isolation: libsql-only"
else
    fail "feature isolation: libsql-only"
fi
if cargo check --no-default-features --features "libsql,otel" 2>&1; then
    pass "feature isolation: libsql+otel"
else
    fail "feature isolation: libsql+otel"
fi

# 6. OTEL E2E test (requires Docker)
step "OTEL E2E (Jaeger)"
COMPOSE_FILE="docker-compose.otel-test.yml"
if command -v docker &>/dev/null && docker info &>/dev/null 2>&1; then
    # Ensure cleanup on exit (container leak prevention)
    cleanup_jaeger() { docker compose -f "$COMPOSE_FILE" down 2>/dev/null || true; }
    trap cleanup_jaeger EXIT
    docker compose -f "$COMPOSE_FILE" up -d 2>&1
    # Poll for Jaeger readiness instead of blind sleep
    for i in $(seq 1 30); do
        if curl -sf http://localhost:16686/ >/dev/null 2>&1; then break; fi
        sleep 1
    done
    if cargo test --features otel --test otel_e2e -- --ignored 2>&1; then
        pass "OTEL E2E — spans arrive in Jaeger"
    else
        fail "OTEL E2E — test failed (check Jaeger logs)"
    fi
    cleanup_jaeger
    trap - EXIT
else
    printf "${YELLOW}SKIP${NC}: Docker not available — skipping OTEL E2E test\n"
fi

# 7. Lizard (cyclomatic complexity + copy-paste detection)
step "lizard"
if command -v python3 &>/dev/null && python3 -m lizard --version &>/dev/null 2>&1; then
    LIZARD_WARNINGS=$(python3 -m lizard src/ -l rust -w -T cyclomatic_complexity=15 -T length=100 2>&1 | grep -c "^.*warning:" || true)
    if [[ "$LIZARD_WARNINGS" -eq 0 ]]; then
        pass "lizard (zero warnings)"
    else
        fail "lizard — $LIZARD_WARNINGS functions exceed complexity/length thresholds"
    fi

    # Duplicate detection
    DUPES=$(python3 -m lizard src/ -l rust -Eduplicate 2>&1 | grep -c "^Duplicate" || true)
    if [[ "$DUPES" -eq 0 ]]; then
        pass "lizard duplicate detection"
    else
        printf "${YELLOW}INFO${NC}: lizard found %s potential duplicate blocks\n" "$DUPES"
    fi
else
    printf "${YELLOW}SKIP${NC}: lizard not installed (pip3 install lizard)\n"
fi

if [[ "$QUICK" == true ]]; then
    printf "\n${YELLOW}Skipping coverage and mutation testing (--quick mode)${NC}\n"
else
    # 8. Test coverage
    step "cargo llvm-cov"
    if command -v cargo-llvm-cov &>/dev/null; then
        COV_OUTPUT=$(cargo llvm-cov --text --skip-functions 2>&1)
        COV_LINE=$(echo "$COV_OUTPUT" | grep "^TOTAL" || true)
        if [[ -n "$COV_LINE" ]]; then
            COV_PCT=$(echo "$COV_LINE" | awk '{print $NF}' | tr -d '%')
            printf "Line coverage: %s%%\n" "$COV_PCT"
            # Global floor — ratchet up as old code gets tests.
            # Current baseline: 49.36% (2026-02-23)
            THRESHOLD=50
            if (( $(echo "$COV_PCT >= $THRESHOLD" | bc -l) )); then
                pass "coverage >= ${THRESHOLD}%"
            else
                fail "coverage ${COV_PCT}% < ${THRESHOLD}% threshold"
            fi
        else
            printf "${YELLOW}SKIP${NC}: could not parse coverage output\n"
        fi
    else
        printf "${YELLOW}SKIP${NC}: cargo-llvm-cov not installed\n"
    fi

    # 9. Mutation testing (targeted — full run is too slow for a gate)
    step "cargo mutants (sampled)"
    if command -v cargo-mutants &>/dev/null; then
        # Run on a small sample to keep the gate fast
        MUTANT_OUTPUT=$(cargo mutants --timeout 120 --jobs 4 --shard 0/10 2>&1 || true)
        SURVIVED=$(echo "$MUTANT_OUTPUT" | grep -c "SURVIVED" || true)
        TESTED=$(echo "$MUTANT_OUTPUT" | grep -c "CAUGHT\|SURVIVED\|TIMEOUT" || true)
        if [[ "$TESTED" -gt 0 ]]; then
            printf "Mutants tested: %s, survived: %s\n" "$TESTED" "$SURVIVED"
            pass "mutation testing ran (${SURVIVED} survived out of ${TESTED})"
        else
            printf "${YELLOW}SKIP${NC}: no mutants tested in this shard\n"
        fi
    else
        printf "${YELLOW}SKIP${NC}: cargo-mutants not installed\n"
    fi
fi

# Summary
printf "\n${YELLOW}=== Summary ===${NC}\n"
printf "${GREEN}Passed${NC}: %s\n" "$passed"
if [[ "$failed" -gt 0 ]]; then
    printf "${RED}Failed${NC}: %s\n" "$failed"
    exit 1
else
    printf "All checks passed.\n"
fi
