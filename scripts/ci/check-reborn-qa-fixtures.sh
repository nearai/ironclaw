#!/usr/bin/env bash
set -euo pipefail

fixture_dir="${1:-tests/fixtures/llm_traces/reborn_qa}"

if [ ! -d "$fixture_dir" ]; then
  echo "Reborn QA fixture directory not found: $fixture_dir" >&2
  exit 1
fi

python3 - "$fixture_dir" <<'PY'
from __future__ import annotations

import pathlib
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

findings: list[tuple[str, str, int, str]] = []
for path in files:
    text = path.read_text(encoding="utf-8")
    for label, pattern in checks:
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            snippet = match.group(0)
            if len(snippet) > 120:
                snippet = snippet[:117] + "..."
            findings.append((str(path), label, line, snippet))

if findings:
    print("Reborn QA fixture scrub check failed:", file=sys.stderr)
    for path, label, line, snippet in findings:
        print(f"{path}:{line}: {label}: {snippet}", file=sys.stderr)
    sys.exit(1)

print(f"Reborn QA fixture scrub check passed ({len(files)} files)")
PY
