#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
score.sh compares GitHub Actions active time for CI build-time experiments.

Usage:
  score.sh [--repo nearai/ironclaw] [--base-branch main] [--candidate-branch BRANCH]
           [--workflows reborn-tests.yml,reborn-e2e.yml,reborn-coverage.yml,test.yml]

Environment:
  REPO                  GitHub repository, default nearai/ironclaw
  BASE_BRANCH           Baseline branch, default main
  CANDIDATE_BRANCH      Candidate branch. If omitted, prints baseline only.
  WORKFLOWS             Comma-separated workflow files for automatic run lookup.
  BASE_RUN_IDS          Comma-separated explicit baseline run IDs.
  CANDIDATE_RUN_IDS     Comma-separated explicit candidate run IDs.

Notes:
  - Explicit run IDs override workflow lookup.
  - Automatic lookup uses the latest successful completed run for each workflow.
  - The score is based on workflow active time, not queue time.
USAGE
}

repo="${REPO:-nearai/ironclaw}"
base_branch="${BASE_BRANCH:-main}"
candidate_branch="${CANDIDATE_BRANCH:-}"
workflows="${WORKFLOWS:-reborn-tests.yml,reborn-e2e.yml,reborn-coverage.yml,test.yml}"
base_run_ids="${BASE_RUN_IDS:-}"
candidate_run_ids="${CANDIDATE_RUN_IDS:-}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo)
      repo="$2"
      shift 2
      ;;
    --base-branch)
      base_branch="$2"
      shift 2
      ;;
    --candidate-branch)
      candidate_branch="$2"
      shift 2
      ;;
    --workflows)
      workflows="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 2
  fi
}

need gh
need jq
need awk

csv_to_lines() {
  printf '%s\n' "$1" | tr ',' '\n' | sed '/^[[:space:]]*$/d'
}

latest_successful_run_id() {
  branch="$1"
  workflow="$2"
  gh run list \
    --repo "$repo" \
    --workflow "$workflow" \
    --branch "$branch" \
    --limit 20 \
    --json databaseId,status,conclusion \
    --jq '.[] | select(.status == "completed" and .conclusion == "success") | .databaseId' \
    | head -n 1
}

resolve_run_ids() {
  explicit_ids="$1"
  branch="$2"
  if [ -n "$explicit_ids" ]; then
    csv_to_lines "$explicit_ids"
    return
  fi

  while IFS= read -r workflow; do
    run_id="$(latest_successful_run_id "$branch" "$workflow")"
    if [ -z "$run_id" ]; then
      echo "no successful completed run found for workflow $workflow on branch $branch" >&2
      exit 1
    fi
    printf '%s\n' "$run_id"
  done < <(csv_to_lines "$workflows")
}

metrics_for_run() {
  run_id="$1"
  gh run view "$run_id" --repo "$repo" --json databaseId,url,conclusion,createdAt,updatedAt,jobs \
    | jq -r '
      def seconds_between($start; $end): (($end | fromdateiso8601) - ($start | fromdateiso8601));
      def job_seconds:
        [
          .jobs[]
          | select(.startedAt != null and .completedAt != null)
          | seconds_between(.startedAt; .completedAt)
        ];
      {
        id: .databaseId,
        url: .url,
        conclusion: (.conclusion // ""),
        active_seconds: seconds_between(.createdAt; .updatedAt),
        critical_job_seconds: ((job_seconds | max) // 0),
        job_count: (.jobs | length),
        failed_jobs: ([.jobs[] | select(.conclusion != "success" and .conclusion != "skipped") | .name] | length)
      }
      | [.id, .conclusion, .active_seconds, .critical_job_seconds, .job_count, .failed_jobs, .url]
      | @tsv'
}

collect_metrics() {
  label="$1"
  output="$2"
  shift 2
  : > "$output"
  for run_id in "$@"; do
    metrics_for_run "$run_id" >> "$output"
  done

  echo
  echo "## $label"
  printf 'run_id\tconclusion\tactive_seconds\tcritical_job_seconds\tjob_count\tfailed_jobs\turl\n'
  cat "$output"
}

sum_column() {
  file="$1"
  column="$2"
  awk -v col="$column" '{ sum += $col } END { printf "%.0f", sum }' "$file"
}

failed_jobs_total() {
  file="$1"
  awk '{ sum += $6 } END { printf "%.0f", sum }' "$file"
}

base_ids_tmp="$(mktemp)"
candidate_ids_tmp="$(mktemp)"
base_metrics_tmp="$(mktemp)"
candidate_metrics_tmp="$(mktemp)"
trap 'rm -f "$base_ids_tmp" "$candidate_ids_tmp" "$base_metrics_tmp" "$candidate_metrics_tmp"' EXIT

resolve_run_ids "$base_run_ids" "$base_branch" > "$base_ids_tmp"
base_ids=()
while IFS= read -r run_id || [ -n "$run_id" ]; do
  base_ids+=("$run_id")
done < "$base_ids_tmp"

echo "# CI build-time score"
echo
echo "repo: $repo"
echo "base_branch: $base_branch"
echo "candidate_branch: ${candidate_branch:-<none>}"
echo "workflows: $workflows"

collect_metrics "Baseline" "$base_metrics_tmp" "${base_ids[@]}"
base_active="$(sum_column "$base_metrics_tmp" 3)"
base_failed="$(failed_jobs_total "$base_metrics_tmp")"

if [ "$base_failed" -gt 0 ]; then
  echo
  echo "VOID: baseline contains failed jobs. Use successful baselines or explicit accepted probe run IDs." >&2
  exit 1
fi

if [ -z "$candidate_branch" ] && [ -z "$candidate_run_ids" ]; then
  echo
  echo "baseline_active_seconds=$base_active"
  echo "candidate not provided; baseline-only mode."
  exit 0
fi

resolve_run_ids "$candidate_run_ids" "$candidate_branch" > "$candidate_ids_tmp"
candidate_ids=()
while IFS= read -r run_id || [ -n "$run_id" ]; do
  candidate_ids+=("$run_id")
done < "$candidate_ids_tmp"

collect_metrics "Candidate" "$candidate_metrics_tmp" "${candidate_ids[@]}"
candidate_active="$(sum_column "$candidate_metrics_tmp" 3)"
candidate_failed="$(failed_jobs_total "$candidate_metrics_tmp")"

if [ "$candidate_failed" -gt 0 ]; then
  echo
  echo "VOID: candidate contains failed jobs." >&2
  exit 1
fi

improvement="$(
  awk -v base="$base_active" -v candidate="$candidate_active" '
    BEGIN {
      if (base <= 0) {
        print "0.00"
      } else {
        printf "%.2f", 100 * (base - candidate) / base
      }
    }'
)"

echo
echo "baseline_active_seconds=$base_active"
echo "candidate_active_seconds=$candidate_active"
echo "speedup_percent=$improvement"

awk -v improvement="$improvement" 'BEGIN {
  if (improvement >= 30) {
    print "result=PASS"
  } else {
    print "result=FAIL"
  }
}'
