#!/usr/bin/env bash
# Pre-commit safety checks for common issues caught by AI code reviewers.
#
# Can be run standalone: bash scripts/pre-commit-safety.sh
# Or installed as a git pre-commit hook via dev-setup.sh.
#
# Checks staged .rs files for:
#   1. Unsafe UTF-8 byte slicing (panics on multi-byte chars)
#   2. Case-sensitive file extension comparisons
#   3. Hardcoded /tmp paths in tests (flaky in parallel runs)
#   4. Tool parameters logged without redaction (secret leaks)
#   5. Multi-step DB operations without transaction wrapping
#   6. .unwrap(), .expect(), assert!() in production code (panics)
#   7. Gateway/CLI handlers bypassing ToolDispatcher (must go through tools)
#   8. CredentialName referenced in web-layer code (wrong identity at boundary)
#
# Also runs check-i18n-parity.sh when crates/ironclaw_gateway/static/i18n/*.js
# files are staged, to ensure every language pack has the same key set.
#
# Suppress individual lines with an inline "// safety: <reason>" comment.
# For check #7, use "// dispatch-exempt: <reason>" instead.
# For check #8, use "// web-identity-exempt: <reason>" instead.

set -euo pipefail

TEST_BOUNDARIES_FILE=""
GATEWAY_APP_JS_TMP=""

# Determine a suitable base ref for standalone diffs.
resolve_base_ref() {
    local candidates=(
        "@{upstream}"
        "origin/HEAD"
        "origin/main"
        "origin/master"
        "main"
        "master"
    )

    for ref in "${candidates[@]}"; do
        if git rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
            echo "$ref"
            return 0
        fi
    done

    echo "pre-commit-safety: could not determine a base Git ref for diff (tried: ${candidates[*]})." >&2
    echo "pre-commit-safety: ensure your repository has an upstream or a local main/master branch." >&2
    exit 1
}

# i18n parity: when any language pack changes, all languages must stay in sync.
# Run before the .rs-focused checks so it fires even when no .rs files change.
if git diff --cached --quiet 2>/dev/null; then
    HAS_STAGED_CHANGES=0
    I18N_CHANGED=$(git diff --name-only -- 'crates/ironclaw_gateway/static/i18n/*.js' 2>/dev/null || true)
    GATEWAY_APP_JS_CHANGED=$(git diff --name-only -- 'crates/ironclaw_gateway/static/app.js' 2>/dev/null || true)
else
    HAS_STAGED_CHANGES=1
    I18N_CHANGED=$(git diff --cached --name-only -- 'crates/ironclaw_gateway/static/i18n/*.js' 2>/dev/null || true)
    GATEWAY_APP_JS_CHANGED=$(git diff --cached --name-only -- 'crates/ironclaw_gateway/static/app.js' 2>/dev/null || true)
