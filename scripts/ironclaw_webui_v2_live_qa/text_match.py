"""Required-text matching for IronClaw WebUI v2 live QA responses."""

from __future__ import annotations

import re


def required_text_matches(text: str, required_text: list[str]) -> bool:
    normalized_text = text.lower()
    return all(
        any(_required_option_matches(normalized_text, option) for option in piece.split("|"))
        for piece in required_text
    )


def _required_option_matches(normalized_text: str, option: str) -> bool:
    normalized_option = option.strip().lower()
    if not normalized_option:
        return False
    if re.fullmatch(r"\w+", normalized_option):
        return re.search(rf"\b{re.escape(normalized_option)}\b", normalized_text) is not None
    return normalized_option in normalized_text
