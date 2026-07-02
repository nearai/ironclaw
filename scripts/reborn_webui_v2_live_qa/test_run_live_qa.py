#!/usr/bin/env python3
"""Unit tests for the Reborn WebUI v2 live QA runner helpers.

Run with::

    python3 scripts/reborn_webui_v2_live_qa/test_run_live_qa.py
"""

from __future__ import annotations

import argparse
import asyncio
import importlib.util
import json
import os
import re
import sqlite3
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

if __package__:
    from . import run_live_qa
    from . import semantic_judge
else:
    import run_live_qa
    import semantic_judge


class RebornWebUiV2LiveQaRunnerTests(unittest.TestCase):
    def _dummy_ctx(self) -> run_live_qa.LiveQaContext:
        return run_live_qa.LiveQaContext(
            base_url="http://127.0.0.1:9",
            output_dir=Path("/tmp"),
            reborn_home=Path("/tmp/reborn-home"),
            env={},
        )

    def _fake_assistant_reply_page(self, response_text: str):
        class FakeApprove:
            @property
            def last(self):
                return self

            async def is_visible(self, **_kwargs):
                return False

        class FakeAssistantBlocks:
            @property
            def last(self):
                return self

            async def count(self):
                return 1

            async def inner_text(self, **_kwargs):
                return response_text

            async def all_inner_texts(self):
                return [response_text]

        class FakeMain:
            async def inner_text(self, **_kwargs):
                return response_text

        class FakePage:
            def locator(self, selector):
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantBlocks()
                if selector == "main":
                    return FakeMain()
                raise AssertionError(f"unexpected selector: {selector}")

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        return FakePage()

    def test_dismiss_visible_connect_action_clicks_only_visible_card(self):
        class FakeDismiss:
            def __init__(self, *, count: int, visible: bool) -> None:
                self._count = count
                self._visible = visible
                self.clicked = False

            @property
            def first(self):
                return self

            async def count(self):
                return self._count

            async def is_visible(self):
                return self._visible

            async def click(self):
                self.clicked = True

        class FakePage:
            def __init__(self, dismiss: FakeDismiss) -> None:
                self.dismiss = dismiss

            def locator(self, selector: str):
                self.selector = selector
                return self.dismiss

        visible = FakeDismiss(count=1, visible=True)
        visible_result = asyncio.run(
            run_live_qa._dismiss_visible_connect_action(FakePage(visible))
        )
        self.assertTrue(visible_result)
        self.assertTrue(visible.clicked)

        hidden = FakeDismiss(count=1, visible=False)
        hidden_result = asyncio.run(
            run_live_qa._dismiss_visible_connect_action(FakePage(hidden))
        )
        self.assertFalse(hidden_result)
        self.assertFalse(hidden.clicked)

        absent = FakeDismiss(count=0, visible=True)
        absent_result = asyncio.run(
            run_live_qa._dismiss_visible_connect_action(FakePage(absent))
        )
        self.assertFalse(absent_result)
        self.assertFalse(absent.clicked)

    def test_slack_connect_case_uses_extensions_channels_surface(self):
        class FakePage:
            def __init__(self) -> None:
                self.gotos: list[tuple[str, str | None]] = []

            async def goto(self, url: str, wait_until: str | None = None) -> None:
                self.gotos.append((url, wait_until))

            def locator(self, selector: str) -> str:
                return selector

        class FakeExpectation:
            def __init__(self, selector: str) -> None:
                self.selector = selector

            async def to_contain_text(
                self,
                text: str,
                timeout: int | None = None,
            ) -> None:
                expected_texts.append((self.selector, text, timeout))

        def fake_expect(selector: str) -> FakeExpectation:
            return FakeExpectation(selector)

        async def fake_with_page(
            _output_dir: Path,
            _case_name: str,
            action,
        ) -> None:
            await action(fake_page)

        async def fake_fetch_webui_json(_page: object, path: str) -> dict[str, object]:
            fetched_paths.append(path)
            return {
                "channels": [
                    {
                        "channel": "slack",
                        "display_name": "Slack",
                        "strategy": "admin_managed_channels",
                        "action": {"title": "Choose Slack channel"},
                    },
                    {
                        "channel": "slack",
                        "display_name": "Slack",
                        "strategy": "inbound_proof_code",
                        "action": {
                            "title": "Slack account connection",
                            "instructions": "Message the Slack app, then enter the code here.",
                        },
                    },
                ]
            }

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            (output_dir / "preflight.json").write_text(
                json.dumps(
                    {
                        "checks": {
                            "slack": {
                                "enabled_in_config": True,
                                "env_present": True,
                                "auth_test": {
                                    "ok": True,
                                    "team_id": "T123",
                                    "user_id": "U123",
                                },
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            fake_page = FakePage()
            fetched_paths: list[str] = []
            expected_texts: list[tuple[str, str, int | None]] = []
            playwright_module = types.ModuleType("playwright")
            playwright_async_api = types.ModuleType("playwright.async_api")
            playwright_async_api.expect = fake_expect
            ctx = run_live_qa.LiveQaContext(
                base_url="http://127.0.0.1:3000",
                output_dir=output_dir,
                reborn_home=output_dir / "reborn-home",
                env={},
            )

            with (
                patch.dict(
                    sys.modules,
                    {
                        "playwright": playwright_module,
                        "playwright.async_api": playwright_async_api,
                    },
                ),
                patch.object(run_live_qa, "_with_page", new=fake_with_page),
                patch.object(
                    run_live_qa,
                    "_fetch_webui_json",
                    new=fake_fetch_webui_json,
                ),
            ):
                result = asyncio.run(
                    run_live_qa._slack_connect_case(
                        ctx,
                        case_name="qa_3a_slack_connect",
                    )
                )

        self.assertTrue(result.success, result.details)
        self.assertEqual(
            fake_page.gotos,
            [
                (
                    "http://127.0.0.1:3000/v2/extensions/channels?"
                    f"token={run_live_qa.AUTH_TOKEN}",
                    "domcontentloaded",
                )
            ],
        )
        self.assertEqual(
            fetched_paths,
            ["/api/webchat/v2/channels/connectable"],
        )
        observed_expectations = [text for _selector, text, _timeout in expected_texts]
        self.assertIn("Channels", observed_expectations)
        self.assertIn("Slack account connection", observed_expectations)
        self.assertIn("Message the Slack app", observed_expectations)
        self.assertNotIn("Connect Slack", observed_expectations)
        self.assertFalse(any("/v2/chat" in url for url, _wait in fake_page.gotos))
        self.assertEqual(
            result.details["slack_connect_surface"],
            "/v2/extensions/channels",
        )
        self.assertEqual(
            result.details["slack_connect_title"],
            "Slack account connection",
        )

    def test_product_connect_cases_start_from_chat_then_verify_registry(self):
        captured_chat: dict[str, dict[str, object]] = {}
        captured_registry: dict[str, dict[str, object]] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            case_name = kwargs["case_name"]
            captured_chat[case_name] = kwargs
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{case_name}",
                success=True,
                latency_ms=1,
                details={"text_excerpt": f"{kwargs['marker']} connected"},
            )

        async def fake_extension_authenticated_case(_ctx, **kwargs):
            case_name = kwargs["case_name"]
            captured_registry[case_name] = kwargs
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{case_name}",
                success=True,
                latency_ms=1,
                details={
                    "package_id": kwargs["package_id"],
                    "ensure_installed": kwargs["ensure_installed"],
                },
            )

        def fake_capability_run_statuses(_reborn_home, capability_ids):
            return {capability_id: ["completed"] for capability_id in capability_ids}

        cases = {
            "qa_2a_gmail_connect": (
                run_live_qa.case_qa_2a_gmail_connect,
                "gmail",
                ["gmail.list_messages"],
            ),
            "qa_2b_calendar_connect": (
                run_live_qa.case_qa_2b_calendar_connect,
                "google-calendar",
                ["google-calendar.list_events"],
            ),
            "qa_2c_drive_connect": (
                run_live_qa.case_qa_2c_drive_connect,
                "google-drive",
                ["google-drive.list_files"],
            ),
            "qa_4a_gmail_connect": (
                run_live_qa.case_qa_4a_gmail_connect,
                "gmail",
                ["gmail.list_messages"],
            ),
            "qa_4b_github_connect": (
                run_live_qa.case_qa_4b_github_connect,
                "github",
                ["github.get_authenticated_user"],
            ),
            "qa_5b_drive_connect": (
                run_live_qa.case_qa_5b_drive_connect,
                "google-drive",
                ["google-drive.list_files"],
            ),
            "qa_6a_gmail_connect": (
                run_live_qa.case_qa_6a_gmail_connect,
                "gmail",
                ["gmail.list_messages"],
            ),
            "qa_6b_sheets_connect": (
                run_live_qa.case_qa_6b_sheets_connect,
                "google-sheets",
                [],
            ),
            "qa_7b_sheets_connect": (
                run_live_qa.case_qa_7b_sheets_connect,
                "google-sheets",
                [],
            ),
        }

        with (
            patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ),
            patch.object(
                run_live_qa,
                "_extension_authenticated_case",
                side_effect=fake_extension_authenticated_case,
            ),
            patch.object(
                run_live_qa,
                "_capability_run_statuses",
                side_effect=fake_capability_run_statuses,
            ),
        ):
            ctx = self._dummy_ctx()
            for case_name, (case_fn, _package_id, _verification_caps) in cases.items():
                with self.subTest(case=case_name):
                    self.assertTrue(asyncio.run(case_fn(ctx)).success)

        self.assertEqual(set(captured_chat), set(cases))
        self.assertEqual(set(captured_registry), set(cases))
        for case_name, (_case_fn, package_id, verification_caps) in cases.items():
            prompt = str(captured_chat[case_name]["prompt"])
            self.assertEqual(prompt, run_live_qa.QA_SHEET_PROMPTS[case_name])
            self.assertNotIn("extension_search", prompt)
            self.assertNotIn(f"`{package_id}`", prompt)
            self.assertNotIn("/v2/extensions/registry", prompt)
            self.assertIsNone(captured_chat[case_name]["marker"])
            extra_details = captured_chat[case_name]["extra_details"]
            self.assertIsInstance(extra_details, dict)
            self.assertTrue(extra_details["chat_connect_flow"])
            required_capabilities = extra_details["required_capabilities"]
            self.assertIn(run_live_qa.EXTENSION_SEARCH_CAPABILITY_ID, required_capabilities)
            self.assertIn(run_live_qa.EXTENSION_INSTALL_CAPABILITY_ID, required_capabilities)
            self.assertIn(run_live_qa.EXTENSION_ACTIVATE_CAPABILITY_ID, required_capabilities)
            for capability_id in verification_caps:
                self.assertNotIn(capability_id, required_capabilities)
            self.assertEqual(
                extra_details["verification_capabilities"],
                verification_caps,
            )
            self.assertFalse(extra_details["verification_capabilities_required"])
            self.assertFalse(captured_registry[case_name]["ensure_installed"])

    def test_product_connect_case_fails_when_chat_does_not_use_extension_lifecycle(self):
        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"text_excerpt": f"{kwargs['marker']} connected"},
            )

        def fake_capability_run_statuses(_reborn_home, capability_ids):
            return {capability_id: [] for capability_id in capability_ids}

        with (
            patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ),
            patch.object(
                run_live_qa,
                "_capability_run_statuses",
                side_effect=fake_capability_run_statuses,
            ),
        ):
            result = asyncio.run(
                run_live_qa._extension_chat_connect_case(
                    self._dummy_ctx(),
                    case_name="qa_test_connect",
                    package_id="gmail",
                    display_name="Gmail",
                    required_tools=["gmail.list_messages"],
                    marker="REBORN_QA_TEST_CONNECT_DONE",
                    verification_instruction=(
                        "After connecting, call gmail.list_messages once."
                    ),
                    verification_capabilities=["gmail.list_messages"],
                )
            )

        self.assertFalse(result.success)
        self.assertIn(
            "chat connect did not complete expected capabilities",
            str(result.details["error"]),
        )

    def test_routine_creation_case_fails_when_no_trigger_is_created(self):
        captured_prompts: list[str] = []

        async def fake_live_chat_case(_ctx, **kwargs):
            captured_prompts.append(kwargs["prompt"])
            extra_details = kwargs.get("extra_details") or {}
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={
                    "text_excerpt": "routine created",
                    **extra_details,
                },
            )

        with (
            patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ),
            patch.object(run_live_qa, "_trigger_record_count", side_effect=[0, 0]),
        ):
            result = asyncio.run(
                run_live_qa._routine_creation_case(
                    self._dummy_ctx(),
                    case_name="qa_test_routine",
                    prompt="original sheet prompt",
                    marker=None,
                    routine_name="qa-test-routine",
                    required_text=["routine"],
                )
            )

        self.assertFalse(result.success)
        self.assertEqual(captured_prompts, ["original sheet prompt"])
        self.assertEqual(result.details["trigger_records_after"], 0)
        self.assertIn("did not add a trigger_record", result.details["error"])

    def test_routine_creation_case_can_preinstall_extensions(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_with_extensions_case(
            _ctx,
            *,
            case_name,
            prompt,
            marker,
            required_text,
            extensions,
            timeout,
            extra_details,
        ):
            captured.update(
                {
                    "case_name": case_name,
                    "prompt": prompt,
                    "marker": marker,
                    "required_text": required_text,
                    "extensions": extensions,
                    "timeout": timeout,
                    "extra_details": extra_details,
                }
            )
            extra_details = extra_details or {}
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{case_name}",
                success=True,
                latency_ms=1,
                details={
                    "text_excerpt": "routine created",
                    **extra_details,
                },
            )

        with (
            patch.object(
                run_live_qa,
                "_live_chat_with_extensions_case",
                side_effect=fake_live_chat_with_extensions_case,
            ),
            patch.object(run_live_qa, "_trigger_record_count", side_effect=[0, 1]),
        ):
            result = asyncio.run(
                run_live_qa._routine_creation_case(
                    self._dummy_ctx(),
                    case_name="qa_test_routine",
                    prompt="original sheet prompt",
                    marker=None,
                    routine_name="qa-test-routine",
                    required_text=["routine"],
                    extensions=[
                        {
                            "package_id": "google-sheets",
                            "display_name": "Google Sheets",
                            "required_tools": ["google-sheets.append_values"],
                        }
                    ],
                    extra_details={"fixture_ready": True},
                )
            )

        self.assertTrue(result.success)
        self.assertEqual(captured["case_name"], "qa_test_routine")
        self.assertEqual(captured["prompt"], "original sheet prompt")
        self.assertIsNone(captured["marker"])
        self.assertEqual(captured["required_text"], ["routine"])
        self.assertEqual(captured["extensions"][0]["package_id"], "google-sheets")
        self.assertEqual(captured["timeout"], 180.0)
        self.assertTrue(result.details["fixture_ready"])

    def test_required_text_accepts_explicit_alternatives(self):
        self.assertTrue(
            run_live_qa._required_text_matches(
                "fires every 5 minutes and watches slack for bug messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertFalse(
            run_live_qa._required_text_matches(
                "records bug messages in a sheet",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertFalse(
            run_live_qa._required_text_matches(
                "fires every 5 minutes and watches slack for debug messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertFalse(
            run_live_qa._required_text_matches(
                "fires every 5 minutes and watches slack for bugfix messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertTrue(
            run_live_qa._required_text_matches(
                "fires every 5 minutes and watches slack for bug: messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertTrue(
            run_live_qa._required_text_matches(
                "https://near.ai responded with HTTP 200 - the endpoint is up and running fine.",
                ["status|http|200|up|running|responded"],
            )
        )
        self.assertTrue(
            run_live_qa._required_text_matches(
                "Trigger created. Schedule: every 5 minutes. Action: fetch latest releases.",
                ["routine|trigger|automation|cron|schedule|created"],
            )
        )
        self.assertTrue(
            run_live_qa._required_text_matches(
                "The email from firat.sertgoz@near.ai is already in the sheet.",
                ["ABC|sheet|spreadsheet", "email|row|near.ai|near ai"],
            )
        )
        self.assertTrue(
            run_live_qa._required_text_matches(
                'Discussion thread "vibe coded eh" (id=47005839) mentions NEAR AI.',
                ["news.ycombinator.com|hacker news|hn|discussion|id="],
            )
        )

    def test_wait_for_assistant_reply_matches_combined_assistant_blocks(self):
        class FakeApprove:
            @property
            def last(self):
                return self

            async def is_visible(self, **_kwargs):
                return False

        class FakeAssistantBlocks:
            @property
            def last(self):
                return self

            async def count(self):
                return 2

            async def inner_text(self, **_kwargs):
                return "latest news on that company\nEmails a concise briefing"

            async def all_inner_texts(self):
                return [
                    'The routine has been created successfully. Routine: "30-min meeting briefing"',
                    "latest news on that company\nEmails a concise briefing",
                ]

        class FakePage:
            def locator(self, selector):
                if selector != "[data-testid='msg-assistant']":
                    raise AssertionError(f"unexpected selector: {selector}")
                return FakeAssistantBlocks()

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        text = asyncio.run(
            run_live_qa._wait_for_assistant_reply(
                FakePage(),
                marker=None,
                required_text=["routine", "email|emails|gmail"],
                timeout=1.0,
            )
        )

        self.assertIn("routine", text.lower())
        self.assertIn("emails", text.lower())

    def test_wait_for_assistant_reply_uses_semantic_judge_for_text_mismatch(self):
        response_text = (
            "Schedule: Every hour\n"
            "Delivery: Slack channel CQA123\n"
            "Trigger ID: trigger-123"
        )

        captured: dict[str, object] = {}

        async def fake_judge(**kwargs):
            captured.update(kwargs)
            return {
                "completed": True,
                "confidence": 0.91,
                "reason": "The response confirms an hourly Slack scheduled trigger.",
            }

        async def fake_sleep(_seconds):
            return None

        with (
            patch.object(
                run_live_qa,
                "_judge_assistant_reply_completion",
                side_effect=fake_judge,
            ),
            patch.object(run_live_qa.asyncio, "sleep", side_effect=fake_sleep),
        ):
            text = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page(response_text),
                    marker=None,
                    required_text=["routine"],
                    timeout=0.001,
                    semantic_goal="Create an hourly Hacker News keyword Slack routine.",
                )
            )

        self.assertIn("Trigger ID", text)
        self.assertEqual(captured["required_text"], ["routine"])
        self.assertIn("hourly Hacker News", captured["semantic_goal"])

    def test_wait_for_assistant_reply_does_not_judge_missing_marker(self):
        response_text = "The routine has been created. Trigger ID: trigger-123"

        async def fail_if_called(**_kwargs):
            raise AssertionError("semantic judge should not run when marker is missing")

        async def fake_sleep(_seconds):
            return None

        with (
            patch.object(
                run_live_qa,
                "_judge_assistant_reply_completion",
                side_effect=fail_if_called,
            ),
            patch.object(run_live_qa.asyncio, "sleep", side_effect=fake_sleep),
        ):
            with self.assertRaisesRegex(AssertionError, "marker='REBORN_QA_DONE'"):
                asyncio.run(
                    run_live_qa._wait_for_assistant_reply(
                        self._fake_assistant_reply_page(response_text),
                        marker="REBORN_QA_DONE",
                        required_text=["routine"],
                        timeout=0.001,
                    )
                )

    def test_semantic_judge_passed_respects_confidence_threshold(self):
        with patch.dict(
            os.environ,
            {"REBORN_WEBUI_V2_LIVE_QA_LLM_JUDGE_MIN_CONFIDENCE": "0.9"},
        ):
            self.assertTrue(
                semantic_judge._semantic_judge_passed(
                    {"completed": True, "confidence": 0.9}
                )
            )
            self.assertFalse(
                semantic_judge._semantic_judge_passed(
                    {"completed": True, "confidence": 0.89}
                )
            )
            self.assertFalse(
                semantic_judge._semantic_judge_passed(
                    {"completed": False, "confidence": 1.0}
                )
            )

    def test_semantic_judge_completion_content_handles_unexpected_shapes(self):
        self.assertEqual(semantic_judge._completion_content(None), "")
        self.assertEqual(semantic_judge._completion_content({"choices": "bad"}), "")
        self.assertEqual(semantic_judge._completion_content({"choices": ["bad"]}), "")
        self.assertEqual(
            semantic_judge._completion_content({"choices": [{"message": "bad"}]}),
            "",
        )
        self.assertEqual(
            semantic_judge._completion_content(
                {"choices": [{"message": {"content": '{"completed": true}'}}]}
            ),
            '{"completed": true}',
        )

    def test_semantic_judge_json_parser_handles_non_string_inputs(self):
        self.assertIsNone(semantic_judge._parse_json_object(None))
        self.assertIsNone(semantic_judge._parse_json_object(123))
        self.assertEqual(
            semantic_judge._parse_json_object('prefix {"completed": true} suffix'),
            {"completed": True},
        )

    def test_slack_delivery_target_dm_detection(self):
        self.assertTrue(run_live_qa._slack_delivery_target_is_dm("D12345"))
        self.assertFalse(run_live_qa._slack_delivery_target_is_dm("C12345"))
        self.assertFalse(run_live_qa._slack_delivery_target_is_dm(None))

    def test_slack_dm_route_discovery_prefers_configured_user(self):
        captured: dict[str, object] = {}

        class FakeResponse:
            def json(self):
                return {
                    "ok": True,
                    "channel": {
                        "id": "DQAUSER",
                        "is_im": True,
                    },
                }

        def fake_post(_url, **kwargs):
            captured.update(kwargs)
            return FakeResponse()

        fake_httpx = types.SimpleNamespace(post=fake_post)
        with (
            patch.dict(
                os.environ,
                {"REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID": "UQAUSER"},
                clear=True,
            ),
            patch.dict(sys.modules, {"httpx": fake_httpx}),
        ):
            result = run_live_qa._discover_slack_dm_route_channel(
                "[slack]\nbot_token_env = \"SLACK_BOT_TOKEN\"\n",
                {"SLACK_BOT_TOKEN": "xoxb-test"},
            )

        self.assertTrue(result["ok"])
        self.assertEqual(result["dm_user_id"], "UQAUSER")
        self.assertEqual(result["dm_user_source"], "env")
        self.assertEqual(captured["data"]["users"], "UQAUSER")

    def test_slack_dm_route_discovery_ignores_synthetic_inbound_user(self):
        captured: dict[str, object] = {}

        class FakeResponse:
            def json(self):
                return {
                    "ok": True,
                    "channel": {
                        "id": "DSLACKBOT",
                        "is_im": True,
                    },
                }

        def fake_post(_url, **kwargs):
            captured.update(kwargs)
            return FakeResponse()

        fake_httpx = types.SimpleNamespace(post=fake_post)
        with (
            patch.dict(
                os.environ,
                {"REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID": "U0REBORNQA"},
                clear=True,
            ),
            patch.dict(sys.modules, {"httpx": fake_httpx}),
        ):
            result = run_live_qa._discover_slack_dm_route_channel(
                "[slack]\nbot_token_env = \"SLACK_BOT_TOKEN\"\n",
                {"SLACK_BOT_TOKEN": "xoxb-test"},
            )

        self.assertTrue(result["ok"])
        self.assertEqual(result["dm_user_id"], "USLACKBOT")
        self.assertEqual(result["dm_user_source"], "fallback_slackbot")
        self.assertEqual(captured["data"]["users"], "USLACKBOT")

    def test_qa_7a_requires_dm_delivery_target(self):
        with (
            patch.object(
                run_live_qa,
                "_slack_preflight",
                return_value={
                    "delivery_target_present": True,
                    "route_configured_from_env": True,
                },
            ),
            patch.object(run_live_qa, "_slack_delivery_channel_id", return_value="C12345"),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_7a_slack_product_channel_connect(self._dummy_ctx())
            )

        self.assertFalse(result.success)
        self.assertIn("must be a DM", str(result.details["error"]))
        self.assertEqual(result.details["slack_delivery_target_kind"], "non_dm")

    def test_qa_7a_connect_capabilities_match_chat_connect_flow(self):
        self.assertEqual(
            run_live_qa.QA_7A_CHAT_CONNECT_CAPABILITY_IDS,
            [
                run_live_qa.EXTENSION_SEARCH_CAPABILITY_ID,
                run_live_qa.EXTENSION_INSTALL_CAPABILITY_ID,
                run_live_qa.EXTENSION_ACTIVATE_CAPABILITY_ID,
            ],
        )
        self.assertNotIn(
            run_live_qa.OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
            run_live_qa.QA_7A_CHAT_CONNECT_CAPABILITY_IDS,
        )

    def test_qa_7a_requires_new_connect_capability_completions(self):
        class FakeLocator:
            @property
            def last(self):
                return self

            async def fill(self, _text):
                return None

            async def press(self, _key):
                return None

        class FakePage:
            async def goto(self, _url, **_kwargs):
                return None

            def locator(self, _selector):
                return FakeLocator()

        class FakeExpectation:
            async def to_be_visible(self, **_kwargs):
                return None

            async def to_contain_text(self, _text, **_kwargs):
                return None

        capability_ids = run_live_qa.QA_7A_CHAT_CONNECT_CAPABILITY_IDS
        baseline = {capability_id: ["completed"] for capability_id in capability_ids}
        stale = {capability_id: ["completed"] for capability_id in capability_ids}
        fresh = {
            capability_id: ["completed", "completed"]
            for capability_id in capability_ids
        }
        status_sequence = [baseline, stale, fresh]

        async def fake_with_page(_output_dir, _case_name, action):
            await action(FakePage())

        async def fake_wait_for_assistant_reply(_page, **_kwargs):
            return "Slack is connected"

        async def fake_approve_visible_tool_gate(_page):
            return None

        async def fake_sleep(_seconds):
            return None

        def fake_capability_run_statuses(_reborn_home, _capability_ids):
            return status_sequence.pop(0) if status_sequence else fresh

        with (
            patch.object(
                run_live_qa,
                "_slack_preflight",
                return_value={
                    "delivery_target_present": True,
                    "route_configured_from_env": True,
                },
            ),
            patch.object(run_live_qa, "_slack_delivery_channel_id", return_value="D12345"),
            patch.object(run_live_qa, "_with_page", side_effect=fake_with_page),
            patch.object(
                run_live_qa,
                "_wait_for_assistant_reply",
                side_effect=fake_wait_for_assistant_reply,
            ),
            patch.object(
                run_live_qa,
                "_approve_visible_tool_gate",
                side_effect=fake_approve_visible_tool_gate,
            ),
            patch.object(
                run_live_qa,
                "_capability_run_statuses",
                side_effect=fake_capability_run_statuses,
            ),
            patch.object(run_live_qa.asyncio, "sleep", side_effect=fake_sleep),
            patch("playwright.async_api.expect", return_value=FakeExpectation()),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_7a_slack_product_channel_connect(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertEqual(result.details["baseline_capability_statuses"], baseline)
        self.assertEqual(result.details["capability_statuses"], fresh)
        self.assertEqual(result.details["text_excerpt"], "Slack is connected")

    def test_completed_capability_counts_ignore_stale_completed_runs(self):
        counts = run_live_qa._completed_capability_counts(
            {
                "extension_search": ["completed", "failed", "completed"],
                "extension_install": ["running"],
                "extension_activate": [],
            }
        )

        self.assertEqual(counts["extension_search"], 2)
        self.assertEqual(counts["extension_install"], 0)
        self.assertEqual(counts["extension_activate"], 0)

    def test_qa_7c_prepares_bug_logging_sheet_before_sheet_prompt(self):
        captured_fixture: dict[str, object] = {}
        captured_routine: dict[str, object] = {}

        async def fake_create_google_spreadsheet_fixture(
            *,
            access_token,
            title,
            values,
            sheet_name="Sheet1",
        ):
            captured_fixture.update(
                {
                    "access_token": access_token,
                    "title": title,
                    "values": values,
                    "sheet_name": sheet_name,
                }
            )
            return {
                "spreadsheet_id": "sheet-123",
                "spreadsheet_url": "https://docs.google.com/spreadsheets/d/sheet-123/edit",
                "title": title,
            }

        async def fake_routine_creation_case(
            _ctx,
            *,
            case_name,
            prompt,
            marker,
            routine_name,
            required_text,
            extensions,
            extra_details,
        ):
            captured_routine.update(
                {
                    "case_name": case_name,
                    "prompt": prompt,
                    "marker": marker,
                    "routine_name": routine_name,
                    "required_text": required_text,
                    "extensions": extensions,
                    "extra_details": extra_details,
                }
            )
            extra_details = extra_details or {}
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live:qa_7c_slack_bug_logger_routine",
                success=True,
                latency_ms=1,
                details={"text_excerpt": "routine bug created", **extra_details},
            )

        with (
            patch.object(
                run_live_qa,
                "_google_runtime_access_token",
                return_value=("fresh-access-token", {"source": "test"}),
            ),
            patch.object(
                run_live_qa,
                "_create_google_spreadsheet_fixture",
                side_effect=fake_create_google_spreadsheet_fixture,
            ),
            patch.object(
                run_live_qa,
                "_routine_creation_case",
                side_effect=fake_routine_creation_case,
            ),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_7c_slack_bug_logger_routine(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertEqual(captured_fixture["access_token"], "fresh-access-token")
        self.assertEqual(
            captured_fixture["title"],
            run_live_qa.QA_7C_BUG_LOGGING_SHEET_TITLE,
        )
        self.assertEqual(captured_fixture["sheet_name"], "Sheet1")
        self.assertEqual(
            captured_fixture["values"],
            [["Summary", "Reporter", "Slack Timestamp", "Status", "QA Marker"]],
        )
        self.assertEqual(captured_routine["case_name"], "qa_7c_slack_bug_logger_routine")
        self.assertIsNone(captured_routine["marker"])
        self.assertEqual(captured_routine["routine_name"], "reborn-qa-7c-slack-bug-sheet")
        self.assertEqual(
            captured_routine["required_text"],
            ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
        )
        self.assertEqual(
            captured_routine["prompt"],
            run_live_qa._qa_sheet_prompt("qa_7c_slack_bug_logger_routine"),
        )
        package_ids = [
            extension["package_id"] for extension in captured_routine["extensions"]
        ]
        self.assertEqual(package_ids, ["slack", "google-drive", "google-sheets"])
        self.assertEqual(
            captured_routine["extra_details"]["bug_log_sheet_fixture"]["spreadsheet_id"],
            "sheet-123",
        )

    def test_qa_7d_accepts_signed_slack_event_into_reborn_run(self):
        captured_event: dict[str, object] = {}

        async def fake_post_signed_slack_dm_event(_ctx, **kwargs):
            captured_event.update(kwargs)
            return {
                "status_code": 200,
                "event_id": kwargs["event_id"],
                "channel_id_present": True,
            }

        async def fake_wait_for_slack_event_run_id(_ctx, **kwargs):
            return f"run-for-{kwargs['event_id']}"

        with (
            patch.object(
                run_live_qa,
                "_slack_preflight",
                return_value={
                    "legacy_actor_configured": True,
                    "legacy_actor_user_id": "U0REBORNQA",
                    "delivery_target_present": True,
                },
            ),
            patch.object(run_live_qa, "_slack_delivery_channel_id", return_value="D12345"),
            patch.object(
                run_live_qa,
                "_post_signed_slack_dm_event",
                side_effect=fake_post_signed_slack_dm_event,
            ),
            patch.object(
                run_live_qa,
                "_wait_for_slack_event_run_id",
                side_effect=fake_wait_for_slack_event_run_id,
            ),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_7d_slack_bug_message_trigger(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertTrue(result.details["accepted_run_id"].startswith("run-for-"))
        self.assertEqual(result.details["signed_event"]["status_code"], 200)
        self.assertTrue(
            str(captured_event["text"]).startswith("bug: reborn QA bug logger smoke ")
        )
        self.assertNotIn("In Slack", str(captured_event["text"]))
        self.assertEqual(result.details["slack_event_text"], captured_event["text"])
        self.assertEqual(
            result.details["qa_sheet_prompt"],
            run_live_qa._qa_sheet_prompt("qa_7d_slack_bug_message_trigger"),
        )

    def test_endpoint_status_routine_prompt_uses_real_endpoint(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured.update(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"text_excerpt": "Routine created"},
            )

        with (
            patch.object(run_live_qa, "_live_chat_case", side_effect=fake_live_chat_case),
            patch.object(run_live_qa, "_trigger_record_count", side_effect=[0, 1]),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_3c_endpoint_status_slack_routine(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        prompt = str(captured["prompt"])
        self.assertNotIn("[endpoint URL]", prompt)
        self.assertIn(run_live_qa.ENDPOINT_STATUS_URL, prompt)
        self.assertIsNone(captured["marker"])
        self.assertEqual(captured["required_text"], ["routine"])

    def test_hn_live_chat_accepts_hacker_news_url_host(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured.update(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"text_excerpt": "news.ycombinator.com/item?id=1"},
            )

        with patch.object(run_live_qa, "_live_chat_case", side_effect=fake_live_chat_case):
            result = asyncio.run(
                run_live_qa.case_qa_8b_hn_keyword_live_chat(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertEqual(
            captured["required_text"],
            ["news.ycombinator.com|hacker news|hn|discussion|id="],
        )

    def test_live_google_side_effect_cases_install_required_extensions(self):
        captured: dict[str, dict[str, object]] = {}
        spreadsheet_id = "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890"

        async def fake_live_chat_with_extensions_case(_ctx, **kwargs):
            case_name = kwargs["case_name"]
            captured[case_name] = kwargs
            details = {
                "text_excerpt": (
                    f"Created https://docs.google.com/spreadsheets/d/{spreadsheet_id}/edit"
                )
            }
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{case_name}",
                success=True,
                latency_ms=1,
                details=details,
            )

        async def fake_gmail_delivery_target_email(**_kwargs):
            return "qa@example.test"

        async def fake_gmail_profile_email(**_kwargs):
            return "sender@example.test"

        async def fake_live_github_latest_release(*_args, **_kwargs):
            return {
                "api_url": "https://api.github.test/repos/nearai/ironclaw/releases/latest",
                "tag_name": "ironclaw-v0.test",
            }

        async def fake_wait_for_gmail_marker(**_kwargs):
            return {"found": True}

        async def fake_google_sheet_contains_marker(**_kwargs):
            return {"found": True}

        with (
            patch.object(
                run_live_qa,
                "_live_chat_with_extensions_case",
                side_effect=fake_live_chat_with_extensions_case,
            ),
            patch.object(
                run_live_qa,
                "_google_runtime_access_token",
                return_value=("fresh-access-token", {"source": "test"}),
            ),
            patch.object(
                run_live_qa,
                "_gmail_delivery_target_email",
                side_effect=fake_gmail_delivery_target_email,
            ),
            patch.object(
                run_live_qa,
                "_gmail_profile_email",
                side_effect=fake_gmail_profile_email,
            ),
            patch.object(
                run_live_qa,
                "_live_github_latest_release",
                side_effect=fake_live_github_latest_release,
            ),
            patch.object(
                run_live_qa,
                "_wait_for_gmail_marker",
                side_effect=fake_wait_for_gmail_marker,
            ),
            patch.object(
                run_live_qa,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
        ):
            ctx = self._dummy_ctx()
            self.assertTrue(
                asyncio.run(run_live_qa.case_qa_2f_calendar_prep_email_delivery(ctx)).success
            )
            self.assertTrue(
                asyncio.run(run_live_qa.case_qa_4e_github_release_email_delivery(ctx)).success
            )
            self.assertTrue(
                asyncio.run(run_live_qa.case_qa_6c_gmail_to_sheet_live_chat(ctx)).success
            )
            self.assertTrue(
                asyncio.run(run_live_qa.case_qa_6e_gmail_to_sheet_delivery(ctx)).success
            )

        extensions_by_case = {
            case: {extension["package_id"]: extension for extension in kwargs["extensions"]}
            for case, kwargs in captured.items()
        }
        prompt_2f = str(captured["qa_2f_calendar_prep_email_delivery"]["prompt"])
        self.assertIn("not `message.raw`", prompt_2f)
        self.assertIn('"from":"sender@example.test"', prompt_2f)
        self.assertIn('"to":"qa@example.test"', prompt_2f)
        self.assertIn('"body":"REBORN_QA_2F_CALENDAR_PREP_EMAIL_DELIVERED_', prompt_2f)

        prompt_4e = str(captured["qa_4e_github_release_email_delivery"]["prompt"])
        self.assertIn("not `message.raw`", prompt_4e)
        self.assertIn('"from":"sender@example.test"', prompt_4e)
        self.assertIn('"to":"qa@example.test"', prompt_4e)
        self.assertIn('"body":"REBORN_QA_4E_GITHUB_RELEASE_EMAIL_DELIVERED_', prompt_4e)
        self.assertIn("ironclaw-v0.test", prompt_4e)

        self.assertEqual(
            captured["qa_6e_gmail_to_sheet_delivery"]["required_text"],
            ["Google Sheet"],
        )
        self.assertEqual(
            captured["qa_6c_gmail_to_sheet_live_chat"]["required_text"],
            ["ABC|sheet|spreadsheet", "email|row|near.ai|near ai"],
        )
        self.assertTrue(
            extensions_by_case["qa_2f_calendar_prep_email_delivery"]["google-docs"].get(
                "ensure_installed",
                True,
            )
        )
        self.assertTrue(
            extensions_by_case["qa_2f_calendar_prep_email_delivery"]["web-access"].get(
                "ensure_installed",
                True,
            )
        )
        self.assertTrue(
            extensions_by_case["qa_6c_gmail_to_sheet_live_chat"]["gmail"].get(
                "ensure_installed",
                True,
            )
        )
        self.assertEqual(
            extensions_by_case["qa_6c_gmail_to_sheet_live_chat"]["google-drive"].get(
                "required_tools",
            ),
            ["google-drive.list_files"],
        )
        self.assertTrue(
            extensions_by_case["qa_6e_gmail_to_sheet_delivery"]["gmail"].get(
                "ensure_installed",
                True,
            )
        )

    def test_gmail_to_sheet_delivery_falls_back_to_drive_name_lookup(self):
        spreadsheet_id = "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890"
        captured_lookup: dict[str, object] = {}

        async def fake_live_chat_with_extensions_case(_ctx, **kwargs):
            marker = kwargs["marker"]
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live:qa_6e_gmail_to_sheet_delivery",
                success=True,
                latency_ms=1,
                details={
                    "marker": marker,
                    "text_excerpt": f"Google Sheet created for {marker}",
                },
            )

        async def fake_google_drive_file_id_by_name(**kwargs):
            captured_lookup.update(kwargs)
            return spreadsheet_id

        async def fake_google_sheet_contains_marker(**kwargs):
            self.assertEqual(kwargs["spreadsheet_id"], spreadsheet_id)
            self.assertEqual(kwargs["marker"], captured_lookup["name"])
            return {"found": True}

        with (
            patch.object(
                run_live_qa,
                "_live_chat_with_extensions_case",
                side_effect=fake_live_chat_with_extensions_case,
            ),
            patch.object(
                run_live_qa,
                "_google_runtime_access_token",
                return_value=("fresh-access-token", {"source": "test"}),
            ),
            patch.object(
                run_live_qa,
                "_google_drive_file_id_by_name",
                side_effect=fake_google_drive_file_id_by_name,
            ),
            patch.object(
                run_live_qa,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_6e_gmail_to_sheet_delivery(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertEqual(result.details["spreadsheet_id"], spreadsheet_id)
        self.assertEqual(result.details["spreadsheet_id_source"], "drive_name_lookup")
        self.assertEqual(
            captured_lookup["mime_type"],
            "application/vnd.google-apps.spreadsheet",
        )

    def test_slack_side_effect_setup_prompts_avoid_connect_action_trigger(self):
        captured_prompts: dict[str, str] = {}
        captured_slack_required_text: list[str] = []
        document_id = "1DocCdEfGhIjKlMnOpQrStUvWxYz_1234567890"
        spreadsheet_id = "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890"

        async def fake_live_chat_with_extensions_case(_ctx, **kwargs):
            case_name = kwargs["case_name"]
            captured_prompts[case_name] = kwargs["prompt"]
            file_url = (
                f"https://docs.google.com/document/d/{document_id}/edit"
                if case_name == "qa_5d_slack_strategy_doc_answer"
                else f"https://docs.google.com/spreadsheets/d/{spreadsheet_id}/edit"
            )
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{case_name}",
                success=True,
                latency_ms=1,
                details={"text_excerpt": f"Created {file_url}"},
            )

        async def fake_post_signed_slack_dm_event(*_args, **_kwargs):
            return {"ok": True}

        async def fake_slack_history_contains_marker(*_args, **kwargs):
            captured_slack_required_text.extend(kwargs["required_text"])
            return {"found": True}

        async def fake_wait_for_google_sheet_marker(*_args, **_kwargs):
            return {"found": True}

        with (
            patch.object(
                run_live_qa,
                "_live_chat_with_extensions_case",
                side_effect=fake_live_chat_with_extensions_case,
            ),
            patch.object(
                run_live_qa,
                "_slack_preflight",
                return_value={"legacy_actor_user_id": "U0REBORNQA"},
            ),
            patch.object(
                run_live_qa,
                "_slack_delivery_channel_id",
                return_value="D0REBORNQA",
            ),
            patch.object(
                run_live_qa,
                "_post_signed_slack_dm_event",
                side_effect=fake_post_signed_slack_dm_event,
            ),
            patch.object(
                run_live_qa,
                "_slack_history_contains_marker",
                side_effect=fake_slack_history_contains_marker,
            ),
            patch.object(
                run_live_qa,
                "_google_runtime_access_token",
                return_value=("fresh-access-token", {"source": "test"}),
            ),
            patch.object(
                run_live_qa,
                "_wait_for_google_sheet_marker_after_slack_event",
                side_effect=fake_wait_for_google_sheet_marker,
            ),
        ):
            ctx = self._dummy_ctx()
            self.assertTrue(
                asyncio.run(run_live_qa.case_qa_5d_slack_strategy_doc_answer(ctx)).success
            )
            self.assertTrue(
                asyncio.run(run_live_qa.case_qa_7e_slack_bug_sheet_delivery(ctx)).success
            )

        trigger = re.compile(r"(^|\s)(connect|link|pair|setup|set up)(\s|$)")
        for case_name in (
            "qa_5d_slack_strategy_doc_answer",
            "qa_7e_slack_bug_sheet_delivery",
        ):
            prompt = captured_prompts[case_name].lower()
            self.assertIsNone(
                trigger.search(prompt),
                f"{case_name} prompt should not trigger WebUI connect action: {prompt}",
            )
        self.assertIn("strategy", captured_slack_required_text)
        self.assertNotIn("Google Docs", captured_slack_required_text)
        self.assertNotIn("grounding", captured_slack_required_text)
        self.assertTrue(
            any(text.startswith("QA5D-NONCE-") for text in captured_slack_required_text)
        )

    def test_signed_slack_event_cases_configure_legacy_actor(self):
        for case_name in (
            "qa_5d_slack_strategy_doc_answer",
            "qa_7d_slack_bug_message_trigger",
            "qa_7e_slack_bug_sheet_delivery",
        ):
            with self.subTest(case=case_name), tempfile.TemporaryDirectory() as tmpdir:
                config_path = Path(tmpdir) / "config.toml"
                config_path.write_text("[slack]\n", encoding="utf-8")
                with patch.dict(
                    os.environ,
                    {"REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID": "U0REBORNQA"},
                    clear=False,
                ):
                    changed, user_id = run_live_qa._configure_slack_legacy_actor_if_needed(
                        config_path,
                        [case_name],
                    )

                self.assertTrue(changed)
                self.assertEqual(user_id, "U0REBORNQA")
                self.assertIn(
                    'slack_user_id = "U0REBORNQA"',
                    config_path.read_text(encoding="utf-8"),
                )

    def test_slack_route_append_matches_exact_user_channel_pair(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = Path(tmpdir) / "config.toml"
            config_path.write_text(
                '[slack]\n\n[[slack.channel_routes]]\nchannel_id = "D0QA"\n'
                'subject_user_id = "U0FIRST"\n',
                encoding="utf-8",
            )

            self.assertTrue(
                run_live_qa._append_slack_channel_route(
                    config_path,
                    subject_user_id="U0SECOND",
                    channel_id="D0QA",
                )
            )

            config = config_path.read_text(encoding="utf-8")
            self.assertEqual(config.count('channel_id = "D0QA"'), 2)
            self.assertIn('subject_user_id = "U0FIRST"', config)
            self.assertIn('subject_user_id = "U0SECOND"', config)

    def test_slack_config_values_are_toml_escaped(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = Path(tmpdir) / "config.toml"
            config_path.write_text("[slack]\n", encoding="utf-8")
            value = 'U0"REBORN\nQA'

            self.assertTrue(
                run_live_qa._set_slack_section_key(config_path, "slack_user_id", value)
            )

            self.assertIn(
                f"slack_user_id = {json.dumps(value)}",
                config_path.read_text(encoding="utf-8"),
            )

    def test_non_signed_slack_cases_do_not_configure_legacy_actor(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = Path(tmpdir) / "config.toml"
            config_path.write_text("[slack]\n", encoding="utf-8")

            changed, user_id = run_live_qa._configure_slack_legacy_actor_if_needed(
                config_path,
                ["qa_3a_slack_connect"],
            )

            self.assertFalse(changed)
            self.assertIsNone(user_id)
            self.assertNotIn("slack_user_id", config_path.read_text(encoding="utf-8"))

    def test_slack_event_run_id_reads_idempotency_record(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_path = home / "local-dev" / "reborn-local-dev.db"
            db_path.parent.mkdir(parents=True)
            with sqlite3.connect(db_path) as db:
                db.execute(
                    """
                    CREATE TABLE root_filesystem_entries (
                        path TEXT PRIMARY KEY,
                        contents BLOB NOT NULL,
                        updated_at TEXT NOT NULL DEFAULT '2026-01-01T00:00:00Z'
                    )
                    """
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents) VALUES (?, ?)",
                    (
                        "/tenants/reborn-cli/shared/slack-product-workflow/"
                        "idempotency/actions/event.json",
                        json.dumps(
                            {
                                "fingerprint": {
                                    "external_event_id": (
                                        "slack-local-dev-installation-EvREBORNQA5D123"
                                    )
                                },
                                "dispatch_kind": {
                                    "user_message_turn": {"run_id": "run-from-dispatch"}
                                },
                                "outcome": {
                                    "accepted": {"submitted_run_id": "run-from-outcome"}
                                },
                            }
                        ),
                    ),
                )

            self.assertEqual(
                run_live_qa._slack_event_run_id_for_event(home, "EvREBORNQA5D123"),
                "run-from-dispatch",
            )

    def test_wait_for_google_sheet_marker_after_slack_event_approves_gate(self):
        ctx = self._dummy_ctx()

        async def fake_resolve_gate(_ctx, *, thread_id, run_id, gate_ref):
            return {
                "status": 200,
                "thread_id": thread_id,
                "run_id": run_id,
                "gate_ref": gate_ref,
            }

        async def fake_google_sheet_contains_marker(**_kwargs):
            return {"found": True, "row_count": 2}

        with (
            patch.object(
                run_live_qa,
                "_slack_event_run_id_for_event",
                return_value="run-123",
            ),
            patch.object(
                run_live_qa,
                "_delivered_gate_routes_for_run",
                return_value=[
                    {
                        "thread_id": "thread-123",
                        "run_id": "run-123",
                        "gate_ref": "gate:approval-123",
                    }
                ],
            ),
            patch.object(
                run_live_qa,
                "_resolve_webui_approval_gate",
                side_effect=fake_resolve_gate,
            ),
            patch.object(
                run_live_qa,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
        ):
            result = asyncio.run(
                run_live_qa._wait_for_google_sheet_marker_after_slack_event(
                    ctx,
                    event_id="EvREBORNQA7E123",
                    access_token="access-token",
                    spreadsheet_id="spreadsheet-id",
                    marker="row-marker",
                    timeout=1.0,
                )
            )

        self.assertTrue(result["found"])
        self.assertEqual(result["slack_event_run_id"], "run-123")
        self.assertEqual(
            result["approval_attempts"],
            [
                {
                    "status": 200,
                    "thread_id": "thread-123",
                    "run_id": "run-123",
                    "gate_ref": "gate:approval-123",
                }
            ],
        )

    def test_generated_google_seed_creates_refreshable_product_auth_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN": "fake-access-token",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "fake-refresh-token",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "fake-client-id",
                "REBORN_WEBUI_V2_LIVE_QA_SKIP_GOOGLE_REFRESH_PROBE": "1",
            }
            with patch.dict(os.environ, env, clear=False):
                seed = run_live_qa._seed_generated_google_product_auth_if_configured(
                    home,
                    "qa-user",
                )
                preflight = run_live_qa._google_product_auth_preflight(
                    home,
                    "qa-user",
                    {"IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "fake-client-id"},
                )

            self.assertTrue(seed["seeded"])
            self.assertTrue(preflight["configured_ready"])
            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)
            account = preflight["accounts"][0]
            self.assertTrue(account["access_secret_expired"])
            self.assertTrue(account["refresh_secret_present"])
            self.assertEqual(account["refresh_probe"]["reason"], "disabled_by_env")

            db_path = home / "local-dev" / "reborn-local-dev.db"
            master_key_path = home / "local-dev" / ".reborn-local-dev-secrets-master-key"
            self.assertEqual(master_key_path.stat().st_mode & 0o777, 0o600)
            master_key = master_key_path.read_text(encoding="utf-8")
            with sqlite3.connect(db_path) as db:
                rows = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE '%/secrets/google-oauth-refresh-%'"
                ).fetchall()
            self.assertEqual(len(rows), 1)
            stored = json.loads(rows[0][0])
            self.assertEqual(
                run_live_qa._decrypt_filesystem_secret(master_key, stored),
                "fake-refresh-token",
            )

    def test_google_runtime_token_refreshes_before_env_access_fallback(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")

        class FakeResponse:
            status_code = 200

            @staticmethod
            def json():
                return {
                    "access_token": "fresh-access-token",
                    "expires_in": 3600,
                    "scope": "gmail.modify spreadsheets",
                }

        class FakeHttpx:
            calls: list[dict[str, object]] = []

            @classmethod
            def post(cls, url, *, data, timeout):
                cls.calls.append({"url": url, "data": data, "timeout": timeout})
                return FakeResponse()

        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN": "stale-env-access-token",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "fake-refresh-token",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "fake-client-id",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET": "fake-client-secret",
            }
            with patch.dict(os.environ, env, clear=False):
                run_live_qa._seed_generated_google_product_auth_if_configured(
                    home,
                    "qa-user",
                )
                with patch.dict(sys.modules, {"httpx": FakeHttpx}):
                    token, meta = run_live_qa._google_runtime_access_token(
                        home,
                        "qa-user",
                    )

            self.assertEqual(token, "fresh-access-token")
            self.assertEqual(meta["source"], "reborn_product_auth_refresh_secret")
            self.assertTrue(meta["refreshed"])
            self.assertEqual(len(FakeHttpx.calls), 1)
            self.assertEqual(
                FakeHttpx.calls[0]["data"]["refresh_token"],
                "fake-refresh-token",
            )

    def test_generated_github_seed_creates_manual_token_product_auth_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GITHUB_TOKEN": "fake-github-token",
            }
            with patch.dict(os.environ, env, clear=False):
                seed = run_live_qa._seed_generated_github_product_auth_if_configured(
                    home,
                    "qa-user",
                )
                preflight = run_live_qa._github_auth_preflight(
                    home,
                    {},
                    requires_github_auth=True,
                )

            self.assertTrue(seed["seeded"])
            self.assertEqual(seed["token_env_source"], "AUTH_LIVE_GITHUB_TOKEN")
            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)

            db_path = home / "local-dev" / "reborn-local-dev.db"
            master_key_path = home / "local-dev" / ".reborn-local-dev-secrets-master-key"
            self.assertEqual(master_key_path.stat().st_mode & 0o777, 0o600)
            master_key = master_key_path.read_text(encoding="utf-8")
            with sqlite3.connect(db_path) as db:
                account_row = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE '%product-auth/callback/accounts/%.json'"
                ).fetchone()
            self.assertIsNotNone(account_row)
            account = json.loads(account_row[0])
            self.assertEqual(account["provider"], "github")
            self.assertEqual(account["status"], "configured")
            expected_handle = (
                f"product-auth-manual-{seed['account_id']}-{seed['account_id']}"
            )
            self.assertEqual(account["access_secret"], expected_handle)

            with sqlite3.connect(db_path) as db:
                secret_row = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE ?",
                    (f"%/{account['access_secret']}.json",),
                ).fetchone()
            self.assertIsNotNone(secret_row)
            stored = json.loads(secret_row[0])
            self.assertEqual(
                run_live_qa._decrypt_filesystem_secret(master_key, stored),
                "fake-github-token",
            )

    def test_prepare_reborn_home_gates_missing_slack_without_raising(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            missing_source = root / "missing-source-home"
            args = argparse.Namespace(
                output_dir=root / "out",
                reborn_home=missing_source,
                require_slack_live=False,
            )
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
            }
            for name in (
                "IRONCLAW_REBORN_SLACK_SIGNING_SECRET",
                "IRONCLAW_REBORN_SLACK_SIGNING_SECRET_PATH",
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN",
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN_PATH",
            ):
                env[name] = ""

            with patch.dict(os.environ, env, clear=False):
                prepared = run_live_qa.prepare_reborn_home(
                    args,
                    ["qa_3a_slack_connect"],
                )

            slack = prepared.preflight["slack"]
            self.assertTrue(slack["enabled_in_config"])
            self.assertTrue(slack["requires_slack"])
            self.assertFalse(slack["env_present"])
            self.assertEqual(slack["auth_test"]["error"], "Slack env unavailable")
            self.assertEqual(slack["config_installation_id"], "local-dev-installation")
            self.assertEqual(slack["config_team_id"], "local-dev-team")
            self.assertEqual(slack["config_api_app_id"], "local-dev-app-id")

    def test_prepare_reborn_home_synthesizes_config_for_copied_db_home(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            source_home = root / "source-home"
            (source_home / "local-dev").mkdir(parents=True)
            run_live_qa._root_filesystem_create_table(
                source_home / "local-dev" / "reborn-local-dev.db"
            )
            args = argparse.Namespace(
                output_dir=root / "out",
                reborn_home=source_home,
                require_slack_live=False,
            )
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
            }

            with patch.dict(os.environ, env, clear=False):
                prepared = run_live_qa.prepare_reborn_home(
                    args,
                    ["qa_3a_slack_connect"],
                )

            config = (prepared.path / "config.toml").read_text(encoding="utf-8")
            self.assertIn('profile = "local-dev"', config)
            self.assertIn("[llm.default]", config)
            self.assertIn("[slack]", config)
            self.assertIn('api_key_env = "LIVE_OPENAI_COMPATIBLE_API_KEY"', config)
            self.assertFalse((source_home / "config.toml").exists())

    def test_generated_slack_home_ignores_empty_ci_vars(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID": "",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID": "",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID": "",
                "IRONCLAW_REBORN_SLACK_SIGNING_SECRET": "",
                "IRONCLAW_REBORN_SLACK_BOT_TOKEN": "",
            }

            with patch.dict(os.environ, env, clear=True):
                run_live_qa.create_generated_reborn_home(home, include_slack=True)

            config = (home / "config.toml").read_text(encoding="utf-8")
            self.assertIn('installation_id = "local-dev-installation"', config)
            self.assertIn('team_id = "local-dev-team"', config)
            self.assertIn('api_app_id = "local-dev-app-id"', config)
            self.assertNotIn('installation_id = ""', config)
            self.assertNotIn('team_id = ""', config)
            self.assertNotIn('api_app_id = ""', config)

    def test_default_suite_includes_github_connect_after_generated_auth_seed(self):
        self.assertTrue(run_live_qa.CASES["qa_4b_github_connect"].default_enabled)
        self.assertTrue(run_live_qa.CASES["qa_4b_github_connect"].requires_github_auth)
        self.assertIn("qa_4b_github_connect", run_live_qa.CASES)
        default_cases = [
            name
            for name, spec in run_live_qa.CASES.items()
            if spec.default_enabled
        ]
        self.assertIn("qa_4b_github_connect", default_cases)
        self.assertTrue(
            set(default_cases).issubset(run_live_qa.QA_SHEET_CASES),
            f"default cases must come from the QA spreadsheet: {default_cases}",
        )

    def test_non_telegram_qa_suite_selects_full_current_live_target(self):
        args = argparse.Namespace(
            all_cases=False,
            non_telegram_qa_cases=True,
            case=[],
        )

        selected_cases = run_live_qa._selected_case_names(args)

        self.assertEqual(len(selected_cases), 33)
        self.assertNotIn("qa_1a_telegram_connect", selected_cases)
        self.assertNotIn("qa_1b_telegram_near_news_chat", selected_cases)
        self.assertNotIn("qa_1c_telegram_near_news_routine", selected_cases)
        for case_name in (
            "qa_2d_calendar_prep_live_chat",
            "qa_2f_calendar_prep_email_delivery",
            "qa_4e_github_release_email_delivery",
            "qa_5c_strategy_doc_knowledge_base",
            "qa_5d_slack_strategy_doc_answer",
            "qa_6c_gmail_to_sheet_live_chat",
            "qa_6e_gmail_to_sheet_delivery",
            "qa_7e_slack_bug_sheet_delivery",
        ):
            self.assertIn(case_name, selected_cases)

    def test_live_canary_workflow_shards_cover_non_telegram_qa_suite(self):
        args = argparse.Namespace(
            all_cases=False,
            non_telegram_qa_cases=True,
            case=[],
        )
        selected_cases = run_live_qa._selected_case_names(args)
        workflow_path = (
            Path(__file__).resolve().parents[2] / ".github/workflows/live-canary.yml"
        )
        workflow = workflow_path.read_text(encoding="utf-8")
        match = re.search(
            r"(?ms)^  reborn-webui-v2-live-qa:\n(?P<body>.*?)^  persona-rotating:",
            workflow,
        )
        self.assertIsNotNone(match, "Reborn WebUI v2 live QA job missing")

        shard_case_lines = re.findall(r"^\s+cases:\s*(\S+)\s*$", match.group("body"), re.M)
        self.assertEqual(len(shard_case_lines), 7)
        sharded_cases = [
            case_name
            for line in shard_case_lines
            for case_name in line.split(",")
            if case_name
        ]

        self.assertEqual(len(sharded_cases), len(set(sharded_cases)))
        self.assertEqual(sharded_cases, selected_cases)
        all_shard_cases_match = re.search(
            r"(?ms)^\s+ALL_SHARD_CASES:\s*>-\n(?P<cases>.*?)(?=^\s+run:\s*\|)",
            match.group("body"),
        )
        self.assertIsNotNone(
            all_shard_cases_match,
            "Reborn WebUI v2 live QA all-case validation list missing",
        )
        all_shard_cases = [
            case_name.strip()
            for line in all_shard_cases_match.group("cases").splitlines()
            for case_name in line.split(",")
            if case_name.strip()
        ]
        self.assertEqual(all_shard_cases, selected_cases)
        self.assertIn(
            "Unknown Reborn WebUI v2 live QA case",
            match.group("body"),
        )
        self.assertIn("target_pr is required when target_ref is supplied", match.group("body"))
        self.assertIn("Refusing to run reborn-webui-v2-live-qa with live secrets on a forked PR", match.group("body"))
        self.assertIn("target_ref does not match PR #${TARGET_PR} head SHA", match.group("body"))
        self.assertIn("ref: ${{ steps.validate_reborn_webui_v2_target.outputs.checkout_ref", match.group("body"))

        command_workflow_path = (
            Path(__file__).resolve().parents[2] / ".github/workflows/live-canary-command.yml"
        )
        command_workflow = command_workflow_path.read_text(encoding="utf-8")
        self.assertIn('-f target_ref="$HEAD_SHA"', command_workflow)
        self.assertIn('-f target_pr="$PR"', command_workflow)

    def test_case_manifest_distinguishes_targeted_from_placeholder_gates(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            sheet_url = "https://docs.google.com/spreadsheets/d/test-spreadsheet/edit"
            with patch.dict(
                os.environ,
                {"REBORN_WEBUI_V2_LIVE_QA_SHEET_URL": sheet_url},
                clear=False,
            ):
                manifest_path = run_live_qa.write_case_manifest(
                    output_dir,
                    [
                        "qa_2d_calendar_prep_live_chat",
                        "qa_2f_calendar_prep_email_delivery",
                    ],
                )
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

        self.assertNotIn("qa_matrix", manifest)
        self.assertEqual(manifest["qa_sheet"]["source"], "google_sheets")
        self.assertEqual(manifest["qa_sheet"]["url"], sheet_url)
        self.assertEqual(manifest["qa_sheet"]["tab"], "Automated")
        cases = {case["case"]: case for case in manifest["cases"]}
        self.assertTrue(
            set(cases).issubset(run_live_qa.QA_SHEET_CASES),
            f"manifest cases must come from the QA spreadsheet: {sorted(cases)}",
        )
        self.assertTrue(cases["qa_2d_calendar_prep_live_chat"]["implemented"])
        self.assertEqual(
            cases["qa_2d_calendar_prep_live_chat"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_2f_calendar_prep_email_delivery"]["implemented"])
        self.assertEqual(
            cases["qa_2f_calendar_prep_email_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_4e_github_release_email_delivery"]["implemented"])
        self.assertEqual(
            cases["qa_4e_github_release_email_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_5d_slack_strategy_doc_answer"]["implemented"])
        self.assertTrue(cases["qa_5d_slack_strategy_doc_answer"]["requires_slack_target"])
        self.assertEqual(
            cases["qa_5d_slack_strategy_doc_answer"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_6e_gmail_to_sheet_delivery"]["implemented"])
        self.assertEqual(
            cases["qa_6e_gmail_to_sheet_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertTrue(cases["qa_7e_slack_bug_sheet_delivery"]["implemented"])
        self.assertTrue(cases["qa_7e_slack_bug_sheet_delivery"]["requires_slack_target"])
        self.assertEqual(
            cases["qa_7e_slack_bug_sheet_delivery"]["status"],
            "gated:requires_live_google_product_auth",
        )
        self.assertFalse(cases["qa_1a_telegram_connect"]["implemented"])
        self.assertEqual(
            cases["qa_1a_telegram_connect"]["status"],
            "gated:requires_live_telegram",
        )

    def test_gmail_delivery_target_prefers_explicit_env(self):
        target = asyncio.run(
            run_live_qa._gmail_delivery_target_email(
                access_token="unused-token",
                extra_env={"REBORN_WEBUI_V2_LIVE_QA_EMAIL_TARGET": "qa@example.test"},
            )
        )
        self.assertEqual(target, "qa@example.test")

    def test_extract_google_spreadsheet_id_from_url_or_label(self):
        spreadsheet_id = "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890"
        self.assertEqual(
            run_live_qa._extract_google_spreadsheet_id(
                f"Created: https://docs.google.com/spreadsheets/d/{spreadsheet_id}/edit#gid=0"
            ),
            spreadsheet_id,
        )
        self.assertEqual(
            run_live_qa._extract_google_spreadsheet_id(
                f"spreadsheet id: {spreadsheet_id}"
            ),
            spreadsheet_id,
        )
        explicit_id = "1NewExplicitSpreadsheetId_1234567890abcdefghi"
        self.assertEqual(
            run_live_qa._extract_google_spreadsheet_id(
                f"Draft URL: https://docs.google.com/spreadsheets/d/{spreadsheet_id}/edit\n"
                f"spreadsheet id: {explicit_id}"
            ),
            explicit_id,
        )
        corrected_id = "18xFRoOs2aLrat-aq7daZ60Y_EPG2Wei6ZyDkkMebF30"
        self.assertEqual(
            run_live_qa._extract_google_spreadsheet_id(
                "Spreadsheet URL: "
                "https://docs.google.com/spreadsheets/d/"
                "18xFRoOs2aLrat-aq7daZYY0Y_EPG2Wei6ZyDkkMebF30/edit\n"
                "Wait - let me correct that URL. The actual returned URL is:\n"
                f"https://docs.google.com/spreadsheets/d/{corrected_id}/edit"
            ),
            corrected_id,
        )
        self.assertIsNone(
            run_live_qa._extract_google_spreadsheet_id(
                "Spreadsheet created: REBORN_QA_6E_GMAIL_TO_SHEET_DELIVERY_1782593757000"
            )
        )
        self.assertIsNone(run_live_qa._extract_google_spreadsheet_id("no sheet here"))

    def test_extract_google_document_id_from_url_or_label(self):
        document_id = "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890"
        self.assertEqual(
            run_live_qa._extract_google_document_id(
                f"Created: https://docs.google.com/document/d/{document_id}/edit"
            ),
            document_id,
        )
        self.assertEqual(
            run_live_qa._extract_google_document_id(
                f"Document created: QA doc (ID: {document_id})"
            ),
            document_id,
        )
        explicit_id = "1NewExplicitDocumentId_1234567890abcdefghijk"
        self.assertEqual(
            run_live_qa._extract_google_document_id(
                f"Draft URL: https://docs.google.com/document/d/{document_id}/edit\n"
                f"Document created: QA doc (ID: {explicit_id})"
            ),
            explicit_id,
        )
        self.assertIsNone(
            run_live_qa._extract_google_document_id(
                "Document created: REBORN_QA_5D_STRATEGY_DOC_1782597084534"
            )
        )
        self.assertIsNone(
            run_live_qa._extract_google_document_id(
                "Document: REBORN_QA_5D_STRATEGY_DOC_1782599165051 (Google Docs)"
            )
        )

    def test_google_runtime_token_requires_client_secret_for_expired_copied_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN": "expired-access-token",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "refresh-token",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id",
            }
            with patch.dict(os.environ, env, clear=True):
                seed = run_live_qa._seed_generated_google_product_auth_if_configured(
                    home,
                    "qa-user",
                )
            self.assertTrue(seed["seeded"])

            with patch.dict(os.environ, {}, clear=True):
                with self.assertRaisesRegex(
                    run_live_qa.LiveQaError,
                    "client id/secret env is incomplete",
                ):
                    run_live_qa._google_runtime_access_token(
                        home,
                        "qa-user",
                        {"IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id"},
                    )

    def test_bootstrap_forwards_all_cases_flag(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir) / "out"
            home = Path(tmpdir) / "home"
            argv = [
                "run_live_qa.py",
                "--output-dir",
                str(output_dir),
                "--reborn-home",
                str(home),
                "--all-cases",
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(run_live_qa, "bootstrap_python", return_value=Path("/venv/bin/python")),
                patch.object(run_live_qa, "install_playwright"),
                patch.object(run_live_qa.subprocess, "run") as subprocess_run,
            ):
                subprocess_run.return_value.returncode = 0
                self.assertEqual(run_live_qa.main(), 0)

            forwarded = subprocess_run.call_args.args[0]
            self.assertIn("--all-cases", forwarded)
            self.assertNotIn("--case", forwarded)

    def test_bootstrap_forwards_non_telegram_qa_cases_flag(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir) / "out"
            home = Path(tmpdir) / "home"
            argv = [
                "run_live_qa.py",
                "--output-dir",
                str(output_dir),
                "--reborn-home",
                str(home),
                "--non-telegram-qa-cases",
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(run_live_qa, "bootstrap_python", return_value=Path("/venv/bin/python")),
                patch.object(run_live_qa, "install_playwright"),
                patch.object(run_live_qa.subprocess, "run") as subprocess_run,
            ):
                subprocess_run.return_value.returncode = 0
                self.assertEqual(run_live_qa.main(), 0)

            forwarded = subprocess_run.call_args.args[0]
            self.assertIn("--non-telegram-qa-cases", forwarded)
            self.assertNotIn("--all-cases", forwarded)
            self.assertNotIn("--case", forwarded)

    def test_delivered_gate_routes_for_run_reads_trigger_gate_records(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3.connect(db_path) as db:
                db.execute(
                    """
                    CREATE TABLE root_filesystem_entries (
                        path TEXT PRIMARY KEY,
                        contents BLOB NOT NULL,
                        updated_at TEXT NOT NULL
                    )
                    """
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents, updated_at) "
                    "VALUES (?, ?, ?)",
                    (
                        "/tenants/reborn-cli/users/qa/outbound/delivered-gate-routes/route.json",
                        json.dumps(
                            {
                                "gate_ref": "gate:approval-abc",
                                "run_id": "run-123",
                                "scope": {"thread_id": "thread-456"},
                            }
                        ),
                        "2026-06-24T00:00:00Z",
                    ),
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents, updated_at) "
                    "VALUES (?, ?, ?)",
                    (
                        "/tenants/reborn-cli/users/qa/outbound/delivered-gate-routes/other.json",
                        json.dumps(
                            {
                                "gate_ref": "gate:approval-other",
                                "run_id": "run-other",
                                "scope": {"thread_id": "thread-other"},
                            }
                        ),
                        "2026-06-24T00:00:01Z",
                    ),
                )

            routes = run_live_qa._delivered_gate_routes_for_run(home, "run-123")

            self.assertEqual(
                routes,
                [
                    {
                        "path": "/tenants/reborn-cli/users/qa/outbound/delivered-gate-routes/route.json",
                        "gate_ref": "gate:approval-abc",
                        "thread_id": "thread-456",
                        "run_id": "run-123",
                    }
                ],
            )

    def test_github_auth_preflight_detects_configured_product_auth_account(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3.connect(db_path) as db:
                db.execute(
                    """
                    CREATE TABLE root_filesystem_entries (
                        path TEXT PRIMARY KEY,
                        contents BLOB NOT NULL
                    )
                    """
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries(path, contents) VALUES (?, ?)",
                    (
                        "/tenants/reborn-cli/users/qa/secrets/agents/reborn-cli-agent/"
                        "product-auth/callback/accounts/github.json",
                        json.dumps(
                            {
                                "provider": "github",
                                "status": "configured",
                                "access_secret": "product-auth-manual-github",
                            }
                        ),
                    ),
                )

            preflight = run_live_qa._github_auth_preflight(
                home,
                {},
                requires_github_auth=True,
            )

            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)

    def test_github_auth_preflight_blocks_without_configured_account(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            (home / "local-dev").mkdir(parents=True)

            preflight = run_live_qa._github_auth_preflight(
                home,
                {},
                requires_github_auth=True,
            )

            self.assertFalse(preflight["ready"])
            self.assertIn("missing GitHub live prerequisites", preflight["reason"])

    def test_google_required_env_for_runtime_block_includes_refresh_inputs(self):
        required = run_live_qa._google_required_env_for_block(
            {
                "missing_google_client_secret": True,
                "refresh_probe_failed": True,
            },
            requires_runtime_access=True,
        )

        self.assertEqual(
            required,
            [
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
                "AUTH_LIVE_GOOGLE_ACCESS_TOKEN",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN",
            ],
        )

    def test_google_required_env_for_connect_block_keeps_client_id_only(self):
        required = run_live_qa._google_required_env_for_block(
            {},
            requires_runtime_access=False,
        )

        self.assertEqual(required, ["IRONCLAW_REBORN_GOOGLE_CLIENT_ID"])

    def test_google_credential_action_for_invalid_grant_requires_token_rotation(self):
        action = run_live_qa._google_credential_action_for_block(
            {
                "accounts": [
                    {
                        "refresh_probe": {
                            "ok": False,
                            "oauth_error_code": "invalid_grant",
                        },
                    },
                ],
            },
        )

        self.assertIsNotNone(action)
        self.assertIn("AUTH_LIVE_GOOGLE_ACCESS_TOKEN", action)
        self.assertIn("AUTH_LIVE_GOOGLE_REFRESH_TOKEN", action)
        self.assertIn("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET", action)

    def test_google_credential_action_for_missing_client_secret_names_secret(self):
        action = run_live_qa._google_credential_action_for_block(
            {
                "accounts": [
                    {
                        "refresh_probe": {
                            "ok": False,
                            "error": "google_oauth_refresh_request_failed",
                            "client_secret_present": False,
                        },
                    },
                ],
            },
        )

        self.assertIsNotNone(action)
        self.assertIn("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET", action)

    def test_slack_delivery_observed_is_status_agnostic_after_gate_resume(self):
        self.assertTrue(
            run_live_qa._slack_delivery_observed(
                {"outcome": "delivered", "run_id": "run-123"},
                {"found": True, "marker_found": True},
            )
        )
        self.assertFalse(
            run_live_qa._slack_delivery_observed(
                {"outcome": "gate_required", "run_id": "run-123"},
                {"found": True, "marker_found": True},
            )
        )
        self.assertFalse(
            run_live_qa._slack_delivery_observed(
                {"outcome": "delivered", "run_id": "run-123"},
                {"found": False, "marker_found": True},
            )
        )

    def test_export_case_trace_writes_runtime_entries_without_secret_store(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            home = root / "reborn-home"
            db_path = home / "local-dev" / "reborn-local-dev.db"
            output_dir = root / "out"
            run_live_qa._root_filesystem_create_table(db_path)
            message_path = (
                "/tenants/reborn-cli/users/test/threads/agents/reborn-cli-agent/"
                "owners/test/threads/thread-1/messages/message-1.json"
            )
            tool_index_path = (
                "/tenants/reborn-cli/users/test/threads/agents/reborn-cli-agent/"
                "owners/test/threads/thread-1/indexes/tool-results/tool-result-1.json"
            )
            secret_path = (
                "/tenants/reborn-cli/users/test/secrets/agents/reborn-cli-agent/"
                "secrets/access-token.json"
            )
            run_live_qa._put_root_filesystem_json(
                db_path,
                message_path,
                {"kind": "user", "content": "hello live trace"},
            )
            run_live_qa._put_root_filesystem_json(
                db_path,
                tool_index_path,
                {"target": "tool-result-1"},
            )
            run_live_qa._put_root_filesystem_json(
                db_path,
                secret_path,
                {"access_token": "should-not-be-exported"},
            )

            trace = run_live_qa.export_case_trace(output_dir, "case_a", home)

            self.assertEqual(trace["entry_count"], 2)
            payload = json.loads(
                (output_dir / "traces" / "case_a.json").read_text(encoding="utf-8")
            )
            paths = [entry["path"] for entry in payload["entries"]]
            self.assertEqual(paths, [tool_index_path, message_path])
            self.assertNotIn(secret_path, paths)
            self.assertEqual(payload["entries"][1]["contents"]["content"], "hello live trace")

    def test_run_cases_isolates_reborn_home_and_preflight_per_selected_case(self):
        async def fake_case(ctx: run_live_qa.LiveQaContext) -> run_live_qa.ProbeResult:
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live",
                success=True,
                latency_ms=1,
                details={"reborn_home": str(ctx.reborn_home)},
            )

        async def fake_start_reborn_server(
            _binary: Path,
            reborn_home: Path,
            _output_dir: Path,
            _env: dict[str, str],
        ):
            return object(), f"http://127.0.0.1/{reborn_home.name}"

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw-reborn"
            binary.touch()
            args = argparse.Namespace(
                all_cases=False,
                non_telegram_qa_cases=False,
                case=["case_a", "case_b"],
                output_dir=output_dir,
                reborn_home=root / "missing-source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "case_a": run_live_qa.CaseSpec(fake_case),
                "case_b": run_live_qa.CaseSpec(fake_case),
            }
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
            }

            with (
                patch.dict(os.environ, env, clear=False),
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fake_start_reborn_server,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            self.assertEqual(status, 0)
            case_a_home = output_dir / "reborn-home" / "case_a"
            case_b_home = output_dir / "reborn-home" / "case_b"
            self.assertTrue((case_a_home / "config.toml").exists())
            self.assertTrue((case_b_home / "config.toml").exists())
            self.assertNotEqual(case_a_home, case_b_home)

            case_a_preflight = json.loads(
                (output_dir / "preflight.case_a.json").read_text(encoding="utf-8")
            )
            case_b_preflight = json.loads(
                (output_dir / "preflight.case_b.json").read_text(encoding="utf-8")
            )
            self.assertEqual(case_a_preflight["reborn_home"], str(case_a_home))
            self.assertEqual(case_b_preflight["reborn_home"], str(case_b_home))
            self.assertTrue((output_dir / "traces" / "case_a.json").exists())
            self.assertTrue((output_dir / "traces" / "case_b.json").exists())
            self.assertTrue((output_dir / "traces" / "index.json").exists())


if __name__ == "__main__":
    unittest.main()
