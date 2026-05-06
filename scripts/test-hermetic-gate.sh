#!/usr/bin/env bash
# Regression tests for the Hermes-style hermetic local gate.
#
# These tests intentionally exercise the gate in dry-run mode so they verify
# command wiring and environment isolation without compiling the workspace.

set -euo pipefail
cd "$(dirname "$0")/.."

PASS=0
FAIL=0

assert_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -Fq -- "$needle" <<<"$haystack"; then
        echo "OK: $label"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — missing: $needle"
        FAIL=$((FAIL + 1))
    fi
}

assert_not_contains() {
    local label="$1" haystack="$2" needle="$3"
    if grep -Fq -- "$needle" <<<"$haystack"; then
        echo "FAIL: $label — unexpectedly found: $needle"
        FAIL=$((FAIL + 1))
    else
        echo "OK: $label"
        PASS=$((PASS + 1))
    fi
}

assert_line() {
    local label="$1" haystack="$2" expected="$3"
    if grep -Fxq -- "$expected" <<<"$haystack"; then
        echo "OK: $label"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $label — missing exact line: $expected"
        FAIL=$((FAIL + 1))
    fi
}

if [ ! -x scripts/ci/hermetic_gate.sh ]; then
    echo "FAIL: scripts/ci/hermetic_gate.sh exists and is executable"
    exit 1
fi

run_default=$(
    OPENAI_API_KEY="secret-openai" \
    ANTHROPIC_API_KEY="secret-anthropic" \
    DATABASE_URL="postgres://user:password@example.invalid/db" \
    IRONCLAW_BASE_DIR="/real/home/that/must/not/leak" \
    IRONCLAW_HERMETIC_DRY_RUN=1 \
    scripts/ci/hermetic_gate.sh
)

assert_contains "dry-run announces hermetic environment" "$run_default" "==> hermetic environment"
assert_line "provider keys are scrubbed and dotenv-blocked" "$run_default" "OPENAI_API_KEY="
assert_line "anthropic key is scrubbed and dotenv-blocked" "$run_default" "ANTHROPIC_API_KEY="
assert_line "database url is scrubbed and dotenv-blocked" "$run_default" "DATABASE_URL="
assert_contains "uses libsql backend" "$run_default" "DATABASE_BACKEND=libsql"
assert_contains "uses isolated ironclaw base dir" "$run_default" "IRONCLAW_BASE_DIR="
assert_not_contains "real home does not leak" "$run_default" "/real/home/that/must/not/leak"
assert_not_contains "secrets do not leak" "$run_default" "secret-openai"

ENV_BACKUP=""
ENV_EXISTED=0
if [ -e .env ]; then
    ENV_BACKUP=$(mktemp "${TMPDIR:-/tmp}/ironclaw-env-backup.XXXXXX")
    cp .env "$ENV_BACKUP"
    ENV_EXISTED=1
fi
cleanup_env_fixture() {
    if [ "$ENV_EXISTED" -eq 1 ]; then
        cp "$ENV_BACKUP" .env
        rm -f "$ENV_BACKUP"
    else
        rm -f .env
    fi
}
trap cleanup_env_fixture EXIT
cat > .env <<'EOF_ENV'
OPENAI_API_KEY=dotenv-openai-secret
ANTHROPIC_API_KEY=dotenv-anthropic-secret
DATABASE_URL=postgres://dotenv:secret@example.invalid/db
EOF_ENV
run_dotenv=$(
    IRONCLAW_HERMETIC_DRY_RUN=1 \
    scripts/ci/hermetic_gate.sh
)
assert_line "dotenv openai is masked empty" "$run_dotenv" "OPENAI_API_KEY="
assert_line "dotenv anthropic is masked empty" "$run_dotenv" "ANTHROPIC_API_KEY="
assert_line "dotenv database url is masked empty" "$run_dotenv" "DATABASE_URL="
assert_not_contains "dotenv openai secret does not leak" "$run_dotenv" "dotenv-openai-secret"
assert_not_contains "dotenv anthropic secret does not leak" "$run_dotenv" "dotenv-anthropic-secret"
assert_not_contains "dotenv database secret does not leak" "$run_dotenv" "postgres://dotenv:secret"

assert_contains "runs fmt" "$run_default" "cargo fmt --all -- --check"
assert_contains "runs pre-commit safety" "$run_default" "bash scripts/pre-commit-safety.sh"
assert_contains "runs correctness clippy" "$run_default" "cargo clippy --locked --all-targets -- -D clippy::correctness"
assert_contains "runs lib tests" "$run_default" "cargo test --locked --lib"
assert_contains "runs libsql hermetic tests" "$run_default" "cargo test --locked --no-default-features --features libsql"
assert_not_contains "replay tier is opt-in" "$run_default" "cargo insta test"
assert_not_contains "browser e2e tier is opt-in" "$run_default" "pytest tests/e2e"

run_optional=$(
    IRONCLAW_HERMETIC_DRY_RUN=1 \
    IRONCLAW_HERMETIC_REPLAY=1 \
    IRONCLAW_HERMETIC_E2E=1 \
    scripts/ci/hermetic_gate.sh
)
assert_contains "replay tier can be enabled" "$run_optional" "cargo insta test"
assert_contains "e2e tier builds libsql binary" "$run_optional" "cargo build --locked --no-default-features --features libsql"
assert_contains "e2e tier runs browser smoke slice" "$run_optional" "pytest tests/e2e/scenarios/test_connection.py tests/e2e/scenarios/test_chat.py -v --timeout=120"

if grep -Fq 'hermetic_gate.sh' .githooks/pre-push; then
    echo "OK: pre-push invokes hermetic gate"
    PASS=$((PASS + 1))
else
    echo "FAIL: pre-push invokes hermetic gate"
    FAIL=$((FAIL + 1))
fi

if grep -Fq 'core.hooksPath .githooks' scripts/dev-setup.sh; then
    echo "OK: dev setup uses core.hooksPath"
    PASS=$((PASS + 1))
else
    echo "FAIL: dev setup uses core.hooksPath"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Passed: $PASS, Failed: $FAIL"
[ "$FAIL" -eq 0 ]