fi
if [ -n "$I18N_CHANGED" ]; then
    # Resolve script location even when invoked via a symlink (the
    # pre-commit hook is typically a symlink at .git/hooks/pre-commit
    # pointing to this file in scripts/). Walk symlinks until we find the
    # real path, then use its parent directory.
    SOURCE="${BASH_SOURCE[0]:-$0}"
    while [ -L "$SOURCE" ]; do
        LINK_TARGET="$(readlink "$SOURCE")"
        case "$LINK_TARGET" in
            /*) SOURCE="$LINK_TARGET" ;;
            *)  SOURCE="$(cd "$(dirname "$SOURCE")" && pwd)/$LINK_TARGET" ;;
        esac
    done
    SCRIPT_DIR="$(cd "$(dirname "$SOURCE")" && pwd)"
    if ! "$SCRIPT_DIR/check-i18n-parity.sh"; then
        echo ""
        echo "Commit blocked: i18n parity check failed."
        echo "Every key added to en.js must also be added to all other language files (zh-CN.js, ko.js, ...)."
        echo "Placeholder tokens like {name} must match across all languages."
        echo "To bypass: git commit --no-verify"
        exit 1
    fi
fi

# Gateway frontend JS must parse cleanly; a syntax error leaves the auth shell
# visible and prevents the app bootstrap from running at all.
if [ -n "$GATEWAY_APP_JS_CHANGED" ]; then
    if ! command -v node >/dev/null 2>&1; then
        echo ""
        echo "Commit blocked: Node.js is required to validate gateway app.js syntax."
        echo "Install Node.js and rerun the commit, or bypass with git commit --no-verify"
        exit 1
    fi

    GATEWAY_APP_JS_TMP=$(mktemp "${TMPDIR:-/tmp}/gateway-app-js.XXXXXX.js")
    trap 'rm -f "${TEST_BOUNDARIES_FILE:-}" "${GATEWAY_APP_JS_TMP:-}"' EXIT
    if [ "$HAS_STAGED_CHANGES" -eq 1 ]; then
        git show ":crates/ironclaw_gateway/static/app.js" > "$GATEWAY_APP_JS_TMP"
    else
        cp crates/ironclaw_gateway/static/app.js "$GATEWAY_APP_JS_TMP"
    fi

    if ! node --check "$GATEWAY_APP_JS_TMP" >/dev/null; then
        echo ""
        echo "Commit blocked: gateway app.js failed syntax validation."
        echo "Fix the parse error in crates/ironclaw_gateway/static/app.js or bypass with git commit --no-verify"
        exit 1
    fi
fi

# Support both pre-commit hook (staged files) and standalone (all changed vs base)
if git diff --cached --quiet 2>/dev/null; then
    # No staged changes -- compare working tree against a resolved base ref
    BASE_REF="$(resolve_base_ref)"
    DIFF_OUTPUT=$(git diff "$BASE_REF" -- '*.rs' 2>/dev/null || true)
    CHANGED_FILES=$(git diff --name-only "$BASE_REF" -- '*.rs' 2>/dev/null || true)
else
    DIFF_OUTPUT=$(git diff --cached -U0 -- '*.rs' 2>/dev/null || true)
    CHANGED_FILES=$(git diff --cached --name-only --diff-filter=AM -- '*.rs' 2>/dev/null || true)
fi

# Early exit if there are no relevant .rs changes
if [ -z "$DIFF_OUTPUT" ]; then
    exit 0
fi

# Build a "test mod start line" lookup per changed file by scanning the actual
# file content. The previous heuristic relied on `mod tests` appearing in the
# git diff hunk header (`@@ ... mod tests @@`), but that context only shows up
# for tiny edits inside a known test fn — and never for brand-new files added
# in a merge. Reading the file directly catches both cases.
TEST_BOUNDARIES_FILE=$(mktemp)
trap 'rm -f "${TEST_BOUNDARIES_FILE:-}" "${GATEWAY_APP_JS_TMP:-}"' EXIT
for f in $CHANGED_FILES; do
    [ -f "$f" ] || continue
    test_line=$(awk '
        /^#\[cfg\(test\)\]/ { cfg_at = NR; next }
        cfg_at && /^mod tests([[:space:]]*\{)?[[:space:]]*$/ { print NR; exit }
        cfg_at && NF > 0 && !/^[[:space:]]*$/ { cfg_at = 0 }
    ' "$f")
    if [ -n "$test_line" ]; then
        printf '%s\t%s\n' "$f" "$test_line" >> "$TEST_BOUNDARIES_FILE"
    fi
done

# Filter a unified diff (DIFF_OUTPUT-shaped) to drop `+` lines that come from
# test code: either inside a `#[cfg(test)] mod tests` block (per the
# precomputed boundaries) or in any file under the top-level `tests/` dir.
# Marker, header, and context lines pass through untouched so downstream
# grep/awk pipelines still see file/hunk anchors.
strip_test_mod_lines() {
    awk -v boundaries="$TEST_BOUNDARIES_FILE" '
        BEGIN {
            while ((getline line < boundaries) > 0) {
                idx = index(line, "\t")
                if (idx) {
                    f = substr(line, 1, idx - 1)
                    test_start[f] = substr(line, idx + 1) + 0
                }
            }
            close(boundaries)
            cur_file = ""
            cur_start = 0
            cur_skip_all = 0
            new_line = 0
        }
        /^\+\+\+ b\// {
            cur_file = substr($0, 7)
            cur_start = (cur_file in test_start) ? test_start[cur_file] : 0
            cur_skip_all = (cur_file ~ /(^|\/)tests\//)
            new_line = 0
            print
            next
        }
        /^\+\+\+ / {
            cur_file = ""; cur_start = 0; cur_skip_all = 0; new_line = 0
            print
            next
        }
        /^--- / { print; next }
        /^@@ / {
            # Parse new-file starting line from `@@ -X[,Y] +U[,V] @@`
            n = split($0, parts, " ")
            for (i = 1; i <= n; i++) {
                if (substr(parts[i], 1, 1) == "+") {
                    range = substr(parts[i], 2)
                    c = index(range, ",")
                    if (c) range = substr(range, 1, c - 1)
                    new_line = range + 0 - 1
                    break
                }
            }
            print
            next
        }
        /^\+/ {
            new_line++
            if (cur_skip_all) next
            if (cur_start > 0 && new_line >= cur_start) next
            print
            next
        }
        /^-/ { print; next }
        { new_line++; print }
    '
}

DIFF_OUTPUT_NO_TESTS=$(printf '%s\n' "$DIFF_OUTPUT" | strip_test_mod_lines)

WARNINGS=0

warn() {
    if [ "$WARNINGS" -eq 0 ]; then
        echo ""
        echo "=== Pre-commit Safety Checks ==="
        echo ""
    fi
    WARNINGS=$((WARNINGS + 1))
    echo "  [$1] $2"
}

# 1. Unsafe UTF-8 byte slicing: &s[..N] or &s[..some_var] on strings
#    Safe patterns: is_char_boundary, char_indices, // safety:
if echo "$DIFF_OUTPUT_NO_TESTS" | grep -nE '^\+' | grep -E '\[\.\..*\]' | grep -vE 'is_char_boundary|char_indices|// safety:|as_bytes|Vec<|&\[u8\]|\[u8\]|bytes\(\)|&bytes' | head -3 | grep -q .; then
    warn "UTF8" "Possible unsafe byte-index string slicing. Use is_char_boundary() or char_indices()."
    echo "$DIFF_OUTPUT_NO_TESTS" | grep -nE '^\+' | grep -E '\[\.\..*\]' | grep -vE 'is_char_boundary|char_indices|// safety:|as_bytes|Vec<|&\[u8\]|\[u8\]|bytes\(\)|&bytes' | head -3 | sed 's/^/    /'
fi

# 2. Case-sensitive file extension checks
#    Match: .ends_with(".png") without prior to_lowercase
if echo "$DIFF_OUTPUT" | grep -nE '^\+.*ends_with\("\.([pP][nN][gG]|[jJ][pP][eE]?[gG]|[gG][iI][fF]|[wW][eE][bB][pP]|[mM][dD])"\)' | grep -vE 'to_lowercase|to_ascii_lowercase|// safety:' | head -3 | grep -q .; then
    warn "CASE" "Case-sensitive file extension comparison. Normalize to lowercase first."
    echo "$DIFF_OUTPUT" | grep -nE '^\+.*ends_with\("\.([pP][nN][gG]|[jJ][pP][eE]?[gG]|[gG][iI][fF]|[wW][eE][bB][pP]|[mM][dD])"\)' | grep -vE 'to_lowercase|to_ascii_lowercase|// safety:' | head -3 | sed 's/^/    /'
fi

# 3. Hardcoded /tmp paths in test files
if echo "$DIFF_OUTPUT_NO_TESTS" | grep -nE '^\+.*"/tmp/' | grep -vE 'tempfile|tempdir|// safety:' | head -3 | grep -q .; then
    warn "TMPDIR" "Hardcoded /tmp path. Use tempfile::tempdir() for parallel-safe tests."
    echo "$DIFF_OUTPUT_NO_TESTS" | grep -nE '^\+.*"/tmp/' | grep -vE 'tempfile|tempdir|// safety:' | head -3 | sed 's/^/    /'
fi

# 4. Logging tool parameters without redaction
if echo "$DIFF_OUTPUT" | grep -nE '^\+.*tracing::(info|debug|warn|error).*param' | grep -vE 'redact|// safety:' | head -3 | grep -q .; then
    warn "REDACT" "Logging tool parameters without redaction. Use redact_params() first."
    echo "$DIFF_OUTPUT" | grep -nE '^\+.*tracing::(info|debug|warn|error).*param' | grep -vE 'redact|// safety:' | head -3 | sed 's/^/    /'
fi

# 5. Multi-step DB operations without transaction
#    Uses -W (function context) to reduce false positives from existing transactions.
#    Suppressible with "// safety:" in the hunk.
DIFF_W_OUTPUT=$(git diff --cached -W -- '*.rs' 2>/dev/null || git diff "$(resolve_base_ref)" -W -- '*.rs' 2>/dev/null || true)
if [ -n "$DIFF_W_OUTPUT" ]; then
    HUNK_COUNT=$(echo "$DIFF_W_OUTPUT" | awk '
        /^@@/ {
            if (count >= 2 && !has_tx && !has_safety) found++
            count=0; has_tx=0; has_safety=0
        }
        /^\+.*\.(execute|query)\(/ { count++ }
        /^\+.*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
        / .*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
        /\/\/ safety:/ { has_safety=1 }
        END {
            if (count >= 2 && !has_tx && !has_safety) found++
            print found+0
        }
    ')
    if [ "$HUNK_COUNT" -gt 0 ]; then
        warn "TX" "Multiple DB operations in same function without transaction. Wrap in a transaction for atomicity."
        echo "$DIFF_W_OUTPUT" | awk '
            /^@@/ {
                if (count >= 2 && !has_tx && !has_safety) { print buf }
                buf=""; count=0; has_tx=0; has_safety=0
            }
            /^\+.*\.(execute|query)\(/ { count++ }
            /^\+.*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
            / .*(transaction|\.tx\.|\.begin\()/ { has_tx=1 }
            /\/\/ safety:/ { has_safety=1 }
            { buf = buf "\n" $0 }
            END {
                if (count >= 2 && !has_tx && !has_safety) { print buf }
            }
        ' | grep -E '^\+.*\.(execute|query)\(' | head -4 | sed 's/^/    /'
    fi
fi

# 6. .unwrap(), .expect(), assert!() in production code
#    Matches added lines containing panic-inducing calls.
#    Excludes test files, test modules, and debug_assert (compiled out in release).
#    Suppress with "// safety: <reason>".
PROD_DIFF="$DIFF_OUTPUT_NO_TESTS"
# Strip hunks from test-only files (tests/ directory, *_test.rs, test_*.rs)
PROD_DIFF=$(echo "$PROD_DIFF" | grep -v '^+++ b/tests/' || true)
# Strip hunks whose @@ context line indicates a test module.
# git diff includes the enclosing function/module name after @@.
# Only match `mod tests` (the conventional #[cfg(test)] module) — do NOT
# match `fn test_*` because production code can have functions named test_*.
PROD_DIFF=$(echo "$PROD_DIFF" | awk '
    /^@@ / { in_test = ($0 ~ /mod tests/) }
    !in_test { print }
' || true)
if echo "$PROD_DIFF" | grep -nE '^\+' \
    | grep -E '\.(unwrap|expect)\(|[^_]assert(_eq|_ne)?!' \
    | grep -vE 'debug_assert|// safety:|#\[cfg\(test\)\]|#\[test\]|mod tests' \
    | head -5 | grep -q .; then
    warn "PANIC" "Production code must not use .unwrap(), .expect(), or assert!(). Use proper error handling."
    echo "$PROD_DIFF" | grep -nE '^\+' \
        | grep -E '\.(unwrap|expect)\(|[^_]assert(_eq|_ne)?!' \
        | grep -vE 'debug_assert|// safety:|#\[cfg\(test\)\]|#\[test\]|mod tests' \
        | head -5 | sed 's/^/    /'
fi

# 7. Gateway/CLI handlers bypassing ToolDispatcher.
#    Channel handlers and CLI commands must route mutations through
#    `ToolDispatcher::dispatch()` so every UI/CLI-initiated action gets the
#    same audit trail and safety pipeline as agent-initiated tool calls.
#    See `.claude/rules/tools.md` "Everything Goes Through Tools".
#
#    The check looks at .rs files under src/channels/web/handlers/ and
#    src/cli/ for newly-added lines that touch direct manager fields on the
#    gateway state. Suppress with "// dispatch-exempt: <reason>".
DISPATCH_DIFF=$(git diff --cached -U0 -- 'src/channels/web/handlers/*.rs' 'src/cli/*.rs' 2>/dev/null || true)
if [ -z "$DISPATCH_DIFF" ]; then
    DISPATCH_DIFF=$(git diff "$(resolve_base_ref)" -U0 -- 'src/channels/web/handlers/*.rs' 'src/cli/*.rs' 2>/dev/null || true)
fi
if [ -n "$DISPATCH_DIFF" ]; then
    DISPATCH_HITS=$(echo "$DISPATCH_DIFF" | grep -nE '^\+' \
        | grep -E 'state\.(store|workspace|workspace_pool|extension_manager|skill_registry|session_manager)\.' \
        | grep -vE '// dispatch-exempt:|// safety:|^\+\+\+' \
        | head -5 || true)
    if [ -n "$DISPATCH_HITS" ]; then
        warn "DISPATCH" "Handler directly touches state.{store,workspace,extension_manager,skill_registry,session_manager}. Route through ToolDispatcher::dispatch() instead. See .claude/rules/tools.md."
        echo "$DISPATCH_HITS" | sed 's/^/    /'
    fi
fi

# 8. CredentialName referenced in web-layer code.
#    CredentialName is a backend/secrets-store identity. Web routes and
#    web DTOs take ExtensionName; the dispatcher and auth_manager resolve
#    credential identity from the extension name server-side. An explicit
#    `CredentialName` reference in src/channels/web/** (except inside
#    `#[cfg(test)] mod tests` blocks) means the wrong identity is reaching
#    the web boundary. See src/channels/web/CLAUDE.md "Identity types at
#    the web boundary" and .claude/rules/types.md.
#
#    Suppress with "// web-identity-exempt: <reason>" when the reference
#    is genuinely reading an already-typed value off a backend struct
#    (e.g., destructuring `ResumeKind::Authentication` to log the name).
WEB_IDENTITY_DIFF=$(git diff --cached -U0 -- 'src/channels/web/*.rs' 'src/channels/web/**/*.rs' 2>/dev/null || true)
if [ -z "$WEB_IDENTITY_DIFF" ]; then
    WEB_IDENTITY_DIFF=$(git diff "$(resolve_base_ref)" -U0 -- 'src/channels/web/*.rs' 'src/channels/web/**/*.rs' 2>/dev/null || true)
