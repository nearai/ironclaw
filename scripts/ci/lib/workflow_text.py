#!/usr/bin/env python3
"""Generic text helpers for reading GitHub Actions workflow YAML.

Deliberately regex-over-text, not a YAML parse: every checker under
`scripts/ci` is stdlib-only so the fast-checks lane needs no pip install,
and several contracts here assert on *formatting* (indentation depth,
step ordering) that a parse would normalise away.

These four were duplicated in `ws12_workflow_contracts.py` — including two
separate, subtly different `JOB_HEADING` definitions where the second
shadowed the first for every validator below it. One definition each,
imported by both callers.
"""

from __future__ import annotations

import re

# Two-space job keys (`  name:` at the top level of `jobs:`). Deeper YAML
# keys are indented further and cannot match.
JOB_HEADING = re.compile(r"^  (?P<name>[A-Za-z0-9_-]+):[ \t]*$", re.MULTILINE)

# One `- name:` step heading. The scan is bounded to its own step because the
# neighbouring `Check all-target lints` legitimately passes `--tests
# --examples`; unbounded, this contract would blame this step for them.
STEP_HEADING = re.compile(r"^[ \t]*- name: (?P<name>.+)$", re.MULTILINE)


def job_blocks(text: str, start: int = 0) -> list[tuple[str, str]]:
    """(job name, block) for every two-space job key at or after `start`.

    The one place that answers "where does a job's YAML block end" — the next
    job heading, or end of file. Both callers that need job boundaries build
    on this: `extract_job_block` filters it to one named job and refuses
    anything but an exact match, and the toolchain contracts enumerate it from
    the `jobs:` key onward.

    `start` exists because JOB_HEADING matches ANY two-space `key:` line, so
    the `on:` trigger's children (`push:`, `workflow_call:`, ...) match too.
    A caller that must not treat those as jobs passes the offset of `jobs:`.
    """
    headings = [h for h in JOB_HEADING.finditer(text) if h.start() >= start]
    return [
        (
            heading.group("name"),
            text[
                heading.start() : headings[index + 1].start()
                if index + 1 < len(headings)
                else len(text)
            ],
        )
        for index, heading in enumerate(headings)
    ]


def step_body(text: str, step_name: str) -> str | None:
    """Return one workflow step's body, bounded by the next step heading."""
    for heading in STEP_HEADING.finditer(text):
        if heading.group("name").strip() != step_name:
            continue
        following = STEP_HEADING.search(text, heading.end())
        return text[heading.end() : following.start() if following else len(text)]
    return None


def job_body(text: str, job_name: str) -> str | None:
    """Return one workflow job's body, bounded by the next job heading."""
    for heading in JOB_HEADING.finditer(text):
        if heading.group("name") != job_name:
            continue
        following = JOB_HEADING.search(text, heading.end())
        return text[heading.end() : following.start() if following else len(text)]
    return None
