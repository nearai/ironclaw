#!/usr/bin/env python3
"""Validate promoted regression provenance, replay freshness, and live retirement."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import re
import sys
from typing import Any


def require_text(value: Any, field: str, errors: list[str]) -> str:
    text = str(value or "").strip()
    if not text:
        errors.append(f"missing {field}")
    return text


def current_date(override: dt.date | None) -> dt.date:
    return override if override is not None else dt.date.today()


def live_qa_matrix_cases(workflow: str, errors: list[str]) -> set[str]:
    job_match = re.search(
        r"(?ms)^  reborn-webui-v2-live-qa:\n(?P<job>.*?)(?=^  [a-zA-Z0-9_-]+:\n|\Z)",
        workflow,
    )
    if not job_match:
        errors.append("live-canary workflow has no reborn-webui-v2-live-qa job")
        return set()

    cases: set[str] = set()
    for value in re.findall(r"(?m)^ {12}cases:\s*([^#\n]+)$", job_match.group("job")):
        cases.update(case.strip() for case in value.split(",") if case.strip())
    if not cases:
        errors.append("reborn-webui-v2-live-qa matrix has no cases")
    return cases


def validate(
    manifest_path: pathlib.Path, today: dt.date, repo_root: pathlib.Path
) -> list[str]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    errors: list[str] = []
    metadata = manifest.get("promotion_metadata")
    if not isinstance(metadata, dict):
        return ["missing promotion_metadata"]
    if metadata.get("schema_version") != 1:
        errors.append("promotion_metadata.schema_version must be 1")
    if not isinstance(metadata.get("fixture_schema_version"), int):
        errors.append("missing fixture_schema_version")

    provenance = metadata.get("provenance")
    if not isinstance(provenance, dict):
        errors.append("missing provenance")
    else:
        require_text(provenance.get("source_kind"), "provenance.source_kind", errors)
        require_text(provenance.get("source_url"), "provenance.source_url", errors)
        require_text(provenance.get("source_commit"), "provenance.source_commit", errors)

    scrub = metadata.get("scrub")
    if not isinstance(scrub, dict) or scrub.get("status") != "verified":
        errors.append("scrub.status must be verified")
    else:
        require_text(scrub.get("pipeline"), "scrub.pipeline", errors)
        require_text(scrub.get("checker"), "scrub.checker", errors)

    owning = metadata.get("owning_journey")
    if not isinstance(owning, dict) or owning.get("strategy") != "case_id":
        errors.append("owning_journey.strategy must be case_id")
    else:
        require_text(owning.get("registry"), "owning_journey.registry", errors)

    replay = metadata.get("last_successful_replay")
    replay_command = ""
    max_age = metadata.get("max_replay_age_days")
    if type(max_age) is not int or max_age <= 0:
        errors.append("max_replay_age_days must be a positive integer")
    if not isinstance(replay, dict):
        errors.append("missing last_successful_replay")
    else:
        require_text(replay.get("commit"), "last_successful_replay.commit", errors)
        replay_command = require_text(
            replay.get("command"), "last_successful_replay.command", errors
        )
        replay_date_text = require_text(
            replay.get("date"), "last_successful_replay.date", errors
        )
        if replay_date_text and isinstance(max_age, int) and max_age > 0:
            try:
                replay_date = dt.date.fromisoformat(replay_date_text)
            except ValueError:
                errors.append("last_successful_replay.date must be YYYY-MM-DD")
            else:
                age = (today - replay_date).days
                if age < 0 or age > max_age:
                    errors.append(
                        f"last_successful_replay is stale ({age} days; maximum {max_age})"
                    )

    lifecycle = metadata.get("live_retirement")
    selected = set(manifest.get("selected_cases", []))
    no_model = set(manifest.get("no_model_cases", []))
    quarantined = set(manifest.get("quarantined_model_cases", []))
    if not isinstance(lifecycle, dict):
        errors.append("missing live_retirement")
        return errors
    minimum = lifecycle.get("minimum_representative_drift_cases")
    drift = lifecycle.get("representative_drift_cases")
    retired = lifecycle.get("retired_cases")
    retirement_evidence = lifecycle.get("retirement_evidence")
    minimum_is_valid = type(minimum) is int and minimum >= 1
    if not minimum_is_valid:
        errors.append("minimum_representative_drift_cases must be positive")
    minimum_count = minimum if minimum_is_valid else 1
    if not isinstance(drift, list) or len(set(drift)) < minimum_count:
        errors.append("representative drift suite is below its minimum")
        drift = []
    drift_set = set(drift)
    for case in sorted(drift_set - selected):
        errors.append(f"drift case is not in the harvested inventory: {case}")
    for case in sorted(drift_set & no_model):
        errors.append(f"drift case has no model replay evidence: {case}")
    for case in sorted(drift_set & quarantined):
        errors.append(f"drift case has quarantined replay evidence: {case}")

    if not isinstance(retired, list):
        errors.append("retired_cases must be a list")
        return errors
    if not isinstance(retirement_evidence, dict):
        errors.append("missing retirement_evidence")
        retirement_evidence = {}
    deterministic_test = require_text(
        retirement_evidence.get("deterministic_test"),
        "retirement_evidence.deterministic_test",
        errors,
    )
    require_text(retirement_evidence.get("reason"), "retirement_evidence.reason", errors)
    retired_at = require_text(
        retirement_evidence.get("retired_at"), "retirement_evidence.retired_at", errors
    )
    if deterministic_test and deterministic_test != replay_command:
        errors.append(
            "retirement_evidence.deterministic_test must match "
            "last_successful_replay.command"
        )
    if retired_at:
        try:
            retirement_date = dt.date.fromisoformat(retired_at)
        except ValueError:
            errors.append("retirement_evidence.retired_at must be YYYY-MM-DD")
        else:
            if retirement_date > today:
                errors.append("retirement_evidence.retired_at cannot be in the future")

    retired_set: set[str] = set()
    for index, entry in enumerate(retired):
        field = f"retired_cases[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{field} must be an object")
            continue
        case = require_text(entry.get("case"), f"{field}.case", errors)
        fixture = require_text(
            entry.get("deterministic_fixture"), f"{field}.deterministic_fixture", errors
        )
        if case in retired_set:
            errors.append(f"{field}.case is duplicated")
        if case:
            retired_set.add(case)
        if case in drift_set:
            errors.append(f"{field}.case cannot also be representative drift")
        if case and case not in selected:
            errors.append(f"{field}.case is not in the harvested inventory")
        if case and case in no_model | quarantined:
            errors.append(f"{field}.case has no active deterministic replay")
        expected_fixture = (
            f"tests/fixtures/llm_traces/reborn_qa/live_canary/{case}.json"
        )
        if case and fixture != expected_fixture:
            errors.append(f"{field}.deterministic_fixture must match its case")
        elif fixture and not (repo_root / fixture).is_file():
            errors.append(f"{field}.deterministic_fixture does not exist")

    workflow = (repo_root / ".github/workflows/live-canary.yml").read_text(
        encoding="utf-8"
    )
    matrix_cases = live_qa_matrix_cases(workflow, errors)
    for case in sorted(matrix_cases - selected):
        errors.append(f"matrix case is not in the harvested inventory: {case}")
    for case in sorted(selected - matrix_cases):
        errors.append(f"harvested case is missing from live-canary matrix: {case}")

    scheduled_match = re.search(
        r"REQUESTED_CASES:\s*\$\{\{\s*github\.event_name == 'schedule'\s*&&\s*'([^']+)'",
        workflow,
    )
    scheduled: set[str] = set()
    if not scheduled_match:
        errors.append("live-canary workflow has no mechanical scheduled case selection")
    else:
        scheduled_selector = scheduled_match.group(1).strip()
        if scheduled_selector.lower() == "all" or scheduled_selector == "*":
            scheduled = set(matrix_cases)
        else:
            scheduled = {
                case.strip()
                for case in scheduled_selector.split(",")
                if case.strip()
            }

    for case in sorted(scheduled - selected):
        errors.append(f"scheduled case is not in the harvested inventory: {case}")
    for case in sorted(scheduled - matrix_cases):
        errors.append(f"scheduled case is not in the live-canary matrix: {case}")
    for case in sorted(drift_set - scheduled):
        errors.append(f"representative drift case is not scheduled: {case}")
    for case in sorted(retired_set & scheduled):
        errors.append(f"retired case is still scheduled: {case}")

    replayable = selected - no_model - quarantined
    accounted = scheduled | retired_set
    missing = sorted(replayable - accounted)
    if missing:
        errors.append(
            "replayable cases must be scheduled or retired: "
            + ", ".join(missing)
        )
    unscheduled_without_replay = sorted((no_model | quarantined) - scheduled)
    if unscheduled_without_replay:
        errors.append(
            "cases without active deterministic replay must remain scheduled: "
            + ", ".join(unscheduled_without_replay)
        )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=pathlib.Path)
    parser.add_argument("--today", type=dt.date.fromisoformat)
    args = parser.parse_args()
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    today = current_date(args.today)
    try:
        errors = validate(args.manifest, today, repo_root)
    except (OSError, json.JSONDecodeError) as error:
        print(f"could not validate promotion manifest: {error}", file=sys.stderr)
        return 2
    if errors:
        for error in errors:
            print(f"regression promotion metadata: {error}", file=sys.stderr)
        return 1
    print("Regression promotion metadata check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
