#!/usr/bin/env python3
"""Regression tests for workflow-canary tool-install evidence."""

from __future__ import annotations

import sys
import types
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.modules.setdefault("httpx", types.ModuleType("httpx"))

from scripts.workflow_canary.scenarios.tool_install_chat import (
    _tool_install_evidence,
)


class ToolInstallEvidenceTests(unittest.TestCase):
    def test_accepts_target_bound_arguments_when_history_exposes_them(self) -> None:
        history = {
            "turns": [
                {
                    "tool_calls": [
                        {
                            "name": "tool_install",
                            "arguments": {"name": "gmail"},
                        }
                    ]
                }
            ]
        }

        self.assertEqual(
            _tool_install_evidence(
                history,
                "gmail",
                target_absent_before_chat=True,
                target_registered_after_chat=True,
            ),
            "target_bound_history",
        )

    def test_accepts_redacted_completed_call_with_clean_slate_readback(self) -> None:
        history = {
            "turns": [
                {
                    "tool_calls": [
                        {
                            "name": "tool_install",
                            "has_result": True,
                            "has_error": False,
                        }
                    ]
                }
            ]
        }

        self.assertEqual(
            _tool_install_evidence(
                history,
                "gmail",
                target_absent_before_chat=True,
                target_registered_after_chat=True,
            ),
            "clean_slate_readback",
        )

    def test_rejects_redacted_call_without_clean_slate(self) -> None:
        history = {
            "turns": [
                {
                    "tool_calls": [
                        {
                            "name": "tool_install",
                            "has_result": True,
                            "has_error": False,
                        }
                    ]
                }
            ]
        }

        self.assertIsNone(
            _tool_install_evidence(
                history,
                "gmail",
                target_absent_before_chat=False,
                target_registered_after_chat=True,
            )
        )

    def test_accepts_error_summary_when_clean_slate_readback_succeeded(self) -> None:
        history = {
            "turns": [
                {
                    "tool_calls": [
                        {
                            "name": "tool_install",
                            "has_result": False,
                            "has_error": True,
                        }
                    ]
                }
            ]
        }

        self.assertEqual(
            _tool_install_evidence(
                history,
                "gmail",
                target_absent_before_chat=True,
                target_registered_after_chat=True,
            ),
            "clean_slate_readback",
        )

    def test_rejects_unrelated_call(self) -> None:
        history = {
            "turns": [
                {
                    "tool_calls": [
                        {
                            "name": "tool_activate",
                            "has_result": True,
                            "has_error": False,
                        }
                    ]
                }
            ]
        }

        self.assertIsNone(
            _tool_install_evidence(
                history,
                "gmail",
                target_absent_before_chat=True,
                target_registered_after_chat=True,
            )
        )


if __name__ == "__main__":
    unittest.main()
