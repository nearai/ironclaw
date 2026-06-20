#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

workflow=".github/workflows/live-canary.yml"

if [[ ! -f "${workflow}" ]]; then
    echo "missing ${workflow}"
    exit 1
fi

python3 - <<'PY'
from pathlib import Path
import re

workflow = Path(".github/workflows/live-canary.yml")
text = workflow.read_text()

paid_provider_jobs = [
    "public-smoke",
    "persona-rotating",
    "provider-matrix",
    "release-public-full",
]

def job_block(job: str) -> str:
    pattern = re.compile(
        rf"^  {re.escape(job)}:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    match = pattern.search(text)
    if not match:
        raise AssertionError(f"missing job {job}")
    return match.group("body")

for job in paid_provider_jobs:
    body = job_block(job)
    if "github.event_name == 'schedule'" in body:
        raise AssertionError(f"{job} must remain manual-only in the fork")

anthropic_defaults = re.findall(
    r"vars\.LIVE_ANTHROPIC_MODEL \|\| '([^']+)'",
    text,
)
if not anthropic_defaults:
    raise AssertionError("missing Anthropic model fallback")

bad_defaults = [model for model in anthropic_defaults if "haiku" not in model]
if bad_defaults:
    raise AssertionError(f"Anthropic fallback model must be Haiku: {bad_defaults}")

if "Provider-backed LLM lanes stay manual in this fork" not in text:
    raise AssertionError("missing fork live-canary policy comment")

print("live-canary fork policy OK")
PY
