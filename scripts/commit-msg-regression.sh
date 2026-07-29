#!/usr/bin/env bash
# commit-msg hook: require meaningful regression tests for fix commits.
#
# Installed by scripts/dev-setup.sh as .git/hooks/commit-msg.
# A deterministic-reproduction impossibility exemption must include its reason;
# CI still requires an independent approving review before accepting it.

set -euo pipefail

MSG_FILE="$1"
REPO_ROOT=$(git rev-parse --show-toplevel)
FIRST_LINE=$(head -1 "$MSG_FILE")
COMMIT_BODY=$(<"$MSG_FILE")

python3 "$REPO_ROOT/scripts/ci/regression-test-check.py" \
  --repo "$REPO_ROOT" \
  --base HEAD \
  --head INDEX \
  --title "$FIRST_LINE" \
  --commit-bodies "$COMMIT_BODY" \
  --allow-unreviewed-reasoned-marker
