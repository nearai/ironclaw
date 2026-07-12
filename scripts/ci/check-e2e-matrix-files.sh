#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
workflow="${1:-.github/workflows/e2e.yml}"

if [[ "${workflow}" != /* ]]; then
  workflow="${repo_root}/${workflow}"
fi

python3 - "${repo_root}" "${workflow}" <<'PY'
import json
import re
import sys
from pathlib import Path

repo_root = Path(sys.argv[1])
workflow = Path(sys.argv[2])
workflow_text = workflow.read_text(encoding="utf-8")

path_re = re.compile(r"tests/e2e/scenarios/[A-Za-z0-9_./-]+\.py")
paths = set(path_re.findall(workflow_text))


def collect_paths(value):
    if isinstance(value, str):
        paths.update(path_re.findall(value))
    elif isinstance(value, list):
        for item in value:
            collect_paths(item)
    elif isinstance(value, dict):
        for item in value.values():
            collect_paths(item)


matrix_assignment_re = re.compile(r"(?m)^\s*([A-Z][A-Z0-9_]*)='(\[[^\n]*\])'\s*$")
parsed_matrix_count = 0

for match in matrix_assignment_re.finditer(workflow_text):
    matrix_name = match.group(1)
    matrix_json = match.group(2)
    try:
        matrix = json.loads(matrix_json)
    except json.JSONDecodeError as exc:
        print(f"{workflow}: invalid JSON in {matrix_name}: {exc}", file=sys.stderr)
        sys.exit(2)
    parsed_matrix_count += 1
    collect_paths(matrix)

if parsed_matrix_count == 0:
    print(
        f"{workflow}: no inline JSON matrix assignments found; checked direct path references only",
        file=sys.stderr,
    )

if not paths:
    print(f"{workflow}: no tests/e2e/scenarios/*.py references found", file=sys.stderr)
    sys.exit(2)

missing = sorted(path for path in paths if not (repo_root / path).is_file())
if missing:
    print(
        "Missing E2E scenario files referenced by .github/workflows/e2e.yml:",
        file=sys.stderr,
    )
    for path in missing:
        print(f"  {path}", file=sys.stderr)
    sys.exit(1)

print(f"All referenced E2E scenario files exist ({len(paths)} checked).")
PY
