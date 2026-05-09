#!/usr/bin/env bash
# Hermes-style hermetic local quality gate for IronClaw.
#
# Defaults to fast, local-only checks that should not read real provider keys,
# user profiles, or the developer's ~/.ironclaw state. Expensive deterministic
# tiers are opt-in via IRONCLAW_HERMETIC_DUAL_BACKEND=1,
# IRONCLAW_HERMETIC_REPLAY=1, and IRONCLAW_HERMETIC_E2E=1.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

DRY_RUN="${IRONCLAW_HERMETIC_DRY_RUN:-0}"
KEEP_TMP="${IRONCLAW_HERMETIC_KEEP_TMP:-0}"
TMP_ROOT="${IRONCLAW_HERMETIC_TMPDIR:-}"

if [ -z "$TMP_ROOT" ]; then
    TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ironclaw-hermetic.XXXXXX")"
    CREATED_TMP=1
else
    mkdir -p "$TMP_ROOT"
    CREATED_TMP=0
fi

cleanup() {
    if [ "$KEEP_TMP" != "1" ] && [ "${CREATED_TMP:-0}" = "1" ]; then
        rm -rf "$TMP_ROOT"
    fi
}
trap cleanup EXIT

# Credential-bearing environment. The list is intentionally explicit and
# conservative: tests that need fake credentials should set test constants in
# code/fixtures, not inherit developer secrets from the shell.
CREDENTIAL_VARS=(
    OPENAI_API_KEY
    ANTHROPIC_API_KEY
    ANTHROPIC_OAUTH_TOKEN
    GEMINI_API_KEY
    GOOGLE_API_KEY
    LLM_API_KEY
    NEARAI_API_KEY
    NEARAI_SESSION_TOKEN
    TINFOIL_API_KEY
    TRANSCRIPTION_API_KEY
    GOOGLE_OAUTH_CLIENT_ID
    GOOGLE_OAUTH_CLIENT_SECRET
    GOOGLE_OAUTH_TOKEN
    GITHUB_TOKEN
    GH_TOKEN
    SLACK_BOT_TOKEN
    SLACK_APP_TOKEN
    TELEGRAM_BOT_TOKEN
    DISCORD_TOKEN
    NOTION_TOKEN
    AWS_ACCESS_KEY_ID
    AWS_SECRET_ACCESS_KEY
    AWS_SESSION_TOKEN
    AWS_PROFILE
    CHANNEL_RELAY_API_KEY
    DATABASE_URL
    LIBSQL_URL
    LIBSQL_AUTH_TOKEN
    SECRETS_MASTER_KEY
)

# Behavior/configuration variables that can make tests read real local state or
# select non-hermetic backends/providers. Controlled replacements are exported
# below after the unset loop.
BEHAVIOR_VARS=(
    IRONCLAW_BASE_DIR
    IRONCLAW_PROFILE
    DATABASE_BACKEND
    LIBSQL_PATH
    LLM_BACKEND
    LLM_BASE_URL
    LLM_MODEL
)

for var in "${CREDENTIAL_VARS[@]}" "${BEHAVIOR_VARS[@]}"; do
    unset "$var" || true
done

# Keep credential keys present-but-empty so dotenv loaders inside Rust and
# Python children cannot repopulate them from the repository's ignored `.env`
# or the isolated IronClaw `.env`. `dotenvy` does not overwrite existing env
# vars, and IronClaw's config helpers treat empty strings as absent.
for var in "${CREDENTIAL_VARS[@]}"; do
    export "$var="
done

export IRONCLAW_BASE_DIR="$TMP_ROOT/ironclaw"
export DATABASE_BACKEND="libsql"
export LIBSQL_PATH="$IRONCLAW_BASE_DIR/ironclaw.db"
HERMETIC_LOCALE="C"
if command -v locale >/dev/null 2>&1 && locale -a 2>/dev/null | grep -qi '^C\.UTF-8$'; then
    HERMETIC_LOCALE="C.UTF-8"
fi
export TZ="UTC"
export LANG="$HERMETIC_LOCALE"
export LC_ALL="$HERMETIC_LOCALE"
export PYTHONHASHSEED="0"
export AWS_EC2_METADATA_DISABLED="true"
export ONBOARD_COMPLETED="true"

mkdir -p "$IRONCLAW_BASE_DIR"

mask_var() {
    local name="$1"
    if [ -n "${!name+x}" ]; then
        printf '%s=%s\n' "$name" "${!name}"
    else
        printf '%s=<unset>\n' "$name"
    fi
}

print_environment_summary() {
    echo "==> hermetic environment"
    mask_var OPENAI_API_KEY
    mask_var ANTHROPIC_API_KEY
    mask_var NEARAI_API_KEY
    mask_var NEARAI_SESSION_TOKEN
    mask_var TINFOIL_API_KEY
    mask_var LLM_API_KEY
    mask_var DATABASE_URL
    mask_var DATABASE_BACKEND
    mask_var LIBSQL_PATH
    mask_var IRONCLAW_BASE_DIR
    mask_var TZ
    mask_var LANG
    mask_var LC_ALL
    mask_var PYTHONHASHSEED
    mask_var AWS_EC2_METADATA_DISABLED
}

run_cmd() {
    echo "+ $*"
    if [ "$DRY_RUN" = "1" ]; then
        return 0
    fi
    "$@"
}

print_environment_summary

echo "==> fmt check"
run_cmd cargo fmt --all -- --check

echo "==> pre-commit safety checks"
run_cmd bash scripts/pre-commit-safety.sh

echo "==> clippy (correctness)"
run_cmd cargo clippy --locked --all-targets -- -D clippy::correctness

if [ "${IRONCLAW_PREPUSH_TEST:-1}" = "1" ]; then
    echo "==> tests (lib only)"
    run_cmd cargo test --locked --lib

    if [ "${IRONCLAW_HERMETIC_DUAL_BACKEND:-0}" = "1" ]; then
        echo "==> tests (libsql hermetic configuration)"
        run_cmd cargo test --locked --no-default-features --features libsql
    else
        echo "==> libsql hermetic tests skipped (IRONCLAW_HERMETIC_DUAL_BACKEND=1 to enable)"
    fi

    if [ "${IRONCLAW_HERMETIC_REPLAY:-0}" = "1" ]; then
        echo "==> replay snapshot tests"
        run_cmd cargo insta test \
            --check \
            --no-default-features \
            --features "libsql,replay" \
            --test e2e_engine_v2 \
            --test e2e_recorded_trace \
            --test e2e_live
    fi

    if [ "${IRONCLAW_HERMETIC_E2E:-0}" = "1" ]; then
        echo "==> browser e2e smoke build"
        run_cmd cargo build --locked --no-default-features --features libsql

        echo "==> browser e2e smoke"
        run_cmd python3 -m pytest tests/e2e/scenarios/test_connection.py tests/e2e/scenarios/test_chat.py -v --timeout=120
    fi
else
    echo "==> tests skipped (IRONCLAW_PREPUSH_TEST=0)"
fi

if [ "$KEEP_TMP" = "1" ]; then
    echo "tmpdir kept: $TMP_ROOT"
fi
