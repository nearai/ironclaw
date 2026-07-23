"""Explain why IronClaw WebUI v2 live QA cases were green."""

from __future__ import annotations

import json
import time
from dataclasses import dataclass
from pathlib import Path

from scripts.live_canary.common import ProbeResult
from scripts.ironclaw_webui_v2_live_qa.text_match import required_text_matches


def write_green_run_explanation(output_dir: Path, results: list[ProbeResult]) -> Path:
    path = output_dir / "green-run-explanation.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    cases: list[dict[str, object]] = []
    summary = _GreenRunSummary()

    for result in results:
        case = _explain_case(result)
        summary.add(case)
        cases.append(case.to_json())

    payload = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "why_things_were_green": summary.message(),
        **summary.to_json(),
        "cases": cases,
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


@dataclass(frozen=True)
class _CaseExplanation:
    name: str
    mode: str
    success: bool
    blocked: bool
    required_text: list[str]
    text_excerpt_present: bool
    literal_required_text_matched: bool
    semantic_judge_used: bool
    semantic_judge_reason: str | None
    semantic_judge_summary: dict[str, object] | None

    def success_reasons(self) -> list[str]:
        if not self.success:
            return []
        reasons: list[str] = []
        if self.literal_required_text_matched:
            reasons.append("literal_required_text_matched")
        if self.semantic_judge_used:
            reasons.append("semantic_judge_completed")
        if not self.required_text:
            reasons.append("case_success_from_non_text_assertions")
        if not reasons:
            reasons.append("case_success_reason_unclassified")
        return reasons

    def to_json(self) -> dict[str, object]:
        return {
            "case": self.name,
            "mode": self.mode,
            "success": self.success,
            "blocked": self.blocked,
            "required_text": self.required_text,
            "text_excerpt_present": self.text_excerpt_present,
            "literal_required_text_matched": self.literal_required_text_matched,
            "semantic_judge_used": self.semantic_judge_used,
            "semantic_judge_reason": self.semantic_judge_reason,
            "semantic_judge_summary": self.semantic_judge_summary,
            "success_reasons": self.success_reasons(),
        }


@dataclass
class _GreenRunSummary:
    total_cases: int = 0
    successful_cases: int = 0
    failed_cases: int = 0
    successful_cases_matching_required_text_literally: int = 0
    successful_cases_using_semantic_judge: int = 0

    def add(self, case: _CaseExplanation) -> None:
        self.total_cases += 1
        if not case.success:
            self.failed_cases += 1
            return
        self.successful_cases += 1
        if case.literal_required_text_matched:
            self.successful_cases_matching_required_text_literally += 1
        if case.semantic_judge_used:
            self.successful_cases_using_semantic_judge += 1

    def message(self) -> str:
        if self.failed_cases:
            status = (
                f"{self.successful_cases} of {self.total_cases} cases were green; "
                f"{self.failed_cases} failed."
            )
        else:
            status = f"All {self.total_cases} cases were green."
        literal = (
            f"{self.successful_cases_matching_required_text_literally} successful cases "
            "matched their required text literally."
        )
        if self.successful_cases_using_semantic_judge:
            judge = (
                f"{self.successful_cases_using_semantic_judge} successful cases used "
                "the semantic judge fallback."
            )
        else:
            judge = "No successful cases used the semantic judge fallback."
        return f"{status} {literal} {judge}"

    def to_json(self) -> dict[str, int]:
        return {
            "total_cases": self.total_cases,
            "successful_cases": self.successful_cases,
            "failed_cases": self.failed_cases,
            "successful_cases_matching_required_text_literally": (
                self.successful_cases_matching_required_text_literally
            ),
            "successful_cases_using_semantic_judge": (
                self.successful_cases_using_semantic_judge
            ),
        }


def _explain_case(result: ProbeResult) -> _CaseExplanation:
    details = result.details if isinstance(result.details, dict) else {}
    required_text = _required_text_from_details(details.get("required_text"))
    text_excerpt = str(details.get("text_excerpt") or "")
    semantic_judge_reason = _semantic_judge_reason(details)
    return _CaseExplanation(
        name=str(details.get("case") or result.mode.rsplit(":", 1)[-1]),
        mode=result.mode,
        success=result.success,
        blocked=bool(details.get("blocked")),
        required_text=required_text,
        text_excerpt_present=bool(text_excerpt),
        literal_required_text_matched=_literal_required_text_matched(
            required_text=required_text,
            text_excerpt=text_excerpt,
            semantic_judge_reason=semantic_judge_reason,
        ),
        semantic_judge_used=(
            bool(details.get("semantic_judge_used"))
            or semantic_judge_reason == "semantic_judge_completed"
        ),
        semantic_judge_reason=semantic_judge_reason or None,
        semantic_judge_summary=_semantic_judge_summary(details),
    )


def _required_text_from_details(value: object) -> list[str]:
    if isinstance(value, list):
        required_text: list[str] = []
        for item in value:
            if item is None:
                continue
            text = str(item)
            if text:
                required_text.append(text)
        return required_text
    if isinstance(value, str) and value:
        return [value]
    return []


def _semantic_judge_reason(details: dict[str, object]) -> str:
    reason = details.get("semantic_judge_reason")
    return reason if isinstance(reason, str) else ""


def _literal_required_text_matched(
    *,
    required_text: list[str],
    text_excerpt: str,
    semantic_judge_reason: str,
) -> bool:
    if semantic_judge_reason == "literal_required_text_matched":
        return True
    if semantic_judge_reason == "semantic_judge_completed":
        return False
    return bool(required_text) and required_text_matches(text_excerpt, required_text)


def _semantic_judge_summary(details: dict[str, object]) -> dict[str, object] | None:
    semantic_judge = details.get("semantic_judge")
    if not isinstance(semantic_judge, dict):
        return None
    return {
        "completed": semantic_judge.get("completed"),
        "confidence": semantic_judge.get("confidence"),
        "reason": semantic_judge.get("reason"),
        "enabled": semantic_judge.get("enabled"),
    }
