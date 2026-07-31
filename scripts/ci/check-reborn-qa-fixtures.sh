#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixture_dir="${1:-tests/fixtures/llm_traces/reborn_qa}"

if [ ! -d "$fixture_dir" ]; then
  echo "Reborn QA fixture directory not found: $fixture_dir" >&2
  exit 1
fi

python3 - "$fixture_dir" <<'PY'
from __future__ import annotations

import pathlib
import json
import re
import sys

fixture_dir = pathlib.Path(sys.argv[1])
files = sorted(fixture_dir.rglob("*.json"))
if not files:
    print(f"no Reborn QA fixture JSON files found under {fixture_dir}", file=sys.stderr)
    sys.exit(1)

checks = [
    (
        "anthropic/openai-style API key",
        re.compile(r"\b(?:sk-ant|sk-proj|sk-live|sk-test|sk-[A-Za-z0-9_-]{24,})\b"),
    ),
    ("google API key", re.compile(r"\bAIza[0-9A-Za-z_-]{20,}\b")),
    ("google OAuth access token", re.compile(r"\bya29\.[0-9A-Za-z._-]+\b")),
    ("slack token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b")),
    (
        "github token",
        re.compile(r"\b(?:ghp_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,})\b"),
    ),
    (
        "bearer token",
        re.compile(r"\bBearer\s+[A-Za-z0-9._-]{20,}\b", re.IGNORECASE),
    ),
    (
        "private key block",
        re.compile(r"-----BEGIN [A-Z ]+PRIVATE KEY-----"),
    ),
    (
        "secret JSON field with raw value",
        re.compile(
            r'"(?:access_token|refresh_token|client_secret|api_key|password)"\s*:\s*'
            r'"(?!<REDACTED>|\[REDACTED\]|redacted)[^"]{8,}"',
            re.IGNORECASE,
        ),
    ),
    ("cookie header/body", re.compile(r"\b(?:cookie|set-cookie)\b", re.IGNORECASE)),
    (
        "email address",
        re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
    ),
    ("local developer path", re.compile(r"/(?:Users|home|tmp)/[^\s\"']+")),
    ("local developer username", re.compile(r"\b(?:firat|sertgoz)\b", re.IGNORECASE)),
]

def has_meaningful_assertion(value: object) -> bool:
    if isinstance(value, dict):
        return any(has_meaningful_assertion(item) for item in value.values())
    if isinstance(value, list):
        return any(has_meaningful_assertion(item) for item in value)
    if isinstance(value, str):
        return bool(value.strip())
    return value is not None

findings: list[tuple[str, str, int]] = []
structure_findings: list[tuple[str, str]] = []
for path in files:
    text = path.read_text(encoding="utf-8")
    try:
        fixture = json.loads(text)
    except json.JSONDecodeError:
        structure_findings.append((str(path), "invalid JSON"))
        continue
    if path.name.endswith(".candidate.json"):
        structure_findings.append((str(path), "candidate file is not a promoted fixture"))
    if "_review" in fixture:
        structure_findings.append((str(path), "review-required candidate metadata remains"))
    promotion = fixture.get("_promotion")
    if promotion is not None:
        if promotion.get("schema_version") != 1:
            structure_findings.append((str(path), "invalid promotion metadata schema"))
        provenance = promotion.get("provenance")
        if not isinstance(provenance, dict) or not all(
            str(provenance.get(field) or "").strip()
            for field in ("source_url", "artifact_schema", "artifact_sha256")
        ):
            structure_findings.append((str(path), "promotion has incomplete provenance"))
        if promotion.get("scrub", {}).get("status") != "verified":
            structure_findings.append((str(path), "promotion scrub status is not verified"))
        if not str(promotion.get("owning_journey") or "").strip():
            structure_findings.append((str(path), "promotion has no owning journey"))
        deterministic_test = promotion.get("deterministic_test")
        if not isinstance(deterministic_test, dict) or not all(
            str(deterministic_test.get(field) or "").strip()
            for field in ("command", "assertion")
        ):
            structure_findings.append(
                (str(path), "promotion has no deterministic regression test evidence")
            )
        replay = promotion.get("last_successful_replay")
        if not isinstance(replay, dict) or not all(
            str(replay.get(field) or "").strip()
            for field in ("date", "commit", "command")
        ):
            structure_findings.append(
                (str(path), "promotion has no successful replay evidence")
            )
    if path.name != "case-manifest.json":
        turns = fixture.get("turns")
        steps = fixture.get("steps")
        if isinstance(turns, list):
            if not turns:
                structure_findings.append((str(path), "fixture has no replayable turns"))
            for index, turn in enumerate(turns):
                expects = turn.get("expects")
                if not isinstance(expects, dict) or not has_meaningful_assertion(expects):
                    structure_findings.append(
                        (str(path), f"turn {index} has empty or missing regression assertions")
                    )
                steps = turn.get("steps")
                if not isinstance(steps, list) or not steps:
                    structure_findings.append(
                        (str(path), f"turn {index} has no deterministic replay steps")
                    )
        elif not isinstance(steps, list) or not steps:
            structure_findings.append((str(path), "fixture has no deterministic replay steps"))
    for label, pattern in checks:
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            findings.append((str(path), label, line))

if findings or structure_findings:
    print("Reborn QA fixture scrub check failed:", file=sys.stderr)
    for path, label, line in findings:
        print(f"{path}:{line}: {label} match redacted from diagnostics", file=sys.stderr)
    for path, message in structure_findings:
        print(f"{path}: {message}", file=sys.stderr)
    sys.exit(1)

print(f"Reborn QA fixture scrub check passed ({len(files)} files)")
PY

promotion_manifest="$fixture_dir/live_canary/case-manifest.json"
if [ ! -f "$promotion_manifest" ]; then
  echo "Reborn QA promotion manifest not found: $promotion_manifest" >&2
  exit 1
fi
python3 "$repo_root/scripts/ci/check-regression-promotions.py" "$promotion_manifest"
