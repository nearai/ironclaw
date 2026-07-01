#!/usr/bin/env bash
#
# Render a Reborn-scoped coverage summary from a cargo-llvm-cov JSON export.
#
# cargo-llvm-cov instruments the whole workspace build, so a run over the
# Reborn integration-tier test binaries produces coverage for every linked
# crate. This script filters that export down to the Reborn crate families and
# computes an aggregate line-coverage percentage plus a per-crate breakdown
# (the "hole list" — which Reborn crates are least covered).
#
# Usage:
#   reborn-coverage-summary.sh <llvm-cov-json-export>
#     Writes a GitHub-flavored Markdown report to stdout. The caller redirects
#     it into "$GITHUB_STEP_SUMMARY" (or anywhere else).
#   reborn-coverage-summary.sh --zero-crates <llvm-cov-json-export>
#     Writes, one per line, the Reborn crates that have instrumented lines but
#     zero of them covered (the "breadth" holes). Same crate filter and
#     aggregation as the report — this script is the single owner of that
#     computation so reborn-coverage-comment.sh never has to recompute it.
#
# The Reborn crate families mirror the package allowlist in
# .github/workflows/reborn-tests.yml: ironclaw_reborn*, ironclaw_product*,
# ironclaw_architecture, the v2 channel adapters, and ironclaw_webui_v2*.

set -euo pipefail

mode="report"
if [ "${1:-}" = "--zero-crates" ]; then
  mode="zero-crates"
  shift
fi

json_path="${1:?usage: reborn-coverage-summary.sh [--zero-crates] <llvm-cov-json-export>}"

if [ ! -f "${json_path}" ]; then
  echo "coverage JSON not found: ${json_path}" >&2
  exit 1
fi

# Matches the absolute filenames llvm-cov emits, e.g.
# /work/ironclaw/crates/ironclaw_reborn/src/runtime.rs
#
# Mirrors the Reborn crate allowlist in .github/workflows/reborn-tests.yml:
# prefix-match for the reborn_*/product_*/webui_v2_* families, exact-match
# (no trailing name chars before the "/") for the four single crates. The
# trailing "/" anchors the crate-name boundary in all cases.
reborn_regex='/crates/(ironclaw_(reborn|product|webui_v2)[a-z0-9_]*|ironclaw_architecture|ironclaw_slack_v2_adapter|ironclaw_telegram_v2_adapter|ironclaw_wasm_product_adapters)/'

jq -r --arg re "${reborn_regex}" --arg mode "${mode}" '
  # Round to 2 decimal places.
  def round2: . * 100 | round / 100;

  # All instrumented files that belong to a Reborn crate family. llvm-cov nests
  # files under data[]; iterate every dataset (a single `cargo llvm-cov --json`
  # emits one, but the export format permits several) so no coverage is dropped.
  # The trailing "?"s swallow a missing/empty data or files, routing no-coverage
  # runs through the $total == 0 branch below instead of erroring.
  [ (.data[]?.files[]?)
    | select(.filename | test($re))
    | { crate: (.filename | capture("/crates/(?<c>ironclaw_[a-z0-9_]+)/").c),
        covered: .summary.lines.covered,
        count: .summary.lines.count }
  ] as $files

  | ($files | map(.count)   | add // 0) as $total
  | ($files | map(.covered) | add // 0) as $hit
  | (if $total > 0 then ($hit / $total * 100) else 0 end) as $pct

  | ($files
      | group_by(.crate)
      | map({ crate: .[0].crate,
              covered: (map(.covered) | add // 0),
              count:   (map(.count)   | add // 0) })
      | map(. + { pct: (if .count > 0 then (.covered / .count * 100) else 0 end) })
      | sort_by(.pct)
    ) as $byCrate

  # --zero-crates: just the breadth holes (crates instrumented but 0% covered),
  # one crate name per line, sorted. The Markdown report is the default mode.
  | if $mode == "zero-crates"
    then ( $byCrate[] | select(.count > 0 and .covered == 0) | .crate )
    else
      "## Reborn integration-tier coverage",
      "",
      (if $total > 0
       then "**Line coverage (Reborn crates): \($pct | round2)%** — \($hit) / \($total) lines"
       else "**No Reborn crate coverage data found** (0 instrumented lines matched the Reborn crate filter)."
       end),
      "",
      (if $total > 0
       then ( "<details><summary>Per-crate breakdown (\($byCrate | length) crates, lowest-covered first)</summary>",
              "",
              "| Crate | Line % | Covered / Total |",
              "|---|---:|---:|",
              ( $byCrate[]
                | "| `\(.crate)` | \(.pct | round2)% | \(.covered) / \(.count) |" ),
              "",
              "</details>",
              "",
              "_This signal is informational: coverage never gates the PR — not the percentage, not the per-crate holes, not the 0-coverage callout._"
            )
       else empty
       end)
    end
' "${json_path}"
