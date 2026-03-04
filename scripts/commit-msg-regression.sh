#!/usr/bin/env bash
# commit-msg hook: require regression tests for fix commits.
#
# Installed by scripts/dev-setup.sh as .git/hooks/commit-msg.
# Bypass with [skip-regression-check] in the commit message.

set -euo pipefail

MSG_FILE="$1"
FIRST_LINE=$(head -1 "$MSG_FILE")

# --- 1. Is this a fix commit? ---
if ! echo "$FIRST_LINE" | grep -qiE '^(fix(\(.*\))?|hotfix|bugfix):'; then
  exit 0
fi

# --- 2. Skip marker ---
if grep -qF '[skip-regression-check]' "$MSG_FILE"; then
  exit 0
fi

# --- 3. Exempt static-only / docs-only changes ---
# Get staged files (commit-msg runs after staging is finalized).
STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACMR)

if [ -z "$STAGED_FILES" ]; then
  exit 0
fi

ALL_EXEMPT=true
while IFS= read -r file; do
  case "$file" in
    src/channels/web/static/*) ;;
    *.md) ;;
    *) ALL_EXEMPT=false; break ;;
  esac
done <<< "$STAGED_FILES"

if [ "$ALL_EXEMPT" = true ]; then
  exit 0
fi

# --- 4. Look for test changes in staged .rs files ---
RS_DIFF=$(git diff --cached -U0 -- '*.rs')

if echo "$RS_DIFF" | grep -qE '^\+.*(#\[test\]|#\[tokio::test\]|#\[cfg\(test\)\]|mod tests)'; then
  exit 0
fi

# Also check for new/modified files under tests/
if echo "$STAGED_FILES" | grep -qE '^tests/'; then
  exit 0
fi

# --- 5. No test found — block the commit ---
echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  REGRESSION TEST REQUIRED                                   ║"
echo "║                                                             ║"
echo "║  This commit looks like a bug fix but has no test changes.  ║"
echo "║  Every fix should include a test that reproduces the bug.   ║"
echo "║                                                             ║"
echo "║  Options:                                                   ║"
echo "║    • Add a #[test] or #[tokio::test] that catches the bug  ║"
echo "║    • Add [skip-regression-check] to your commit message    ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
exit 1
