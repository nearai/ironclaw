"""Safety layer — mirrors crates/ironclaw_safety/.

Provides prompt-injection detection, content sanitization, and output
length enforcement.  The policy engine maps detection results to actions:
Block, Warn, Sanitize.
"""

from __future__ import annotations

import html
import re
from dataclasses import dataclass, field
from enum import Enum
from typing import NamedTuple


# ---------------------------------------------------------------------------
# Injection patterns (a small subset of the Rust implementation's list)
# ---------------------------------------------------------------------------

_INJECTION_PATTERNS: list[re.Pattern[str]] = [
    re.compile(p, re.IGNORECASE)
    for p in [
        r"ignore (all )?previous instructions?",
        r"disregard (your|all|the) (previous |prior )?instructions?",
        r"you are now",
        r"new persona",
        r"act as (a|an|the)\b",
        r"forget (your|all|everything|previous)",
        r"system prompt",
        r"<\|im_start\|>",
        r"<\|im_end\|>",
        r"\[INST\]",
        r"\[/INST\]",
        r"### (Human|Assistant|System):",
        r"jailbreak",
        r"DAN mode",
        r"developer mode",
    ]
]


class PolicyAction(Enum):
    """Action to take when a safety rule fires."""

    Allow = "allow"
    Warn = "warn"
    Sanitize = "sanitize"
    Block = "block"


@dataclass
class SafetyViolation:
    """A detected safety issue."""

    pattern: str
    action: PolicyAction
    sanitized: str | None = None


class SafetyResult(NamedTuple):
    """Result of a safety check."""

    safe: bool
    violations: list[SafetyViolation]
    sanitized_content: str


class SafetyLayer:
    """
    Multi-layer safety enforcement.

    Pipeline:
        raw input → injection detection → sanitization → length enforcement → safe output

    Mirrors the Rust ``SafetyLayer`` in ``crates/ironclaw_safety/``.
    """

    def __init__(
        self,
        injection_check_enabled: bool = True,
        max_output_length: int = 100_000,
    ) -> None:
        self.injection_check_enabled = injection_check_enabled
        self.max_output_length = max_output_length

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def check_input(self, content: str) -> SafetyResult:
        """Check and sanitize user or tool input before passing to the LLM."""
        violations: list[SafetyViolation] = []
        current = content

        if self.injection_check_enabled:
            for pattern in _INJECTION_PATTERNS:
                if pattern.search(current):
                    violation = SafetyViolation(
                        pattern=pattern.pattern,
                        action=PolicyAction.Block,
                    )
                    violations.append(violation)

        # If any blocking violation found, reject
        blocked = any(v.action == PolicyAction.Block for v in violations)
        return SafetyResult(
            safe=not blocked,
            violations=violations,
            sanitized_content=current,
        )

    def sanitize_tool_output(self, output: str) -> str:
        """
        Sanitize tool output before injecting into the LLM context.

        Wraps the output in a safe boundary marker so the LLM cannot
        interpret it as instructions.
        """
        truncated = output[: self.max_output_length]
        # Escape any HTML/XML-like tags that might confuse the model
        safe = html.escape(truncated, quote=False)
        return f"<tool_output>\n{safe}\n</tool_output>"

    def check_output(self, content: str) -> SafetyResult:
        """Check agent output before sending to the user."""
        violations: list[SafetyViolation] = []

        # Length enforcement (warn only — truncation is handled by compaction)
        if len(content) > self.max_output_length:
            violations.append(
                SafetyViolation(
                    pattern="max_output_length",
                    action=PolicyAction.Warn,
                )
            )

        return SafetyResult(
            safe=True,
            violations=violations,
            sanitized_content=content,
        )

    def scan_for_secrets(self, content: str, secrets: list[str]) -> bool:
        """Return True if any secret appears verbatim in content (leak detection)."""
        return any(secret in content for secret in secrets if secret)
