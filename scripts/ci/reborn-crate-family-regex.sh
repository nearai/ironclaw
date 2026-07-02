#!/usr/bin/env bash
#
# Single source of truth for the Reborn crate-family predicate: which
# workspace crates count as "Reborn" for CI test discovery
# (.github/workflows/reborn-tests.yml package-matrix) and coverage reporting
# (scripts/ci/reborn-coverage-summary.sh, reborn-crate-family-packages.sh).
# Both consumers previously hand-maintained their own copy of this set (a jq
# startswith/== chain vs a regex), which could silently drift apart (#5477).
# This is now the one place the crate-family membership is written down.
#
# Prints ONE line: a POSIX ERE alternation body (no anchors, no wrapping
# capture group) that matches a Reborn-family package name exactly, e.g.
# "ironclaw_reborn_composition" or "ironclaw_architecture". Callers add
# whatever anchoring their context needs — e.g. `^(...)$` for jq's test(),
# or `/crates/(...)/ ` to match an llvm-cov export's file path.

set -euo pipefail

printf '%s\n' 'ironclaw_reborn[a-z0-9_]*|ironclaw_product[a-z0-9_]*|ironclaw_webui_v2[a-z0-9_]*|ironclaw_architecture|ironclaw_slack_v2_adapter|ironclaw_telegram_v2_adapter|ironclaw_wasm_product_adapters'