fi
if [ -n "$WEB_IDENTITY_DIFF" ]; then
    # Strip lines inside `#[cfg(test)] mod tests` blocks using the same
    # precomputed boundaries used for other prod-only checks.
    WEB_IDENTITY_PROD=$(printf '%s\n' "$WEB_IDENTITY_DIFF" | strip_test_mod_lines)
    WEB_IDENTITY_HITS=$(echo "$WEB_IDENTITY_PROD" | grep -nE '^\+' \
        | grep -E '\bCredentialName\b' \
        | grep -vE '// web-identity-exempt:|// safety:|^\+\+\+' \
        | head -5 || true)
    if [ -n "$WEB_IDENTITY_HITS" ]; then
        warn "CREDNAME" "\`CredentialName\` referenced in src/channels/web/** — web code takes \`ExtensionName\`; credential identity stays backend-side. Push the mapping into bridge::auth_manager or annotate with '// web-identity-exempt: <reason>'."
        echo "$WEB_IDENTITY_HITS" | sed 's/^/    /'
    fi
fi

if [ "$WARNINGS" -gt 0 ]; then
    echo ""
    echo "Found $WARNINGS potential issue(s). Fix them or add '// safety: <reason>' to suppress."
    echo "(For DISPATCH warnings, use '// dispatch-exempt: <reason>' instead.)"
    echo "(For CREDNAME warnings, use '// web-identity-exempt: <reason>' instead.)"
    echo ""
    exit 1
fi
