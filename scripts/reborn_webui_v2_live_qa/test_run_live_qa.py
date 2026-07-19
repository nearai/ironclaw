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
from contextlib import closing
from pathlib import Path
from unittest.mock import patch

if __package__:
    from . import google_api_helpers
    from . import run_live_qa
    from . import semantic_judge
    from . import text_match
else:
    import run_live_qa
    from scripts.reborn_webui_v2_live_qa import google_api_helpers
    import semantic_judge
    import text_match


class _FakeEmptyErrorBlocks:
    @property
    def last(self):
        return self

    async def count(self):
        return 0

    async def inner_text(self, **_kwargs):
        return ""

    async def get_attribute(self, name, **_kwargs):
        if name in ("data-failure-category", "data-failure-status"):
            return None
        raise AssertionError(f"unexpected error attribute: {name}")


class RebornWebUiV2LiveQaRunnerTests(unittest.TestCase):
    def _dummy_ctx(self) -> run_live_qa.LiveQaContext:
        return run_live_qa.LiveQaContext(
            base_url="http://127.0.0.1:9",
            output_dir=Path("/tmp"),
            reborn_home=Path("/tmp/reborn-home"),
            env={},
        )

    def _drive_submission_capture_state(
        self,
        *,
        response_payloads: list[dict[str, object] | None],
        user_bubble_failures: int = 0,
        terminal_failure: run_live_qa.TerminalRunFailureObservation | None = None,
    ) -> tuple[run_live_qa.ProbeResult, dict[str, int]]:
        state = {
            "presses": 0,
            "dismissals": 0,
            "user_bubble_assertions": 0,
            "response_waiters": 0,
        }

        class FakeLocator:
            def __init__(self, selector):
                self.selector = selector

            @property
            def last(self):
                return self

            async def count(self):
                return 0

        class FakeComposer(FakeLocator):
            async def fill(self, _text):
                return None

            async def press(self, _key):
                state["presses"] += 1

        class FakeDismiss(FakeLocator):
            @property
            def first(self):
                return self

            async def count(self):
                return int(state["presses"] > 0 and state["dismissals"] == 0)

            async def is_visible(self):
                return True

            async def click(self):
                state["dismissals"] += 1

        class FakeRequest:
            method = "POST"
            post_data_json = {"content": "Read Slack."}

        class FakeResponse:
            url = (
                "http://127.0.0.1:9/api/webchat/v2/threads/"
                "thread-current/messages"
            )
            request = FakeRequest()

            def __init__(self, payload):
                self.payload = payload

            async def json(self):
                return self.payload

        class FakeResponseInfo:
            def __init__(self, payload):
                self.payload = payload

            async def __aenter__(self):
                return self

            async def __aexit__(self, *_args):
                return None

            @property
            def value(self):
                async def get_response():
                    if self.payload is None:
                        raise TimeoutError("no matching submission response")
                    return FakeResponse(self.payload)

                return get_response()

        class FakePage:
            async def goto(self, _url, **_kwargs):
                return None

            def expect_response(self, predicate, **_kwargs):
                index = state["response_waiters"]
                state["response_waiters"] += 1
                payload = response_payloads[index]
                if payload is not None:
                    self_outer.assertTrue(predicate(FakeResponse(payload)))
                return FakeResponseInfo(payload)

            def locator(self, selector):
                if selector == "[aria-label='Dismiss connect action']":
                    return FakeDismiss(selector)
                if selector == "[data-testid='chat-composer']":
                    return FakeComposer(selector)
                return FakeLocator(selector)

        class FakeExpectation:
            def __init__(self, locator):
                self.locator = locator

            async def to_be_visible(self, **_kwargs):
                return None

            async def to_contain_text(self, _text, **_kwargs):
                if self.locator.selector == "[data-testid='msg-user']":
                    state["user_bubble_assertions"] += 1
                    if state["user_bubble_assertions"] <= user_bubble_failures:
                        raise AssertionError("user bubble not visible")

        async def fake_with_page(_output_dir, _case_name, action):
            await action(FakePage())

        async def fake_wait(_page, **_kwargs):
            if terminal_failure is not None:
                raise run_live_qa.TerminalRunFailure(terminal_failure)
            return run_live_qa.AssistantReplyWaitResult(
                text_excerpt="Slack answer.",
                full_text="Slack answer.",
                semantic_judge_used=False,
                semantic_judge_reason="literal_required_text_matched",
                final_reply_wait_ms=0,
                final_reply_reason="final_reply_observed",
            )

        playwright_module = types.ModuleType("playwright")
        playwright_async_api = types.ModuleType("playwright.async_api")
        playwright_async_api.expect = lambda locator: FakeExpectation(locator)
        self_outer = self
        with (
            patch.dict(
                sys.modules,
                {
                    "playwright": playwright_module,
                    "playwright.async_api": playwright_async_api,
                },
            ),
            patch.object(run_live_qa, "_with_page", new=fake_with_page),
            patch.object(run_live_qa, "_wait_for_assistant_reply", new=fake_wait),
        ):
            result = asyncio.run(
                run_live_qa._live_chat_case(
                    self._dummy_ctx(),
                    case_name="qa_submission_capture_state",
                    prompt="Read Slack.",
                    marker=None,
                    required_text=[],
                    capture_submission_identity=True,
                    enforce_marker=False,
                )
            )
        return result, state

    def test_case_spec_accepts_only_explicit_contract_or_behavioral_tiers(self):
        async def fake_case(_ctx):
            return None

        contract = run_live_qa.CaseSpec(
            fake_case,
            tier="contract",
            blocking=True,
        )
        behavioral = run_live_qa.CaseSpec(
            fake_case,
            tier="behavioral",
            blocking=False,
        )

        self.assertEqual(contract.tier, "contract")
        self.assertTrue(contract.blocking)
        self.assertEqual(behavioral.tier, "behavioral")
        self.assertFalse(behavioral.blocking)
        with self.assertRaisesRegex(ValueError, "tier"):
            run_live_qa.CaseSpec(fake_case, tier="advisory", blocking=False)

    def _fake_assistant_reply_page(
        self,
        response_text: str,
        *,
        final_reply_state: str | None = "true",
        error_messages: list[dict[str, str | None]] | None = None,
        assistant_block_texts: list[str] | None = None,
    ):
        error_messages = error_messages or []
        assistant_block_texts = assistant_block_texts or [response_text]

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
                return len(assistant_block_texts)

            async def inner_text(self, **_kwargs):
                return response_text

            async def get_attribute(self, name, **_kwargs):
                if name == "data-final-reply":
                    return final_reply_state
                return None

            async def all_inner_texts(self):
                return assistant_block_texts

        class FakeErrorBlocks:
            @property
            def last(self):
                return self

            async def count(self):
                return len(error_messages)

            async def inner_text(self, **_kwargs):
                if not error_messages:
                    return ""
                return str(error_messages[-1].get("summary") or "")

            async def get_attribute(self, name, **_kwargs):
                if not error_messages:
                    return None
                if name == "data-failure-category":
                    return error_messages[-1].get("failure_category")
                if name == "data-failure-status":
                    return error_messages[-1].get("failure_status")
                raise AssertionError(f"unexpected error attribute: {name}")

        class FakeMain:
            async def inner_text(self, **_kwargs):
                return response_text

        class FakePage:
            def locator(self, selector):
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantBlocks()
                if selector == "[data-testid='msg-error']":
                    return FakeErrorBlocks()
                if selector == "main":
                    return FakeMain()
                raise AssertionError(f"unexpected selector: {selector}")

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        return FakePage()

    def _fake_sequenced_terminal_page(
        self,
        snapshots: list[dict[str, list[dict[str, str | None]]]],
    ):
        state = {"index": 0}

        def current() -> dict[str, list[dict[str, str | None]]]:
            return snapshots[state["index"]]

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
                return len(current()["assistants"])

            async def inner_text(self, **_kwargs):
                return str(current()["assistants"][-1].get("text") or "")

            async def get_attribute(self, name, **_kwargs):
                if name == "data-final-reply":
                    return current()["assistants"][-1].get("final_reply_state")
                raise AssertionError(f"unexpected assistant attribute: {name}")

            async def all_inner_texts(self):
                return [
                    str(message.get("text") or "")
                    for message in current()["assistants"]
                ]

        class FakeErrorBlocks:
            @property
            def last(self):
                return self

            async def count(self):
                return len(current()["errors"])

            async def inner_text(self, **_kwargs):
                return str(current()["errors"][-1].get("summary") or "")

            async def get_attribute(self, name, **_kwargs):
                if name == "data-failure-category":
                    return current()["errors"][-1].get("failure_category")
                if name == "data-failure-status":
                    return current()["errors"][-1].get("failure_status")
                raise AssertionError(f"unexpected error attribute: {name}")

        class FakeMain:
            async def inner_text(self, **_kwargs):
                assistants = current()["assistants"]
                return str(assistants[-1].get("text") or "") if assistants else ""

        class FakePage:
            def locator(self, selector):
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantBlocks()
                if selector == "[data-testid='msg-error']":
                    return FakeErrorBlocks()
                if selector == "main":
                    return FakeMain()
                raise AssertionError(f"unexpected selector: {selector}")

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        def advance() -> None:
            state["index"] = min(len(snapshots) - 1, state["index"] + 1)

        return FakePage(), advance

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

        async def fake_webui_json(
            _page: object,
            method: str,
            path: str,
            payload: dict[str, object] | None = None,
        ) -> dict[str, object]:
            fetched_paths.append(path)
            if method == "POST" and path == "/api/webchat/v2/extensions/install":
                self.assertEqual(
                    payload,
                    {"package_ref": {"kind": "extension", "id": "slack"}},
                )
                return {
                    "success": True,
                    "message": "Slack installed",
                    "onboarding_state": "auth_required",
                }
            if method == "POST" and path == "/api/reborn/product-auth/accounts/list":
                self.assertEqual(payload["provider"], "slack_personal")
                self.assertEqual(payload["requester_extension"], "slack")
                self.assertEqual(payload["invocation_id"], "invocation-slack")
                self.assertEqual(payload["thread_id"], "thread-slack")
                return {
                    "accounts": [
                        {
                            "id": "slack-account",
                            "provider": "slack_personal",
                            "label": "slack_personal",
                            "status": "configured",
                            "ownership": "user_reusable",
                            "secret_handle_count": 1,
                        }
                    ]
                }
            if method == "POST" and path == "/api/webchat/v2/extensions/slack/setup/oauth/start":
                self.assertEqual(payload["provider"], "slack_personal")
                self.assertEqual(payload["scopes"], [])
                self.assertIsInstance(payload.get("invocation_id"), str)
                self.assertIsInstance(payload.get("expires_at"), str)
                return {
                    "provider": "slack_personal",
                    "authorization_url": "https://slack.com/oauth/v2/authorize?user_scope=chat:write",
                    "flow_id": "flow-slack",
                    "status": "pending",
                }
            self.assertEqual(method, "GET")
            if path == "/api/webchat/v2/extensions":
                return {"extensions": []}
            self.assertEqual(path, "/api/webchat/v2/channels/connectable")
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
                        "strategy": "oauth",
                        "action": {
                            "title": "Slack account connection",
                            "instructions": (
                                "Connect Slack with OAuth from the extension "
                                "configuration, then message the Slack bot directly."
                            ),
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
                                "setup": {
                                    "configured": True,
                                    "personal_oauth_ready": True,
                                },
                                "auth_test": {
                                    "ok": True,
                                    "team_id": "T123",
                                    "user_id": "U123",
                                },
                            },
                            "slack_personal_auth": {
                                "ready": True,
                                "configured_account_count": 1,
                                "accounts": [
                                    {
                                        "id": "slack-account",
                                        "ready": True,
                                        "thread_id": "thread-slack",
                                        "invocation_id": "invocation-slack",
                                    }
                                ],
                                "auth_test": {
                                    "ok": True,
                                    "team_id": "T123",
                                    "user_id": "U123",
                                },
                            },
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
                    "_webui_json",
                    new=fake_webui_json,
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
                    "http://127.0.0.1:3000/extensions/channels?"
                    f"token={run_live_qa.AUTH_TOKEN}",
                    "domcontentloaded",
                )
            ],
        )
        observed_expectations = [text for _selector, text, _timeout in expected_texts]
        self.assertIn("Channels", observed_expectations)
        self.assertIn("Slack workspace setup", observed_expectations)
        self.assertNotIn("Slack account connection", observed_expectations)
        self.assertNotIn("Connect Slack with OAuth", observed_expectations)
        self.assertNotIn("Connect Slack", observed_expectations)
        self.assertNotIn("pairing code", observed_expectations)
        self.assertFalse(any("/chat" in url for url, _wait in fake_page.gotos))
        self.assertEqual(
            result.details["slack_connect_surface"],
            "/extensions/channels",
        )
        self.assertEqual(
            result.details["slack_connect_title"],
            "Slack account connection",
        )
        self.assertEqual(
            fetched_paths,
            [
                "/api/webchat/v2/channels/connectable",
                "/api/webchat/v2/extensions",
                "/api/webchat/v2/extensions/install",
                "/api/reborn/product-auth/accounts/list",
                "/api/webchat/v2/extensions/slack/setup/oauth/start",
            ],
        )
        self.assertEqual(result.details["slack_install_message"], "Slack installed")
        self.assertEqual(
            result.details["slack_install_onboarding_state"],
            "auth_required",
        )
        self.assertEqual(result.details["slack_product_auth_account_count"], 1)
        self.assertEqual(
            result.details["slack_product_auth_configured_account_count"],
            1,
        )
        self.assertEqual(
            result.details["slack_oauth_start_provider"],
            "slack_personal",
        )
        self.assertIn(
            "https://slack.com/oauth/v2/authorize",
            result.details["slack_oauth_start_url"],
        )

    def test_slack_connect_case_fails_on_workspace_mismatch(self):
        async def fail_if_page_opens(*_args, **_kwargs) -> None:
            raise AssertionError("browser should not open with mismatched Slack teams")

        def fake_expect(_selector: str) -> object:
            raise AssertionError("expect should not be used with mismatched Slack teams")

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            (output_dir / "preflight.json").write_text(
                json.dumps(
                    {
                        "checks": {
                            "slack": {
                                "enabled_in_config": True,
                                "env_present": True,
                                "setup": {
                                    "configured": True,
                                    "team_id": "T-BOT",
                                    "personal_oauth_ready": True,
                                },
                                "auth_test": {
                                    "ok": True,
                                    "team_id": "T-BOT",
                                    "user_id": "U-BOT",
                                },
                            },
                            "slack_personal_auth": {
                                "ready": True,
                                "configured_account_count": 1,
                                "accounts": [{"id": "slack-account", "ready": True}],
                                "auth_test": {
                                    "ok": True,
                                    "team_id": "T-PERSONAL",
                                    "user_id": "U-PERSONAL",
                                },
                            },
                        }
                    }
                ),
                encoding="utf-8",
            )
            ctx = run_live_qa.LiveQaContext(
                base_url="http://127.0.0.1:3000",
                output_dir=output_dir,
                reborn_home=output_dir / "reborn-home",
                env={},
            )

            with (
                patch.object(run_live_qa, "_with_page", new=fail_if_page_opens),
                patch.object(run_live_qa, "_webui_json", new=fail_if_page_opens),
                patch.dict(
                    sys.modules,
                    {
                        "playwright": types.ModuleType("playwright"),
                        "playwright.async_api": types.SimpleNamespace(expect=fake_expect),
                    },
                ),
            ):
                result = asyncio.run(
                    run_live_qa._slack_connect_case(
                        ctx,
                        case_name="qa_3a_slack_connect",
                    )
                )

        self.assertFalse(result.success)
        self.assertIn("different workspaces", result.details["error"])
        self.assertIn("bot_token team_id=T-BOT", result.details["error"])
        self.assertIn("personal_oauth team_id=T-PERSONAL", result.details["error"])

    def test_slack_connect_case_fails_without_seeded_personal_account(self):
        async def fail_if_page_opens(*_args, **_kwargs) -> None:
            raise AssertionError("browser should not open without Slack product-auth account")

        def fake_expect(_selector: str) -> object:
            raise AssertionError("expect should not be used without Slack product-auth account")

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            (output_dir / "preflight.json").write_text(
                json.dumps(
                    {
                        "checks": {
                            "slack": {
                                "enabled_in_config": True,
                                "env_present": True,
                                "setup": {
                                    "configured": True,
                                    "personal_oauth_ready": True,
                                },
                                "auth_test": {
                                    "ok": True,
                                    "team_id": "T123",
                                    "user_id": "U123",
                                },
                            },
                            "slack_personal_auth": {
                                "ready": False,
                                "reason": "no configured Slack personal product-auth account",
                            },
                        }
                    }
                ),
                encoding="utf-8",
            )
            ctx = run_live_qa.LiveQaContext(
                base_url="http://127.0.0.1:3000",
                output_dir=output_dir,
                reborn_home=output_dir / "reborn-home",
                env={},
            )

            with (
                patch.object(run_live_qa, "_with_page", new=fail_if_page_opens),
                patch.object(run_live_qa, "_webui_json", new=fail_if_page_opens),
                patch.dict(
                    sys.modules,
                    {
                        "playwright": types.ModuleType("playwright"),
                        "playwright.async_api": types.SimpleNamespace(expect=fake_expect),
                    },
                ),
            ):
                result = asyncio.run(
                    run_live_qa._slack_connect_case(
                        ctx,
                        case_name="qa_3a_slack_connect",
                    )
                )

        self.assertFalse(result.success)
        self.assertIn("Slack personal product-auth", result.details["error"])

    def test_slack_connect_case_fails_when_personal_oauth_not_ready(self):
        async def fail_if_page_opens(*_args, **_kwargs) -> None:
            raise AssertionError("browser should not open when Slack OAuth setup is incomplete")

        def fake_expect(_selector: str) -> object:
            raise AssertionError("expect should not be used when Slack OAuth setup is incomplete")

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            (output_dir / "preflight.json").write_text(
                json.dumps(
                    {
                        "checks": {
                            "slack": {
                                "enabled_in_config": True,
                                "env_present": True,
                                "setup": {
                                    "configured": True,
                                    "personal_oauth_ready": False,
                                    "oauth_client_id_configured": False,
                                    "oauth_client_secret_configured": False,
                                },
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
            ctx = run_live_qa.LiveQaContext(
                base_url="http://127.0.0.1:3000",
                output_dir=output_dir,
                reborn_home=output_dir / "reborn-home",
                env={},
            )
            playwright_module = types.ModuleType("playwright")
            playwright_async_api = types.ModuleType("playwright.async_api")
            playwright_async_api.expect = fake_expect

            with (
                patch.dict(
                    sys.modules,
                    {
                        "playwright": playwright_module,
                        "playwright.async_api": playwright_async_api,
                    },
                ),
                patch.object(run_live_qa, "_with_page", new=fail_if_page_opens),
            ):
                result = asyncio.run(
                    run_live_qa._slack_connect_case(
                        ctx,
                        case_name="qa_3a_slack_connect",
                    )
                )

        self.assertFalse(result.success)
        self.assertIn("personal OAuth", str(result.details["error"]))

    def test_slack_connect_instruction_validation_accepts_oauth_copy(self):
        self.assertTrue(
            run_live_qa._slack_connect_instructions_look_valid(
                "Connect Slack with OAuth from the extension configuration, "
                "then message the Slack bot directly."
            )
        )
        self.assertFalse(
            run_live_qa._slack_connect_instructions_look_valid(
                "Message the IronClaw Reborn app in Slack to get a pairing code, "
                "then paste it here."
            )
        )

    def test_extension_is_listed_recognizes_an_existing_package(self):
        extensions = [
            {"package_ref": {"kind": "extension", "id": "github"}},
            {"package_ref": {"kind": "extension", "id": "slack"}},
        ]

        self.assertTrue(run_live_qa._extension_is_listed(extensions, "slack"))
        self.assertFalse(run_live_qa._extension_is_listed(extensions, "gmail"))

    def test_extension_install_preflight_reuses_existing_installation(self):
        async def fake_fetch(_page: object, path: str) -> dict[str, object]:
            self.assertEqual(path, "/api/webchat/v2/extensions")
            return {
                "extensions": [
                    {"package_ref": {"kind": "extension", "id": "slack"}}
                ]
            }

        async def fail_if_install_runs(*_args, **_kwargs) -> dict[str, object]:
            raise AssertionError("existing Slack installation must not be reinstalled")

        observed: dict[str, object] = {}
        with (
            patch.object(run_live_qa, "_fetch_webui_json", new=fake_fetch),
            patch.object(run_live_qa, "_webui_json", new=fail_if_install_runs),
        ):
            asyncio.run(
                run_live_qa._ensure_extension_installed_on_page(
                    object(),
                    observed,
                    package_id="slack",
                    display_name="Slack",
                )
            )

        self.assertEqual(observed["slack_install_message"], "Slack already installed")
        self.assertEqual(
            observed["slack_install_onboarding_state"],
            "existing_installation",
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
            self.assertNotIn("/extensions/registry", prompt)
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
        captured_follow_up_flags: list[bool] = []
        captured_timeouts: list[float] = []

        async def fake_live_chat_case(_ctx, **kwargs):
            captured_prompts.append(kwargs["prompt"])
            captured_follow_up_flags.append(
                kwargs.get("routine_confirmation_follow_up", False)
            )
            captured_timeouts.append(kwargs["timeout"])
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
            patch.object(run_live_qa, "_trigger_record_count", return_value=0),
            patch.object(
                run_live_qa,
                "_wait_for_trigger_record_after_count",
                return_value=(0, 25),
            ),
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
        self.assertEqual(captured_follow_up_flags, [True])
        # Plain (non-extensions) routine-creation turn must also get the
        # extended reply-wait headroom past the former 180.0 timeout.
        self.assertEqual(
            captured_timeouts,
            [run_live_qa.ROUTINE_CREATION_REPLY_TIMEOUT_SECONDS],
        )
        self.assertGreater(captured_timeouts[0], 180.0)
        self.assertEqual(result.details["trigger_records_after"], 0)
        self.assertEqual(result.details["trigger_record_wait_ms"], 25)
        self.assertIn("did not add a trigger_record", result.details["error"])

    def test_wait_for_trigger_record_after_count_polls_until_record_added(self):
        counts = iter([0, 0, 1])
        observed_sleeps: list[float] = []

        def fake_trigger_record_count(_home: Path, routine_name: str | None) -> int:
            self.assertIsNone(routine_name)
            return next(counts)

        async def fake_sleep(seconds: float) -> None:
            observed_sleeps.append(seconds)

        with (
            patch.object(
                run_live_qa,
                "_trigger_record_count",
                side_effect=fake_trigger_record_count,
            ),
            patch.object(run_live_qa.asyncio, "sleep", new=fake_sleep),
        ):
            after_count, waited_ms = asyncio.run(
                run_live_qa._wait_for_trigger_record_after_count(
                    Path("/tmp/reborn-home"),
                    None,
                    before_count=0,
                    timeout=10.0,
                    poll_interval=0.01,
                )
            )

        self.assertEqual(after_count, 1)
        self.assertGreaterEqual(len(observed_sleeps), 1)
        self.assertGreaterEqual(waited_ms, 0)

    def test_wait_for_slack_search_marker_ready_when_marker_indexed(self):
        # Slack's search index is eventually consistent: the first sweeps come
        # back empty (message not yet indexed), then the marker appears. The
        # barrier must report ready as soon as a hit lands.
        sweeps = iter(
            [
                {"checked": True, "hits": []},
                {"checked": True, "hits": []},
                {"checked": True, "hits": [{"ts": "1784422248.376"}]},
            ]
        )
        observed_sleeps: list[float] = []

        async def fake_search(_ctx, *, marker: str) -> dict[str, object]:
            self.assertEqual(marker, "LASTSENT_GLOBAL_1784422248376")
            return next(sweeps)

        async def fake_sleep(seconds: float) -> None:
            observed_sleeps.append(seconds)

        with (
            patch.object(
                run_live_qa, "_slack_search_marker_hits", side_effect=fake_search
            ),
            patch.object(run_live_qa.asyncio, "sleep", new=fake_sleep),
        ):
            readiness = asyncio.run(
                run_live_qa._wait_for_slack_search_marker(
                    self._dummy_ctx(),
                    marker="LASTSENT_GLOBAL_1784422248376",
                    timeout=10.0,
                    poll_interval=0.01,
                )
            )

        self.assertTrue(readiness.get("ready"))
        self.assertTrue(readiness.get("checked"))
        self.assertFalse(readiness.get("permanent"))
        self.assertEqual(readiness.get("attempts"), 3)
        self.assertGreaterEqual(len(observed_sleeps), 2)

    def test_wait_for_slack_search_marker_inconclusive_when_never_indexed(self):
        # The marker never becomes searchable within the bounded deadline —
        # external index lag, not an agent regression. The barrier must report
        # not-ready (non-permanent) so the caller can surface an INCONCLUSIVE
        # result instead of a spurious answer-mismatch red.
        async def fake_search(_ctx, *, marker: str) -> dict[str, object]:
            return {"checked": True, "hits": []}

        async def fake_sleep(_seconds: float) -> None:
            return None

        # Deterministic clock: each call advances 20ms past a 50ms deadline, so
        # the loop exits after a bounded number of iterations instead of
        # busy-spinning real CPU for the whole timeout window.
        clock = {"now": 0.0}

        def fake_monotonic() -> float:
            clock["now"] += 0.02
            return clock["now"]

        with (
            patch.object(
                run_live_qa, "_slack_search_marker_hits", side_effect=fake_search
            ),
            patch.object(run_live_qa.asyncio, "sleep", new=fake_sleep),
            patch.object(run_live_qa.time, "monotonic", new=fake_monotonic),
        ):
            readiness = asyncio.run(
                run_live_qa._wait_for_slack_search_marker(
                    self._dummy_ctx(),
                    marker="LASTSENT_GLOBAL_never",
                    timeout=0.05,
                    poll_interval=0.01,
                )
            )

        self.assertFalse(readiness.get("ready"))
        self.assertFalse(readiness.get("permanent"))
        self.assertGreaterEqual(readiness.get("attempts"), 1)

    def test_wait_for_slack_search_marker_permanent_when_search_cannot_run(self):
        # A permanent token/scope problem means the sweep can NEVER run here;
        # the barrier must short-circuit as permanent (not spin to the
        # deadline) so the caller can surface the real env-repair reason.
        calls = {"count": 0}

        async def fake_search(_ctx, *, marker: str) -> dict[str, object]:
            calls["count"] += 1
            return {"checked": False, "permanent": True, "error": "missing_scope"}

        async def fake_sleep(_seconds: float) -> None:
            raise AssertionError("permanent failure must not poll")

        with (
            patch.object(
                run_live_qa, "_slack_search_marker_hits", side_effect=fake_search
            ),
            patch.object(run_live_qa.asyncio, "sleep", new=fake_sleep),
        ):
            readiness = asyncio.run(
                run_live_qa._wait_for_slack_search_marker(
                    self._dummy_ctx(),
                    marker="LASTSENT_GLOBAL_perm",
                    timeout=10.0,
                    poll_interval=0.01,
                )
            )

        self.assertFalse(readiness.get("ready"))
        self.assertTrue(readiness.get("permanent"))
        self.assertEqual(calls["count"], 1)
        self.assertEqual(readiness.get("error"), "missing_scope")

    def test_routine_confirmation_follow_up_answers_timezone_confirmation(self):
        text = (
            "I'll set up a trigger every 5 minutes and send a Slack DM. "
            "I need a timezone for scheduling. Shall I go ahead and create this?"
        )

        self.assertEqual(
            run_live_qa._routine_confirmation_follow_up_for_text(text),
            "Yes, go ahead and create it. Use Europe/London (London time) "
            "for the schedule.",
        )

    def test_routine_confirmation_follow_up_ignores_slack_pairing_gate(self):
        text = (
            "Connect Slack. Message the IronClaw Reborn app in Slack to get a "
            "pairing code, then paste it here."
        )

        self.assertIsNone(run_live_qa._routine_confirmation_follow_up_for_text(text))

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
        # Heavy routine-creation turn must get the extended reply-wait headroom
        # (regression: the former hardcoded 180.0 timed out mid-work).
        self.assertEqual(
            captured["timeout"], run_live_qa.ROUTINE_CREATION_REPLY_TIMEOUT_SECONDS
        )
        self.assertGreater(captured["timeout"], 180.0)
        self.assertTrue(result.details["fixture_ready"])

    def test_required_text_accepts_explicit_alternatives(self):
        self.assertTrue(
            text_match.required_text_matches(
                "fires every 5 minutes and watches slack for bug messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertFalse(
            text_match.required_text_matches(
                "records bug messages in a sheet",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertFalse(
            text_match.required_text_matches(
                "fires every 5 minutes and watches slack for debug messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertFalse(
            text_match.required_text_matches(
                "fires every 5 minutes and watches slack for bugfix messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertTrue(
            text_match.required_text_matches(
                "fires every 5 minutes and watches slack for bug: messages",
                ["trigger|routine|automation|cron|schedule|fires|watches", "bug"],
            )
        )
        self.assertTrue(
            text_match.required_text_matches(
                "https://near.ai responded with HTTP 200 - the endpoint is up and running fine.",
                ["status|http|200|up|running|responded"],
            )
        )
        self.assertTrue(
            text_match.required_text_matches(
                "Trigger created. Schedule: every 5 minutes. Action: fetch latest releases.",
                ["routine|trigger|automation|cron|schedule|created"],
            )
        )
        self.assertTrue(
            text_match.required_text_matches(
                "The email from firat.sertgoz@near.ai is already in the sheet.",
                ["ABC|sheet|spreadsheet", "email|row|near.ai|near ai"],
            )
        )
        self.assertTrue(
            text_match.required_text_matches(
                'Discussion thread "vibe coded eh" (id=47005839) mentions NEAR AI.',
                ["news.ycombinator.com|hacker news|hn|discussion|id="],
            )
        )

    def test_forbidden_auth_phrase_ignores_positive_no_auth_required_copy(self):
        self.assertFalse(
            run_live_qa._forbidden_phrase_matches(
                "activation succeeded with no additional authentication required.",
                "authentication required",
            )
        )
        self.assertFalse(
            run_live_qa._forbidden_phrase_matches(
                "google sheets connected; no authentication required.",
                "authentication required",
            )
        )
        self.assertTrue(
            run_live_qa._forbidden_phrase_matches(
                "google sheets cannot connect because authentication required.",
                "authentication required",
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

            async def get_attribute(self, name, **_kwargs):
                if name == "data-final-reply":
                    return "true"
                return None

            async def all_inner_texts(self):
                return [
                    'The routine has been created successfully. Routine: "30-min meeting briefing"',
                    "latest news on that company\nEmails a concise briefing",
                ]

        class FakePage:
            def locator(self, selector):
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantBlocks()
                if selector == "[data-testid='msg-error']":
                    return _FakeEmptyErrorBlocks()
                raise AssertionError(f"unexpected selector: {selector}")

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        reply = asyncio.run(
            run_live_qa._wait_for_assistant_reply(
                FakePage(),
                marker=None,
                required_text=["routine", "email|emails|gmail"],
                timeout=1.0,
            )
        )

        text = reply.text_excerpt
        self.assertIn("routine", text.lower())
        self.assertIn("emails", text.lower())
        self.assertFalse(reply.semantic_judge_used)
        self.assertEqual(reply.semantic_judge_reason, "literal_required_text_matched")
        self.assertEqual(reply.final_reply_reason, "final_reply_observed")

    def test_wait_for_assistant_reply_waits_for_final_marked_message(self):
        state = {"index": 0, "sleep_calls": 0}
        responses = [
            ("I'll connect Google Calendar and get it connected.", "false"),
            ("Google Calendar connected.", "true"),
        ]

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
                return responses[state["index"]][0]

            async def get_attribute(self, name, **_kwargs):
                if name == "data-final-reply":
                    return responses[state["index"]][1]
                return None

            async def all_inner_texts(self):
                return [responses[state["index"]][0]]

        class FakePage:
            def locator(self, selector):
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantBlocks()
                if selector == "[data-testid='msg-error']":
                    return _FakeEmptyErrorBlocks()
                raise AssertionError(f"unexpected selector: {selector}")

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        async def fake_sleep(_seconds):
            state["sleep_calls"] += 1
            state["index"] = min(1, state["index"] + 1)

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fake_sleep):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    FakePage(),
                    marker=None,
                    required_text=["Google Calendar", "connected"],
                    timeout=1.0,
                )
            )

        self.assertGreaterEqual(state["sleep_calls"], 1)
        self.assertEqual(reply.text_excerpt, "Google Calendar connected.")
        self.assertEqual(reply.final_reply_reason, "final_reply_observed")

    def test_wait_for_assistant_reply_preserves_non_final_state_on_attribute_error(self):
        state = {"index": 0, "sleep_calls": 0}
        responses = [
            ("Google Calendar connected.", "false"),
            ("Google Calendar connected.", RuntimeError("transient attr failure")),
            ("Google Calendar connected.", "true"),
        ]

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
                return responses[state["index"]][0]

            async def get_attribute(self, name, **_kwargs):
                if name != "data-final-reply":
                    return None
                value = responses[state["index"]][1]
                if isinstance(value, Exception):
                    raise value
                return value

            async def all_inner_texts(self):
                return [responses[state["index"]][0]]

        class FakePage:
            def locator(self, selector):
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantBlocks()
                if selector == "[data-testid='msg-error']":
                    return _FakeEmptyErrorBlocks()
                raise AssertionError(f"unexpected selector: {selector}")

            def get_by_role(self, _role, **_kwargs):
                return FakeApprove()

        async def fake_sleep(_seconds):
            state["sleep_calls"] += 1
            state["index"] = min(len(responses) - 1, state["index"] + 1)

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fake_sleep):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    FakePage(),
                    marker=None,
                    required_text=["Google Calendar", "connected"],
                    timeout=1.0,
                )
            )

        self.assertGreaterEqual(state["sleep_calls"], 2)
        self.assertEqual(reply.text_excerpt, "Google Calendar connected.")
        self.assertEqual(reply.final_reply_reason, "final_reply_observed")

    def test_wait_for_assistant_reply_returns_final_reply_when_marker_is_not_enforced(
        self,
    ):
        response_text = "Seeded fixture content. REBORN QA 10A marker reformatted."

        async def fail_if_waits(_seconds):
            raise AssertionError("finalized reply should return without waiting")

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fail_if_waits):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page(response_text),
                    marker="REBORN_QA_10A_EXACT",
                    required_text=["seeded fixture"],
                    timeout=30.0,
                    enforce_marker=False,
                )
            )

        self.assertEqual(reply.full_text, response_text)
        self.assertFalse(reply.semantic_judge_used)
        self.assertEqual(reply.final_reply_reason, "final_reply_observed")

    def test_wait_for_assistant_reply_waits_for_post_baseline_final_reply(self):
        stale_reply = {
            "text": "Stale finalized reply.",
            "final_reply_state": "true",
        }
        current_reply = {
            "text": "Current finalized reply.",
            "final_reply_state": "true",
        }
        page, advance = self._fake_sequenced_terminal_page(
            [
                {"assistants": [stale_reply], "errors": []},
                {"assistants": [stale_reply, current_reply], "errors": []},
            ]
        )

        async def reveal_current_reply(_seconds):
            advance()

        with patch.object(
            run_live_qa.asyncio,
            "sleep",
            side_effect=reveal_current_reply,
        ):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    page,
                    marker="MUTATED_CURRENT_MARKER",
                    required_text=[],
                    timeout=30.0,
                    assistant_count_before=1,
                    error_count_before=0,
                    enforce_marker=False,
                )
            )

        self.assertEqual(reply.full_text, "Current finalized reply.")
        self.assertEqual(reply.final_reply_reason, "final_reply_observed")

    def test_wait_for_assistant_reply_new_terminal_error_beats_stale_final_reply(
        self,
    ):
        stale_reply = {
            "text": "Stale finalized reply.",
            "final_reply_state": "true",
        }
        terminal_error = {
            "summary": "The current model provider is unavailable.",
            "failure_category": "model_unavailable",
            "failure_status": "failed",
        }
        page, advance = self._fake_sequenced_terminal_page(
            [
                {"assistants": [stale_reply], "errors": []},
                {"assistants": [stale_reply], "errors": [terminal_error]},
            ]
        )

        async def reveal_terminal_error(_seconds):
            advance()

        with patch.object(
            run_live_qa.asyncio,
            "sleep",
            side_effect=reveal_terminal_error,
        ):
            with self.assertRaises(run_live_qa.TerminalRunFailure) as raised:
                asyncio.run(
                    run_live_qa._wait_for_assistant_reply(
                        page,
                        marker="MUTATED_CURRENT_MARKER",
                        required_text=[],
                        timeout=30.0,
                        assistant_count_before=1,
                        error_count_before=0,
                        enforce_marker=False,
                    )
                )

        self.assertEqual(
            raised.exception.observation.summary,
            "The current model provider is unavailable.",
        )

    def test_wait_for_assistant_reply_baseline_zero_final_reply_is_immediate(self):
        async def fail_if_waits(_seconds):
            raise AssertionError("new finalized reply should return immediately")

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fail_if_waits):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page("Current finalized reply."),
                    marker=None,
                    required_text=[],
                    timeout=30.0,
                    assistant_count_before=0,
                )
            )

        self.assertEqual(reply.full_text, "Current finalized reply.")
        self.assertEqual(reply.final_reply_reason, "final_reply_observed")

    def test_wait_for_assistant_reply_fails_immediately_when_marker_is_enforced(self):
        async def fail_if_waits(_seconds):
            raise AssertionError("finalized reply should not continue the wait loop")

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fail_if_waits):
            with self.assertRaisesRegex(
                AssertionError,
                "finalized assistant reply.*required marker",
            ):
                asyncio.run(
                    run_live_qa._wait_for_assistant_reply(
                        self._fake_assistant_reply_page("Routine created."),
                        marker="REBORN_QA_DONE",
                        required_text=["routine"],
                        timeout=30.0,
                        enforce_marker=True,
                    )
                )

    def test_wait_for_assistant_reply_enforces_marker_on_finalized_bubble_only(self):
        with self.assertRaisesRegex(
            AssertionError,
            "finalized assistant reply.*required marker",
        ):
            asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page(
                        "Routine created without the marker.",
                        assistant_block_texts=[
                            "Earlier reply REBORN_QA_DONE",
                            "Routine created without the marker.",
                        ],
                    ),
                    marker="REBORN_QA_DONE",
                    required_text=["routine"],
                    timeout=30.0,
                    enforce_marker=True,
                )
            )

    def test_wait_for_assistant_reply_raises_terminal_model_failure_without_waiting(
        self,
    ):
        errors = [
            {
                "summary": "Old request failed.",
                "failure_category": "driver_invalid_request",
                "failure_status": "failed",
            },
            {
                "summary": "The configured model provider is unavailable.",
                "failure_category": "model_unavailable",
                "failure_status": "failed",
            },
        ]

        async def fail_if_waits(_seconds):
            raise AssertionError("terminal model failure should not wait")

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fail_if_waits):
            with self.assertRaises(run_live_qa.TerminalRunFailure) as raised:
                asyncio.run(
                    run_live_qa._wait_for_assistant_reply(
                        self._fake_assistant_reply_page(
                            "",
                            final_reply_state=None,
                            error_messages=errors,
                        ),
                        marker="REBORN_QA_DONE",
                        required_text=[],
                        timeout=30.0,
                        error_count_before=1,
                    )
                )

        observation = raised.exception.observation
        self.assertEqual(
            observation.summary,
            "The configured model provider is unavailable.",
        )
        self.assertEqual(observation.failure_category, "model_unavailable")
        self.assertEqual(observation.failure_status, "failed")

    def test_wait_for_assistant_reply_ignores_stale_terminal_failure(self):
        stale_error = {
            "summary": "A previous run failed.",
            "failure_category": "model_unavailable",
            "failure_status": "failed",
        }
        response_text = "Routine created. REBORN_QA_DONE"

        async def fail_if_waits(_seconds):
            raise AssertionError("finalized reply should return without waiting")

        with patch.object(run_live_qa.asyncio, "sleep", side_effect=fail_if_waits):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page(
                        response_text,
                        error_messages=[stale_error],
                    ),
                    marker="REBORN_QA_DONE",
                    required_text=["routine"],
                    timeout=30.0,
                    error_count_before=1,
                )
            )

        self.assertEqual(reply.full_text, response_text)

    def test_wait_for_assistant_reply_retains_quiet_fallback_without_final_metadata(
        self,
    ):
        response_text = "Routine created. REBORN_QA_DONE"
        with patch.object(
            run_live_qa,
            "ASSISTANT_REPLY_FALLBACK_QUIET_SECONDS",
            0.0,
        ):
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page(
                        response_text,
                        final_reply_state=None,
                    ),
                    marker="REBORN_QA_DONE",
                    required_text=["routine"],
                    timeout=1.0,
                )
            )

        self.assertEqual(reply.full_text, response_text)
        self.assertEqual(reply.final_reply_reason, "fallback_quiet_period_matched")

    def test_live_chat_case_persists_terminal_failure_metadata(self):
        captured_wait: dict[str, object] = {}

        class FakeComposer:
            async def fill(self, _text):
                return None

            async def press(self, _key):
                return None

        class FakeUserMessages:
            @property
            def last(self):
                return self

        class FakeDismiss:
            @property
            def first(self):
                return self

            async def count(self):
                return 0

        class FakeErrors:
            async def count(self):
                return 2

        class FakeAssistantMessages:
            async def count(self):
                return 1

        class FakePage:
            async def goto(self, _url, **_kwargs):
                return None

            def locator(self, selector):
                if selector == "[aria-label='Dismiss connect action']":
                    return FakeDismiss()
                if selector == "[data-testid='chat-composer']":
                    return FakeComposer()
                if selector == "[data-testid='msg-user']":
                    return FakeUserMessages()
                if selector == "[data-testid='msg-error']":
                    return FakeErrors()
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantMessages()
                raise AssertionError(f"unexpected selector: {selector}")

        class FakeExpectation:
            async def to_be_visible(self, **_kwargs):
                return None

            async def to_contain_text(self, _text, **_kwargs):
                return None

        def fake_expect(_locator):
            return FakeExpectation()

        async def fake_with_page(_output_dir, _case_name, action):
            await action(FakePage())

        async def fake_wait(_page, **kwargs):
            captured_wait.update(kwargs)
            observation = run_live_qa.TerminalRunFailureObservation(
                summary="The configured model provider is unavailable.",
                failure_category="model_unavailable",
                failure_status="failed",
            )
            raise run_live_qa.TerminalRunFailure(observation)

        playwright_module = types.ModuleType("playwright")
        playwright_async_api = types.ModuleType("playwright.async_api")
        playwright_async_api.expect = fake_expect
        with (
            patch.dict(
                sys.modules,
                {
                    "playwright": playwright_module,
                    "playwright.async_api": playwright_async_api,
                },
            ),
            patch.object(run_live_qa, "_with_page", new=fake_with_page),
            patch.object(run_live_qa, "_wait_for_assistant_reply", new=fake_wait),
        ):
            result = asyncio.run(
                run_live_qa._live_chat_case(
                    self._dummy_ctx(),
                    case_name="qa_test_terminal_failure",
                    prompt="Run a live model turn.",
                    marker="REBORN_QA_DONE",
                    required_text=[],
                )
            )

        self.assertFalse(result.success)
        self.assertEqual(captured_wait["assistant_count_before"], 1)
        self.assertEqual(captured_wait["error_count_before"], 2)
        self.assertEqual(result.details["failure_category"], "model_unavailable")
        self.assertEqual(result.details["failure_status"], "failed")

    def test_live_chat_case_refreshes_assistant_baseline_before_confirmation_submit(
        self,
    ):
        events: list[str] = []
        assistant_counts = iter([1, 2])
        captured_baselines: list[int] = []
        wait_calls = 0

        class FakeComposer:
            async def fill(self, _text):
                return None

            async def press(self, _key):
                events.append("submit")

        class FakeUserMessages:
            @property
            def last(self):
                return self

        class FakeDismiss:
            @property
            def first(self):
                return self

            async def count(self):
                return 0

        class FakeErrors:
            async def count(self):
                return 0

        class FakeAssistantMessages:
            async def count(self):
                count = next(assistant_counts)
                events.append(f"assistant_baseline:{count}")
                return count

        class FakePage:
            async def goto(self, _url, **_kwargs):
                return None

            def locator(self, selector):
                if selector == "[aria-label='Dismiss connect action']":
                    return FakeDismiss()
                if selector == "[data-testid='chat-composer']":
                    return FakeComposer()
                if selector == "[data-testid='msg-user']":
                    return FakeUserMessages()
                if selector == "[data-testid='msg-error']":
                    return FakeErrors()
                if selector == "[data-testid='msg-assistant']":
                    return FakeAssistantMessages()
                raise AssertionError(f"unexpected selector: {selector}")

        class FakeExpectation:
            async def to_be_visible(self, **_kwargs):
                return None

            async def to_contain_text(self, _text, **_kwargs):
                return None

        def fake_expect(_locator):
            return FakeExpectation()

        async def fake_with_page(_output_dir, _case_name, action):
            await action(FakePage())

        async def fake_wait(_page, **kwargs):
            nonlocal wait_calls
            wait_calls += 1
            captured_baselines.append(kwargs["assistant_count_before"])
            text = (
                "Would you like me to create the routine?"
                if wait_calls == 1
                else "Routine created."
            )
            return run_live_qa.AssistantReplyWaitResult(
                text_excerpt=text,
                full_text=text,
                semantic_judge_used=False,
                semantic_judge_reason="literal_required_text_matched",
                final_reply_wait_ms=0,
                final_reply_reason="final_reply_observed",
            )

        playwright_module = types.ModuleType("playwright")
        playwright_async_api = types.ModuleType("playwright.async_api")
        playwright_async_api.expect = fake_expect
        with (
            patch.dict(
                sys.modules,
                {
                    "playwright": playwright_module,
                    "playwright.async_api": playwright_async_api,
                },
            ),
            patch.object(run_live_qa, "_with_page", new=fake_with_page),
            patch.object(run_live_qa, "_wait_for_assistant_reply", new=fake_wait),
        ):
            result = asyncio.run(
                run_live_qa._live_chat_case(
                    self._dummy_ctx(),
                    case_name="qa_test_confirmation_baseline",
                    prompt="Create a routine.",
                    marker=None,
                    required_text=["routine"],
                    routine_confirmation_follow_up=True,
                )
            )

        self.assertTrue(result.success)
        self.assertEqual(captured_baselines, [1, 2])
        self.assertEqual(
            events,
            ["assistant_baseline:1", "submit", "assistant_baseline:2", "submit"],
        )

    def test_slack_correctness_chat_reply_does_not_enforce_answer_marker(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured.update(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live:qa_10_test",
                success=True,
                latency_ms=1,
                details={"full_reply_text": "Seeded Slack fixture content."},
            )

        with (
            patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ),
            patch.object(
                run_live_qa,
                "_capability_run_statuses",
            ) as capability_statuses,
        ):
            chat, reply_text = asyncio.run(
                run_live_qa._slack_correctness_chat_reply(
                    self._dummy_ctx(),
                    case_name="qa_10_test",
                    started=0.0,
                    prompt="Read the seeded Slack fixture.",
                    answer_marker="REBORN_QA_10_TEST_EXACT",
                    extra_details={},
                )
            )

        self.assertTrue(chat.success)
        self.assertEqual(reply_text, "Seeded Slack fixture content.")
        self.assertIs(captured["enforce_marker"], False)
        self.assertEqual(
            run_live_qa.SLACK_EXTENSION_REQUIREMENT,
            {
                "package_id": "slack",
                "display_name": "Slack",
                "required_tools": [
                    "slack.list_conversations",
                    "slack.get_conversation_info",
                    "slack.get_conversation_history",
                ],
            },
        )
        self.assertEqual(
            captured["extensions"],
            [run_live_qa.SLACK_EXTENSION_REQUIREMENT],
        )
        capability_statuses.assert_not_called()

    def test_live_chat_case_preactivates_extensions_before_chat_submit(self):
        events: list[str] = []
        authenticated: list[dict[str, object]] = []

        class FakeComposer:
            async def fill(self, _text):
                events.append("fill")

            async def press(self, _key):
                events.append("submit")

        class FakeMessages:
            @property
            def last(self):
                return self

            async def count(self):
                return 0

        class FakeDismiss:
            @property
            def first(self):
                return self

            async def count(self):
                return 0

        class FakePage:
            class FakeRequest:
                method = "POST"
                post_data_json = {"content": "Read Slack."}

            class FakeResponse:
                url = (
                    "http://127.0.0.1:9/api/webchat/v2/threads/"
                    "thread-current/messages"
                )
                request = None

                async def json(self):
                    return {
                        "outcome": "submitted",
                        "thread_id": "thread-current",
                        "accepted_message_ref": "msg:message-current",
                        "turn_id": "turn-current",
                        "run_id": "run-current",
                    }

            class FakeResponseInfo:
                def __init__(self, response):
                    self.response = response

                async def __aenter__(self):
                    return self

                async def __aexit__(self, *_args):
                    return None

                @property
                def value(self):
                    async def get_response():
                        return self.response

                    return get_response()

            async def goto(self, url, **_kwargs):
                events.append(f"goto:{url}")

            def expect_response(self, predicate, **_kwargs):
                response = self.FakeResponse()
                response.request = self.FakeRequest()
                self_outer.assertTrue(predicate(response))
                return self.FakeResponseInfo(response)

            def locator(self, selector):
                if selector == "body":
                    return object()
                if selector == "[aria-label='Dismiss connect action']":
                    return FakeDismiss()
                if selector == "[data-testid='chat-composer']":
                    return FakeComposer()
                if selector in (
                    "[data-testid='msg-user']",
                    "[data-testid='msg-error']",
                    "[data-testid='msg-assistant']",
                ):
                    return FakeMessages()
                raise AssertionError(f"unexpected selector: {selector}")

        class FakeExpectation:
            async def to_be_visible(self, **_kwargs):
                return None

            async def to_contain_text(self, _text, **_kwargs):
                return None

        async def fake_with_page(_output_dir, _case_name, action):
            await action(FakePage())

        async def fake_ensure(_page, observed, **kwargs):
            events.append(f"authenticate:{kwargs['package_id']}")
            authenticated.append({"observed": observed, **kwargs})

        async def fake_wait(_page, **_kwargs):
            return run_live_qa.AssistantReplyWaitResult(
                text_excerpt="Slack fixture answer.",
                full_text="Slack fixture answer.",
                semantic_judge_used=False,
                semantic_judge_reason="literal_required_text_matched",
                final_reply_wait_ms=0,
                final_reply_reason="final_reply_observed",
            )

        playwright_module = types.ModuleType("playwright")
        playwright_async_api = types.ModuleType("playwright.async_api")
        playwright_async_api.expect = lambda _locator: FakeExpectation()
        self_outer = self
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
                "_ensure_extension_authenticated_on_page",
                new=fake_ensure,
            ),
            patch.object(run_live_qa, "_wait_for_assistant_reply", new=fake_wait),
        ):
            result = asyncio.run(
                run_live_qa._live_chat_case(
                    self._dummy_ctx(),
                    case_name="qa_slack_preactivation_test",
                    prompt="Read Slack.",
                    marker=None,
                    required_text=[],
                    extensions=[run_live_qa.SLACK_EXTENSION_REQUIREMENT],
                    enforce_marker=False,
                    capture_submission_identity=True,
                )
            )

        self.assertTrue(result.success)
        self.assertEqual(len(authenticated), 1)
        self.assertEqual(
            {key: value for key, value in authenticated[0].items() if key != "observed"},
            {
                "package_id": "slack",
                "display_name": "Slack",
                "required_tools": [
                    "slack.list_conversations",
                    "slack.get_conversation_info",
                    "slack.get_conversation_history",
                ],
                "ensure_installed": True,
            },
        )
        self.assertEqual(
            result.details["extensions"],
            ["slack"],
        )
        self.assertEqual(
            result.details["submission_identity"],
            {
                "accepted_message_ref": "msg:message-current",
                "thread_id": "thread-current",
                "turn_id": "turn-current",
                "run_id": "run-current",
            },
        )
        registry_index = events.index(
            f"goto:{self._dummy_ctx().base_url}/extensions/registry?token="
            f"{run_live_qa.AUTH_TOKEN}"
        )
        auth_index = events.index("authenticate:slack")
        chat_index = events.index(
            f"goto:{self._dummy_ctx().base_url}/?token={run_live_qa.AUTH_TOKEN}"
        )
        submit_index = events.index("submit")
        self.assertLess(registry_index, auth_index)
        self.assertLess(auth_index, chat_index)
        self.assertLess(chat_index, submit_index)

    def test_submission_capture_retries_once_after_no_post_and_connect_dismissal(self):
        submitted = {
            "outcome": "submitted",
            "thread_id": "thread-current",
            "accepted_message_ref": "msg:message-current",
            "turn_id": "turn-current",
            "run_id": "run-current",
        }
        result, state = self._drive_submission_capture_state(
            response_payloads=[None, submitted]
        )

        self.assertTrue(result.success)
        self.assertEqual(state["presses"], 2)
        self.assertEqual(state["dismissals"], 1)
        self.assertEqual(result.details["submission_identity"]["run_id"], "run-current")

    def test_submission_capture_recovers_already_submitted_retry_identity(self):
        replay = {
            "outcome": "already_submitted",
            "thread_id": "thread-current",
            "accepted_message_ref": "msg:message-current",
            "run_id": "run-current",
            "status": "running",
            "event_cursor": 42,
        }
        result, state = self._drive_submission_capture_state(
            response_payloads=[None, replay]
        )

        self.assertTrue(result.success)
        self.assertEqual(state["presses"], 2)
        self.assertEqual(state["dismissals"], 1)
        self.assertEqual(
            result.details["submission_identity"],
            {
                "accepted_message_ref": "msg:message-current",
                "thread_id": "thread-current",
                "run_id": "run-current",
            },
        )

    def test_submission_capture_rejects_busy_retry_without_prior_identity(self):
        rejected_busy = {
            "outcome": "rejected_busy",
            "thread_id": "thread-current",
            "accepted_message_ref": "msg:message-rejected",
            "active_run_id": "run-blocking",
        }
        result, state = self._drive_submission_capture_state(
            response_payloads=[None, rejected_busy]
        )

        self.assertFalse(result.success)
        self.assertEqual(state["presses"], 2)
        self.assertEqual(state["dismissals"], 1)
        self.assertNotIn("submission_identity", result.details)
        self.assertIn("rejected_busy", result.details["error"])

    def test_submitted_ack_survives_missing_user_bubble_without_second_enter(self):
        submitted = {
            "outcome": "submitted",
            "thread_id": "thread-first",
            "accepted_message_ref": "msg:message-first",
            "turn_id": "turn-first",
            "run_id": "run-first",
        }
        result, state = self._drive_submission_capture_state(
            response_payloads=[submitted, submitted],
            user_bubble_failures=1,
        )

        self.assertTrue(result.success)
        self.assertEqual(state["presses"], 1)
        self.assertEqual(state["dismissals"], 0)
        self.assertEqual(result.details["submission_identity"]["run_id"], "run-first")

    def test_submitted_run_terminal_provider_error_wins_without_retry(self):
        submitted = {
            "outcome": "submitted",
            "thread_id": "thread-first",
            "accepted_message_ref": "msg:message-first",
            "turn_id": "turn-first",
            "run_id": "run-first",
        }
        terminal = run_live_qa.TerminalRunFailureObservation(
            summary="The model provider is transiently unavailable.",
            failure_category="model_transient",
            failure_status="failed",
        )
        result, state = self._drive_submission_capture_state(
            response_payloads=[submitted, submitted],
            user_bubble_failures=1,
            terminal_failure=terminal,
        )

        self.assertFalse(result.success)
        self.assertEqual(state["presses"], 1)
        self.assertEqual(result.details["failure_category"], "model_transient")
        self.assertEqual(result.details["failure_status"], "failed")
        self.assertEqual(result.details["submission_identity"]["run_id"], "run-first")

    def test_submission_identity_rejects_distinct_submitted_acknowledgements(self):
        record_identity = getattr(run_live_qa, "_record_submitted_identity", None)
        self.assertIsNotNone(
            record_identity,
            "submission capture must expose one immutable identity transition",
        )
        if record_identity is None:
            return
        observed: dict[str, object] = {}
        first = {
            "outcome": "submitted",
            "thread_id": "thread-first",
            "accepted_message_ref": "msg:message-first",
            "turn_id": "turn-first",
            "run_id": "run-first",
        }
        second = {
            "outcome": "submitted",
            "thread_id": "thread-second",
            "accepted_message_ref": "msg:message-second",
            "turn_id": "turn-second",
            "run_id": "run-second",
        }

        record_identity(observed, first)
        with self.assertRaisesRegex(AssertionError, "ambiguous submission identity"):
            record_identity(observed, second)
        self.assertEqual(observed["submission_identity"]["run_id"], "run-first")

    def test_live_chat_with_extensions_delegates_to_shared_live_chat_case(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured.update(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"text_excerpt": "done"},
            )

        with patch.object(
            run_live_qa,
            "_live_chat_case",
            side_effect=fake_live_chat_case,
        ):
            result = asyncio.run(
                run_live_qa._live_chat_with_extensions_case(
                    self._dummy_ctx(),
                    case_name="qa_extension_delegate_test",
                    prompt="Use Slack.",
                    marker="DONE",
                    required_text=["done"],
                    extensions=[run_live_qa.SLACK_EXTENSION_REQUIREMENT],
                    timeout=31.0,
                    extra_details={"fixture": "ready"},
                    forbidden_text=["failed"],
                )
            )

        self.assertTrue(result.success)
        self.assertEqual(captured["extensions"], [run_live_qa.SLACK_EXTENSION_REQUIREMENT])
        self.assertEqual(captured["timeout"], 31.0)
        self.assertEqual(captured["extra_details"], {"fixture": "ready"})
        self.assertEqual(captured["forbidden_text"], ["failed"])

    def test_qa_10d_requires_a_new_completed_list_conversations_call(self):
        membership_view = {
            "ok": True,
            "member_channels": [{"id": "C0MEMBER01", "name": "general"}],
            "member_channel_ids": ["C0MEMBER01"],
            "listed": [
                {"id": "C0MEMBER01", "name": "general", "is_member": True},
                {"id": "C0OUTSIDE1", "name": "random", "is_member": False},
            ],
        }

        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"full_reply_text": "general"},
            )

        def drive(statuses):
            evidence = {
                "accepted_message_ref": "msg:test",
                "thread_id": "thread-test",
                "turn_id": "turn-test",
                "run_id": "run-test",
                "invocation_ids": {
                    "slack.list_conversations": ["invocation-test"]
                    if statuses
                    else []
                },
                "statuses": {"slack.list_conversations": statuses},
            }
            with (
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_token",
                    return_value="xoxp-unit-test",
                ),
                patch.object(
                    run_live_qa,
                    "_slack_membership_view",
                    return_value=membership_view,
                ),
                patch.object(
                    run_live_qa,
                    "_live_chat_case",
                    side_effect=fake_live_chat_case,
                ),
                patch.object(
                    run_live_qa,
                    "_current_turn_capability_evidence",
                    return_value=evidence,
                ),
            ):
                return asyncio.run(
                    run_live_qa.case_qa_10d_slack_channel_membership(
                        self._dummy_ctx()
                    )
                )

        no_new_call = drive([])
        completed_call = drive(["completed"])

        self.assertFalse(no_new_call.success)
        self.assertEqual(no_new_call.details["failure_class"], "model_quality")
        self.assertEqual(
            no_new_call.details["expected_capabilities"],
            ["slack.list_conversations"],
        )
        self.assertEqual(
            no_new_call.details["capability_evidence"]["statuses"],
            {"slack.list_conversations": []},
        )
        self.assertTrue(completed_call.success)
        self.assertEqual(completed_call.details["member_channels_named"], ["general"])
        self.assertEqual(completed_call.details["non_member_channels_claimed"], [])

    def test_slack_correctness_capability_evidence_is_bound_to_submitted_turn(self):
        capability_id = "slack.get_conversation_history"
        prompt = "Read the exact Slack fixture for CURRENT_TURN_123."

        def create_store(reborn_home: Path) -> Path:
            db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
            db_path.parent.mkdir(parents=True)
            with closing(sqlite3.connect(db_path)) as db:
                db.execute(
                    """
                    CREATE TABLE root_filesystem_entries (
                        path TEXT PRIMARY KEY,
                        contents BLOB NOT NULL,
                        is_dir INTEGER NOT NULL DEFAULT 0,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        content_type TEXT NOT NULL,
                        kind TEXT,
                        indexed TEXT NOT NULL DEFAULT '{}',
                        version INTEGER NOT NULL DEFAULT 0
                    )
                    """
                )
                db.execute(
                    """
                    CREATE TABLE root_filesystem_events (
                        seq INTEGER PRIMARY KEY AUTOINCREMENT,
                        path TEXT NOT NULL,
                        payload BLOB NOT NULL,
                        created_at TEXT NOT NULL
                    )
                    """
                )
            return db_path

        def write_capability_execution(
            db_path: Path,
            *,
            thread_id: str,
            run_id: str,
            invocation_id: str,
        ) -> None:
            with closing(sqlite3.connect(db_path)) as db:
                timestamp = "2026-07-12T00:00:00.000Z"
                event = {
                    "kind": "capability_activity_succeeded",
                    "scope": {
                        "thread_id": thread_id,
                        "invocation_id": invocation_id,
                    },
                    "parent_invocation_id": run_id,
                    "capability_id": capability_id,
                }
                db.execute(
                    """
                    INSERT INTO root_filesystem_events (path, payload, created_at)
                    VALUES ('/events/runtime/test', ?, ?)
                    """,
                    (json.dumps(event), timestamp),
                )
                run_state = {
                    "invocation_id": invocation_id,
                    "capability_id": capability_id,
                    "scope": {"thread_id": thread_id},
                    "status": "completed",
                }
                db.execute(
                    """
                    INSERT INTO root_filesystem_entries (
                        path, contents, is_dir, created_at, updated_at,
                        content_type, kind
                    ) VALUES (?, ?, 0, ?, ?, 'application/json', 'run_state_record')
                    """,
                    (
                        f"/run-state/threads/{thread_id}/runs/{invocation_id}.json",
                        json.dumps(run_state),
                        timestamp,
                        timestamp,
                    ),
                )
                preview = {
                    "invocation_id": invocation_id,
                    "capability_id": capability_id,
                    "status": "completed",
                    "input_summary": json.dumps(
                        {"channel": "C0CURRENT1"}, indent=2
                    ),
                }
                thread_message = {
                    "message_id": f"message-{invocation_id}",
                    "thread_id": thread_id,
                    "kind": "capability_display_preview",
                    "turn_run_id": run_id,
                    "content": json.dumps(preview),
                }
                db.execute(
                    """
                    INSERT INTO root_filesystem_entries (
                        path, contents, is_dir, created_at, updated_at,
                        content_type, kind
                    ) VALUES (?, ?, 0, ?, ?, 'application/json', 'thread_message')
                    """,
                    (
                        f"/threads/{thread_id}/messages/{invocation_id}.json",
                        json.dumps(thread_message),
                        timestamp,
                        timestamp,
                    ),
                )
                db.commit()

        def drive(
            *,
            before: dict[str, str] | None = None,
            during: dict[str, str] | None = None,
            include_turn_id: bool = True,
        ) -> run_live_qa.ProbeResult:
            with tempfile.TemporaryDirectory() as tmpdir:
                reborn_home = Path(tmpdir) / "reborn-home"
                db_path = create_store(reborn_home)
                if before is not None:
                    write_capability_execution(db_path, **before)

                async def fake_live_chat_case(_ctx, **_kwargs):
                    if during is not None:
                        write_capability_execution(db_path, **during)
                    submission_identity = {
                        "accepted_message_ref": "msg:message-current",
                        "thread_id": "thread-current",
                        "run_id": "run-current",
                    }
                    if include_turn_id:
                        submission_identity["turn_id"] = "turn-current"
                    return run_live_qa.ProbeResult(
                        provider="test",
                        mode="live:qa_10_turn_binding",
                        success=True,
                        latency_ms=1,
                        details={
                            "full_reply_text": "CURRENT_TURN_123",
                            "submission_identity": submission_identity,
                        },
                    )

                ctx = self._dummy_ctx()
                ctx.reborn_home = reborn_home
                with patch.object(
                    run_live_qa,
                    "_live_chat_case",
                    side_effect=fake_live_chat_case,
                ):
                    result, _ = asyncio.run(
                        run_live_qa._slack_correctness_chat_reply(
                            ctx,
                            case_name="qa_10_turn_binding",
                            started=run_live_qa.time.monotonic(),
                            prompt=prompt,
                            answer_marker="ANSWER_MARKER",
                            extra_details={},
                            expected_capability=capability_id,
                        )
                    )
                return result

        stale = drive(
            before={
                "thread_id": "thread-stale",
                "run_id": "run-stale",
                "invocation_id": "invocation-stale",
            }
        )
        unrelated_concurrent = drive(
            during={
                "thread_id": "thread-background",
                "run_id": "run-background",
                "invocation_id": "invocation-background",
            }
        )
        current = drive(
            during={
                "thread_id": "thread-current",
                "run_id": "run-current",
                "invocation_id": "invocation-current",
            }
        )
        replay_current = drive(
            during={
                "thread_id": "thread-current",
                "run_id": "run-current",
                "invocation_id": "invocation-current",
            },
            include_turn_id=False,
        )

        self.assertFalse(stale.success)
        self.assertEqual(stale.details["failure_category"], "missing_expected_capability")
        self.assertFalse(unrelated_concurrent.success)
        self.assertEqual(
            unrelated_concurrent.details["failure_category"],
            "missing_expected_capability",
        )
        self.assertTrue(current.success)
        self.assertEqual(
            current.details["capability_evidence"],
            {
                "accepted_message_ref": "msg:message-current",
                "thread_id": "thread-current",
                "turn_id": "turn-current",
                "run_id": "run-current",
                "invocation_ids": {capability_id: ["invocation-current"]},
                "statuses": {capability_id: ["completed"]},
                "input_arguments": {
                    capability_id: [{"channel": "C_REDACTED"}]
                },
                "terminal_sequence": [
                    {
                        "seq": 1,
                        "capability_id": capability_id,
                        "invocation_id": "invocation-current",
                        "status": "completed",
                    }
                ],
            },
        )
        self.assertTrue(replay_current.success)
        self.assertEqual(
            replay_current.details["capability_evidence"]["statuses"],
            {capability_id: ["completed"]},
        )

    def test_current_turn_capability_evidence_reports_sqlite_read_failure(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            reborn_home = Path(tmpdir)
            db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
            db_path.parent.mkdir(parents=True)
            db_path.touch()

            with patch.object(
                run_live_qa.sqlite3,
                "connect",
                side_effect=sqlite3.OperationalError("database is locked"),
            ) as connect:
                evidence = run_live_qa._current_turn_capability_evidence(
                    reborn_home,
                    {
                        "accepted_message_ref": "msg:current",
                        "thread_id": "thread-current",
                        "turn_id": "turn-current",
                        "run_id": "run-current",
                    },
                    ["slack.get_conversation_history"],
                    {"completed"},
                )

        connection_target, = connect.call_args.args
        self.assertTrue(str(connection_target).startswith("file:"))
        self.assertIn("mode=ro", str(connection_target))
        self.assertTrue(connect.call_args.kwargs["uri"])
        self.assertEqual(evidence["read_error"], "database is locked")
        self.assertEqual(
            evidence["statuses"],
            {"slack.get_conversation_history": []},
        )

    def test_slack_correctness_evidence_read_failure_is_inconclusive(self):
        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={
                    "full_reply_text": "Slack result",
                    "submission_identity": {
                        "accepted_message_ref": "msg:current",
                        "thread_id": "thread-current",
                        "turn_id": "turn-current",
                        "run_id": "run-current",
                    },
                },
            )

        evidence = {
            "accepted_message_ref": "msg:current",
            "thread_id": "thread-current",
            "turn_id": "turn-current",
            "run_id": "run-current",
            "invocation_ids": {"slack.get_conversation_history": []},
            "statuses": {"slack.get_conversation_history": []},
            "read_error": "database is locked",
        }
        with (
            patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ),
            patch.object(
                run_live_qa,
                "_current_turn_capability_evidence",
                return_value=evidence,
            ),
        ):
            chat, reply_text = asyncio.run(
                run_live_qa._slack_correctness_chat_reply(
                    self._dummy_ctx(),
                    case_name="qa_10_evidence_read_failure_test",
                    started=run_live_qa.time.monotonic(),
                    prompt="Read Slack.",
                    answer_marker="ANSWER_MARKER",
                    extra_details={},
                    expected_capability="slack.get_conversation_history",
                )
            )

        self.assertFalse(chat.success)
        self.assertEqual(reply_text, "Slack result")
        self.assertEqual(chat.details["failure_class"], "infrastructure")
        self.assertEqual(
            chat.details["failure_category"],
            "capability_evidence_unavailable",
        )
        self.assertEqual(chat.details["failure_status"], "inconclusive")
        self.assertTrue(chat.details["inconclusive"])
        self.assertFalse(chat.details["blocking"])

    def test_slack_correctness_requires_lookup_before_write(self):
        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={
                    "full_reply_text": "posted",
                    "submission_identity": {
                        "accepted_message_ref": "msg:current",
                        "thread_id": "thread-current",
                        "turn_id": "turn-current",
                        "run_id": "run-current",
                    },
                },
            )

        def drive(
            sequence: list[str],
            *,
            lookup_channel: str = "D0EXPECTED1",
        ) -> run_live_qa.ProbeResult:
            evidence = {
                "accepted_message_ref": "msg:current",
                "thread_id": "thread-current",
                "turn_id": "turn-current",
                "run_id": "run-current",
                "invocation_ids": {
                    "slack.get_conversation_info": ["invocation-lookup"],
                    "slack.send_message": ["invocation-send"],
                },
                "statuses": {
                    "slack.get_conversation_info": ["completed"],
                    "slack.send_message": ["completed"],
                },
                "input_arguments": {
                    "slack.get_conversation_info": [
                        {"channel": lookup_channel}
                    ],
                    "slack.send_message": [{"channel": "D0EXPECTED1"}],
                },
                "terminal_sequence": [
                    {
                        "seq": index + 1,
                        "capability_id": capability_id,
                        "invocation_id": f"invocation-{index + 1}",
                        "status": "completed",
                    }
                    for index, capability_id in enumerate(sequence)
                ],
            }
            with (
                patch.object(
                    run_live_qa,
                    "_live_chat_case",
                    side_effect=fake_live_chat_case,
                ),
                patch.object(
                    run_live_qa,
                    "_current_turn_capability_evidence",
                    return_value=evidence,
                ),
            ):
                result, _ = asyncio.run(
                    run_live_qa._slack_correctness_chat_reply(
                        self._dummy_ctx(),
                        case_name="qa_10_order_test",
                        started=run_live_qa.time.monotonic(),
                        prompt="Resolve the DM, then send the message.",
                        answer_marker="ANSWER_MARKER",
                        extra_details={},
                        expected_capability="slack.get_conversation_info",
                        expected_capability_sequence=(
                            "slack.get_conversation_info",
                            "slack.send_message",
                        ),
                        expected_capability_arguments={
                            "slack.get_conversation_info": {
                                "channel": "D0EXPECTED1"
                            }
                        },
                    )
                )
            return result

        ordered = drive(
            ["slack.get_conversation_info", "slack.send_message"]
        )
        reversed_order = drive(
            ["slack.send_message", "slack.get_conversation_info"]
        )
        wrong_lookup = drive(
            ["slack.get_conversation_info", "slack.send_message"],
            lookup_channel="D0WRONG001",
        )

        self.assertTrue(ordered.success)
        self.assertFalse(reversed_order.success)
        self.assertEqual(
            reversed_order.details["failure_category"],
            "unexpected_capability_order",
        )
        self.assertFalse(wrong_lookup.success)
        self.assertEqual(
            wrong_lookup.details["failure_category"],
            "unexpected_capability_arguments",
        )

    def test_qa_10e_prompt_echo_without_current_turn_history_call_fails(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured.update(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live:qa_10e_slack_error_honesty",
                success=True,
                latency_ms=1,
                details={
                    "full_reply_text": "The prompt says channel_not_found.",
                    "prompt": kwargs["prompt"],
                },
            )

        with tempfile.TemporaryDirectory() as tmpdir:
            ctx = self._dummy_ctx()
            ctx.reborn_home = Path(tmpdir) / "reborn-home"
            with patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ):
                result = asyncio.run(
                    run_live_qa.case_qa_10e_slack_error_honesty(ctx)
                )

        self.assertFalse(result.success)
        self.assertEqual(result.details["failure_class"], "model_quality")
        self.assertEqual(
            result.details["failure_category"],
            "missing_expected_capability",
        )
        self.assertEqual(captured["prompt"], result.details["prompt"])

    def test_blocking_qa_10_cases_declare_intended_slack_capability(self):
        captured: dict[str, tuple[str | None, tuple[str, ...]]] = {}

        async def fake_chat_reply(_ctx, **kwargs):
            captured[str(kwargs["case_name"])] = (
                kwargs.get("expected_capability"),
                tuple(kwargs.get("accept_any_capability") or ()),
            )
            return (
                run_live_qa.ProbeResult(
                    provider="test",
                    mode=f"live:{kwargs['case_name']}",
                    success=False,
                    latency_ms=1,
                    details={"error": "stop after capability declaration"},
                ),
                "",
            )

        membership_view = {
            "ok": True,
            "member_channels": [{"id": "C0MEMBER01", "name": "general"}],
            "member_channel_ids": ["C0MEMBER01"],
            "listed": [
                {"id": "C0MEMBER01", "name": "general", "is_member": True},
                {"id": "C0OUTSIDE1", "name": "random", "is_member": False},
            ],
        }
        with (
            patch.object(run_live_qa, "_require_slack_personal_token", return_value="xoxp-test"),
            patch.object(run_live_qa, "_require_slack_bot_token", return_value="xoxb-test"),
            patch.object(run_live_qa, "_require_slack_personal_bot_dm_channel", return_value="D0FIXTURE1"),
            patch.object(run_live_qa, "_seed_slack_fixture_message", return_value="1.000001"),
            patch.object(run_live_qa, "_slack_auth_identity", return_value={"ok": True, "user_id": "U0SELF001"}),
            patch.object(run_live_qa, "_slack_user_status_text", return_value={"ok": True, "status_text": "OOO-CANARY-FIXTURE"}),
            patch.object(run_live_qa, "_slack_membership_view", return_value=membership_view),
            patch.object(run_live_qa, "_slack_dm_counterpart", return_value={"ok": True, "display_name": "Canary Person", "own_user_id": "U0SELF001", "user_id": "U0OTHER01"}),
            patch.object(run_live_qa, "_slack_correctness_chat_reply", side_effect=fake_chat_reply),
        ):
            ctx = self._dummy_ctx()
            for case_fn in (
                run_live_qa.case_qa_10a_slack_self_attribution,
                run_live_qa.case_qa_10b_slack_ooo_status,
                run_live_qa.case_qa_10c_slack_thread_replies,
                run_live_qa.case_qa_10d_slack_channel_membership,
                run_live_qa.case_qa_10e_slack_error_honesty,
                run_live_qa.case_qa_10f_slack_mention_encoding,
                run_live_qa.case_qa_10g_slack_last_message_sent,
                run_live_qa.case_qa_10h_slack_email_hallucination_guard,
            ):
                asyncio.run(case_fn(ctx))

        self.assertEqual(
            captured,
            {
                "qa_10a_slack_self_attribution": (
                    "slack.get_conversation_history",
                    (),
                ),
                "qa_10b_slack_ooo_status": ("slack.get_user_info", ()),
                # 10C asserts an OUTCOME (thread replies surfaced), so it pins
                # no single tool id — it accepts any capability that can
                # retrieve the replies.
                "qa_10c_slack_thread_replies": (
                    None,
                    (
                        "slack.get_thread_replies",
                        "slack.search_messages",
                    ),
                ),
                "qa_10d_slack_channel_membership": (
                    "slack.list_conversations",
                    (),
                ),
                "qa_10e_slack_error_honesty": (
                    "slack.get_conversation_history",
                    (),
                ),
                "qa_10f_slack_mention_encoding": (
                    "slack.get_conversation_info",
                    (),
                ),
                "qa_10g_slack_last_message_sent": (
                    "slack.get_conversation_history",
                    (),
                ),
                "qa_10h_slack_email_hallucination_guard": (
                    "slack.get_user_info",
                    (),
                ),
            },
        )

    def test_qa_10c_thread_replies_asserts_outcome_not_tool_identity(self):
        # Regression for the 10C flake: the arm asserts an OUTCOME (the seeded
        # thread replies appear in the answer), not TOOL IDENTITY. It must pass
        # when the model surfaces the replies through indexed search
        # (never touching a dedicated get_thread_replies tool), still fail when
        # the replies are not surfaced (real thread-visibility regression), and
        # not turn accept-any into a blanket bypass when no retrieval
        # capability produced terminal evidence.
        seeded: list[str] = []

        async def fake_seed(_token, _channel, text, **_kwargs):
            seeded.append(str(text))
            return f"{len(seeded)}.000001"

        def drive(
            *, surface_replies: bool, evidence: dict
        ) -> run_live_qa.ProbeResult:
            seeded.clear()

            async def fake_live_chat_case(_ctx, **kwargs):
                answer = [str(kwargs["marker"])]
                for text in seeded:
                    # A dropped-thread-replies run surfaces the control
                    # (root/top-level) messages but not the REPLY_* markers.
                    if surface_replies or not text.startswith("REPLY_"):
                        answer.append(text)
                return run_live_qa.ProbeResult(
                    provider="test",
                    mode=f"live:{kwargs['case_name']}",
                    success=True,
                    latency_ms=1,
                    details={
                        "full_reply_text": "\n".join(answer),
                        "submission_identity": {
                            "thread_id": "thread-current",
                            "run_id": "run-current",
                            "turn_id": "turn-current",
                        },
                    },
                )

            with (
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_token",
                    return_value="xoxp-test",
                ),
                patch.object(
                    run_live_qa,
                    "_require_slack_bot_token",
                    return_value="xoxb-test",
                ),
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_bot_dm_channel",
                    return_value="D0FIXTURE1",
                ),
                patch.object(
                    run_live_qa,
                    "_seed_slack_fixture_message",
                    side_effect=fake_seed,
                ),
                patch.object(
                    run_live_qa,
                    "_live_chat_case",
                    side_effect=fake_live_chat_case,
                ),
                patch.object(
                    run_live_qa,
                    "_current_turn_capability_evidence",
                    return_value=evidence,
                ),
            ):
                return asyncio.run(
                    run_live_qa.case_qa_10c_slack_thread_replies(
                        self._dummy_ctx()
                    )
                )

        search_only_evidence = {
            "statuses": {
                "slack.get_thread_replies": [],
                "slack.search_messages": ["completed"],
            }
        }
        no_retrieval_evidence = {
            "statuses": {
                "slack.get_thread_replies": [],
                "slack.search_messages": [],
            }
        }

        # Replies surfaced via indexed search alone (the dedicated
        # thread-replies tool never ran): OUTCOME met -> PASS. This is exactly
        # the trace that used to flake red on tool identity. (History is NOT
        # an accept-any member — the shipped manifest documents it can never
        # return replies.)
        surfaced = drive(
            surface_replies=True, evidence=search_only_evidence
        )
        self.assertTrue(surfaced.success, surfaced.details.get("error"))
        self.assertEqual(surfaced.details.get("missing_thread_reply_markers"), [])

        # Replies NOT surfaced (agent genuinely can't see thread replies) ->
        # FAIL, even though a retrieval capability did run.
        dropped = drive(
            surface_replies=False, evidence=search_only_evidence
        )
        self.assertFalse(dropped.success)
        self.assertIn(
            "thread replies are invisible to the agent",
            str(dropped.details.get("error")),
        )

        # accept-any is an OR-group, not a bypass: with NO retrieval capability
        # producing terminal evidence the capability arm still fails closed.
        no_capability = drive(
            surface_replies=True, evidence=no_retrieval_evidence
        )
        self.assertFalse(no_capability.success)
        self.assertEqual(
            no_capability.details.get("failure_category"),
            "missing_expected_capability",
        )

    def test_slack_correctness_chat_reply_classifies_terminal_provider_errors(self):
        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=False,
                latency_ms=1,
                details={
                    "error": "The configured model provider is unavailable.",
                    "failure_category": "model_unavailable",
                    "failure_status": "failed",
                },
            )

        with patch.object(
            run_live_qa,
            "_live_chat_case",
            side_effect=fake_live_chat_case,
        ):
            chat, reply_text = asyncio.run(
                run_live_qa._slack_correctness_chat_reply(
                    self._dummy_ctx(),
                    case_name="qa_10_provider_failure_test",
                    started=run_live_qa.time.monotonic(),
                    prompt="Read Slack.",
                    answer_marker="ANSWER_MARKER",
                    extra_details={},
                )
            )

        self.assertFalse(chat.success)
        self.assertEqual(reply_text, "")
        self.assertEqual(chat.details["failure_class"], "infrastructure")
        self.assertEqual(chat.details["failure_category"], "model_unavailable")

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
            reply = asyncio.run(
                run_live_qa._wait_for_assistant_reply(
                    self._fake_assistant_reply_page(response_text),
                    marker=None,
                    required_text=["routine"],
                    timeout=0.001,
                    semantic_goal="Create an hourly Hacker News keyword Slack routine.",
                )
            )

        text = reply.text_excerpt
        self.assertIn("Trigger ID", text)
        self.assertTrue(reply.semantic_judge_used)
        self.assertEqual(reply.semantic_judge_reason, "semantic_judge_completed")
        self.assertEqual(
            reply.final_reply_reason,
            "semantic_judge_final_reply_observed",
        )
        self.assertEqual(reply.semantic_judge["confidence"], 0.91)
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

    def test_browser_diagnostic_redaction_handles_empty_auth_token_and_llm_keys(self):
        value = (
            "prefix abc "
            "sk-test1234567890abcdefghijklmnop "
            "sk-ant-test1234567890"
        )

        with patch.object(run_live_qa, "AUTH_TOKEN", ""):
            redacted = run_live_qa._redact_browser_diagnostic_value(value)

        self.assertIn("prefix abc", redacted)
        self.assertIn("<REDACTED_OPENAI_KEY>", redacted)
        self.assertIn("<REDACTED_ANTHROPIC_KEY>", redacted)
        self.assertNotIn("sk-test1234567890abcdefghijklmnop", redacted)
        self.assertNotIn("sk-ant-test1234567890", redacted)

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
                {"IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-test"},
            )

        self.assertTrue(result["ok"])
        self.assertEqual(result["dm_user_id"], "UQAUSER")
        self.assertEqual(result["dm_user_source"], "env")
        self.assertEqual(captured["data"]["users"], "UQAUSER")

    def test_slack_dm_route_discovery_reads_path_materialized_user(self):
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
        with tempfile.TemporaryDirectory() as tmpdir:
            user_id_path = Path(tmpdir) / "route-user-id"
            user_id_path.write_text("UQAUSER\n", encoding="utf-8")
            with (
                patch.dict(
                    os.environ,
                    {
                        "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID_PATH": str(
                            user_id_path
                        )
                    },
                    clear=True,
                ),
                patch.dict(sys.modules, {"httpx": fake_httpx}),
            ):
                result = run_live_qa._discover_slack_dm_route_channel(
                    {"IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-test"},
                )

        self.assertTrue(result["ok"])
        self.assertEqual(result["dm_user_id"], "UQAUSER")
        self.assertEqual(captured["data"]["users"], "UQAUSER")

    def test_slack_dm_route_discovery_rejects_missing_real_route_user(self):
        fake_httpx = types.SimpleNamespace(
            post=lambda *_args, **_kwargs: self.fail("Slack API should not be called")
        )
        with (
            patch.dict(
                os.environ,
                {"REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID": "U0REBORNQA"},
                clear=True,
            ),
            patch.dict(sys.modules, {"httpx": fake_httpx}),
        ):
            result = run_live_qa._discover_slack_dm_route_channel(
                {"IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-test"},
            )

        self.assertFalse(result["ok"])
        self.assertEqual(result["error"], "missing_slack_route_user_id")
        self.assertIn("REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID", result["required_env"])

    def test_live_github_latest_release_uses_configured_token(self):
        captured: dict[str, object] = {}

        class FakeResponse:
            def raise_for_status(self):
                return None

            def json(self):
                return {"tag_name": "ironclaw-v0.test", "name": "Test release"}

        class FakeAsyncClient:
            def __init__(self, **kwargs):
                captured.update(kwargs)

            async def __aenter__(self):
                return self

            async def __aexit__(self, _exc_type, _exc, _tb):
                return None

            async def get(self, url):
                captured["url"] = url
                return FakeResponse()

        fake_httpx = types.SimpleNamespace(AsyncClient=FakeAsyncClient)
        with (
            patch.dict(os.environ, {"AUTH_LIVE_GITHUB_TOKEN": "ghs_live"}, clear=True),
            patch.dict(sys.modules, {"httpx": fake_httpx}),
        ):
            release = asyncio.run(
                run_live_qa._live_github_latest_release("nearai", "ironclaw")
            )

        self.assertEqual(release["tag_name"], "ironclaw-v0.test")
        self.assertEqual(
            captured["url"],
            "https://api.github.com/repos/nearai/ironclaw/releases/latest",
        )
        self.assertEqual(
            captured["headers"]["Authorization"],
            "Bearer ghs_live",
        )

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

    def test_slack_delivery_channel_ignores_failed_route_discovery_channel_id(self):
        with tempfile.TemporaryDirectory() as tmpdir, patch.object(
            run_live_qa,
            "_slack_preflight",
            return_value={
                "route_discovery": {
                    "checked": True,
                    "ok": False,
                    "channel_id": "D0STALE",
                }
            },
        ):
            ctx = run_live_qa.LiveQaContext(
                base_url="http://127.0.0.1:3000",
                output_dir=Path(tmpdir),
                reborn_home=Path(tmpdir) / "reborn-home",
                env={},
            )

            self.assertIsNone(run_live_qa._slack_delivery_channel_id(ctx))

    def test_qa_7a_accepts_existing_dm_delivery_target_without_chat_connect(self):
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
            patch.object(
                run_live_qa,
                "_with_page",
                side_effect=AssertionError("QA 7A should not open WebUI chat"),
            ),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_7a_slack_product_channel_connect(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertEqual(result.details["slack_delivery_target_kind"], "dm")
        self.assertEqual(result.details["delivery_target_present"], True)
        self.assertIn("preflight", result.details)

    def test_start_reborn_server_sets_slack_personal_oauth_redirect(self):
        captured: dict[str, object] = {}

        class FakeProcess:
            pass

        def fake_popen(*_args, **kwargs):
            captured["env"] = kwargs["env"]
            captured["cwd"] = kwargs["cwd"]
            kwargs["stdout"].close()
            kwargs["stderr"].close()
            return FakeProcess()

        async def fake_wait_for_ready(url: str, *, timeout: float) -> None:
            captured["health_url"] = url
            captured["timeout"] = timeout

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            with (
                patch.object(run_live_qa, "reserve_loopback_port", return_value=38555),
                patch.object(run_live_qa.subprocess, "Popen", side_effect=fake_popen),
                patch.object(run_live_qa, "wait_for_ready", side_effect=fake_wait_for_ready),
            ):
                proc, base_url = asyncio.run(
                    run_live_qa.start_reborn_server(
                        root / "ironclaw",
                        root / "reborn-home",
                        root / "out",
                        {
                            "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID": "slack-client",
                            "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET": "slack-secret",
                        },
                    )
                )

        self.assertIsInstance(proc, FakeProcess)
        self.assertEqual(base_url, "http://127.0.0.1:38555")
        self.assertEqual(captured["health_url"], "http://127.0.0.1:38555/api/health")
        env = captured["env"]
        self.assertIsInstance(env, dict)
        self.assertEqual(
            env["IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"],
            (
                "http://127.0.0.1:38555"
                "/api/reborn/product-auth/oauth/slack_personal/callback"
            ),
        )

    def test_start_reborn_server_sets_slack_redirect_from_persisted_oauth_client_id(self):
        captured: dict[str, object] = {}

        class FakeProcess:
            pass

        def fake_popen(*_args, **kwargs):
            captured["env"] = kwargs["env"]
            kwargs["stdout"].close()
            kwargs["stderr"].close()
            return FakeProcess()

        async def fake_wait_for_ready(url: str, *, timeout: float) -> None:
            captured["health_url"] = url
            captured["timeout"] = timeout

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            reborn_home = root / "reborn-home"
            reborn_home.mkdir()
            (reborn_home / "config.toml").write_text(
                "[slack]\nenabled = true\n",
                encoding="utf-8",
            )
            db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
            run_live_qa._root_filesystem_create_table(db_path)
            run_live_qa._put_root_filesystem_json(
                db_path,
                "/tenants/reborn-cli/shared/slack-setup/installation.json",
                {
                    "installation_id": "local-dev-installation",
                    "team_id": "T123",
                    "api_app_id": "A123",
                    "oauth_client_id": "persisted-client-id",
                },
            )
            env = {
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID": "",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID_PATH": "",
                "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI": "",
                "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI_PATH": "",
            }
            with (
                patch.dict(os.environ, env, clear=False),
                patch.object(run_live_qa, "reserve_loopback_port", return_value=38555),
                patch.object(run_live_qa.subprocess, "Popen", side_effect=fake_popen),
                patch.object(run_live_qa, "wait_for_ready", side_effect=fake_wait_for_ready),
            ):
                proc, base_url = asyncio.run(
                    run_live_qa.start_reborn_server(
                        root / "ironclaw",
                        reborn_home,
                        root / "out",
                        {
                            "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET": "slack-secret",
                        },
                    )
                )

        self.assertIsInstance(proc, FakeProcess)
        self.assertEqual(base_url, "http://127.0.0.1:38555")
        env = captured["env"]
        self.assertIsInstance(env, dict)
        self.assertEqual(
            env["IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"],
            (
                "http://127.0.0.1:38555"
                "/api/reborn/product-auth/oauth/slack_personal/callback"
            ),
        )

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
        self.assertIn(
            run_live_qa._qa_sheet_prompt("qa_7c_slack_bug_logger_routine"),
            captured_routine["prompt"],
        )
        self.assertIn("bug logging Google Sheet", captured_routine["prompt"])
        self.assertIn(
            "https://docs.google.com/spreadsheets/d/sheet-123/edit",
            captured_routine["prompt"],
        )
        self.assertIn("Sheet1", captured_routine["prompt"])
        self.assertIn(
            "Summary, Reporter, Slack Timestamp, Status, QA Marker",
            captured_routine["prompt"],
        )
        package_ids = [
            extension["package_id"] for extension in captured_routine["extensions"]
        ]
        self.assertEqual(package_ids, ["google-drive", "google-sheets"])
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
                    "inbound_user_id": "U0REBORNQA",
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

    def test_hn_routine_accepts_schedule_monitor_confirmation(self):
        captured: dict[str, object] = {}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured.update(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={
                    "text_excerpt": (
                        "Done! Here's what was created: HN Monitor. "
                        "Schedule: Every hour at :00."
                    )
                },
            )

        with (
            patch.object(run_live_qa, "_live_chat_case", side_effect=fake_live_chat_case),
            patch.object(run_live_qa, "_trigger_record_count", side_effect=[0, 1]),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_8c_hn_keyword_slack_routine(self._dummy_ctx())
            )

        self.assertTrue(result.success)
        self.assertEqual(
            captured["required_text"],
            ["routine|trigger|automation|cron|schedule|created|monitor"],
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

        async def fake_wait_for_google_sheet_marker(**_kwargs):
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
            patch.object(
                run_live_qa,
                "_wait_for_google_sheet_marker",
                side_effect=fake_wait_for_google_sheet_marker,
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

        async def fake_wait_for_google_sheet_marker(**kwargs):
            self.assertEqual(kwargs["spreadsheet_id"], spreadsheet_id)
            self.assertEqual(kwargs["marker"], captured_lookup["name"])
            self.assertEqual(kwargs["timeout"], 90.0)
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
                "_wait_for_google_sheet_marker",
                side_effect=fake_wait_for_google_sheet_marker,
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

    def test_gmail_to_sheet_delivery_records_empty_verifier_exception_type(self):
        async def fake_live_chat_with_extensions_case(_ctx, **kwargs):
            marker = kwargs["marker"]
            return run_live_qa.ProbeResult(
                provider="test",
                mode="live:qa_6e_gmail_to_sheet_delivery",
                success=True,
                latency_ms=1,
                details={
                    "marker": marker,
                    "text_excerpt": (
                        "Google Sheet "
                        "https://docs.google.com/spreadsheets/d/"
                        "1AbCdEfGhIjKlMnOpQrStUvWxYz_1234567890/edit"
                    ),
                },
            )

        async def fake_wait_for_google_sheet_marker(**_kwargs):
            raise TimeoutError()

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
                "_wait_for_google_sheet_marker",
                side_effect=fake_wait_for_google_sheet_marker,
            ),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_6e_gmail_to_sheet_delivery(self._dummy_ctx())
            )

        self.assertFalse(result.success)
        self.assertEqual(result.details["error"], "TimeoutError")

    def test_wait_for_google_sheet_marker_retries_transient_read_errors(self):
        class FakeHttpxReadError(Exception):
            pass

        attempts = 0

        async def fake_google_sheet_contains_marker(**_kwargs):
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                raise FakeHttpxReadError("read timed out")
            return {"found": True, "row_count": 2}

        with (
            patch.object(google_api_helpers, "_HTTPX_HTTP_ERROR", FakeHttpxReadError),
            patch.object(
                google_api_helpers,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
            patch.object(google_api_helpers.asyncio, "sleep", return_value=None),
        ):
            result = asyncio.run(
                google_api_helpers._wait_for_google_sheet_marker(
                    access_token="access-token",
                    spreadsheet_id="spreadsheet-id",
                    marker="marker",
                    timeout=1.0,
                )
            )

        self.assertEqual(result, {"found": True, "row_count": 2})
        self.assertEqual(attempts, 2)

    def test_wait_for_google_sheet_marker_reports_empty_exception_type(self):
        class FakeHttpxReadError(Exception):
            pass

        async def fake_google_sheet_contains_marker(**_kwargs):
            raise FakeHttpxReadError("")

        with (
            patch.object(google_api_helpers, "_HTTPX_HTTP_ERROR", FakeHttpxReadError),
            patch.object(
                google_api_helpers,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
            patch.object(google_api_helpers.asyncio, "sleep", return_value=None),
        ):
            with self.assertRaisesRegex(AssertionError, "last_error=FakeHttpxReadError"):
                asyncio.run(
                    google_api_helpers._wait_for_google_sheet_marker(
                        access_token="access-token",
                        spreadsheet_id="spreadsheet-id",
                        marker="marker",
                        timeout=0.001,
                    )
                )

    def test_wait_for_google_sheet_marker_does_not_retry_explicit_api_failures(self):
        attempts = 0

        async def fake_google_sheet_contains_marker(**_kwargs):
            nonlocal attempts
            attempts += 1
            raise AssertionError("Google Sheets read returned HTTP 403: forbidden")

        with (
            patch.object(
                google_api_helpers,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
            patch.object(google_api_helpers.asyncio, "sleep", return_value=None),
        ):
            with self.assertRaisesRegex(AssertionError, "HTTP 403"):
                asyncio.run(
                    google_api_helpers._wait_for_google_sheet_marker(
                        access_token="access-token",
                        spreadsheet_id="spreadsheet-id",
                        marker="marker",
                        timeout=1.0,
                    )
                )

        self.assertEqual(attempts, 1)

    def test_wait_for_google_sheet_marker_propagates_unexpected_errors(self):
        attempts = 0

        async def fake_google_sheet_contains_marker(**_kwargs):
            nonlocal attempts
            attempts += 1
            raise ValueError("bad sheet payload")

        with (
            patch.object(
                google_api_helpers,
                "_google_sheet_contains_marker",
                side_effect=fake_google_sheet_contains_marker,
            ),
            patch.object(google_api_helpers.asyncio, "sleep", return_value=None),
        ):
            with self.assertRaisesRegex(ValueError, "bad sheet payload"):
                asyncio.run(
                    google_api_helpers._wait_for_google_sheet_marker(
                        access_token="access-token",
                        spreadsheet_id="spreadsheet-id",
                        marker="marker",
                        timeout=1.0,
                    )
                )

        self.assertEqual(attempts, 1)

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
                return_value={"inbound_user_id": "U0REBORNQA"},
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

    def test_signed_slack_event_cases_resolve_inbound_user_without_legacy_config(self):
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
                    user_id = run_live_qa._slack_inbound_user_id_for_cases(
                        [case_name],
                    )

                self.assertEqual(user_id, "U0REBORNQA")
                self.assertNotIn("slack_user_id", config_path.read_text(encoding="utf-8"))

    def test_signed_slack_event_cases_prefer_real_route_user_actor_without_legacy_config(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = Path(tmpdir) / "config.toml"
            config_path.write_text("[slack]\n", encoding="utf-8")
            with patch.dict(
                os.environ,
                {
                    "REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID": "UQAUSER",
                    "REBORN_WEBUI_V2_LIVE_QA_SLACK_INBOUND_USER_ID": "U0REBORNQA",
                },
                clear=True,
            ):
                user_id = run_live_qa._slack_inbound_user_id_for_cases(
                    ["qa_5d_slack_strategy_doc_answer"],
                )

            self.assertEqual(user_id, "UQAUSER")
            self.assertNotIn("slack_user_id", config_path.read_text(encoding="utf-8"))

    def test_slack_personal_dm_seed_satisfies_delivery_target(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir)
            (home / "local-dev").mkdir(parents=True)
            run_live_qa._root_filesystem_create_table(
                home / "local-dev" / "reborn-local-dev.db"
            )
            run_live_qa._put_root_filesystem_json(
                home / "local-dev" / "reborn-local-dev.db",
                "/tenants/reborn-cli/shared/slack-setup/installation.json",
                {
                    "installation_id": "install-alpha",
                    "team_id": "T123",
                    "api_app_id": "A123",
                    "user_id": "user:web",
                    "bot_token_handle": "slack_bot_token_handle",
                    "signing_secret_handle": "slack_signing_secret_handle",
                    "revision": 1,
                    "updated_at": "2026-01-01T00:00:00Z",
                },
            )
            result = run_live_qa._seed_slack_personal_dm_target(
                home,
                "[slack]\nenabled = true\n",
                auth_user_id="user:web",
                slack_user_id="UQAUSER",
                dm_channel_id="D0QA",
            )

            self.assertTrue(
                run_live_qa._has_slack_delivery_target("", home, "user:web"),
                result,
            )
            self.assertEqual(
                run_live_qa._persisted_slack_personal_dm_channel_id(home, "user:web"),
                "D0QA",
            )
            db_path = home / "local-dev" / "reborn-local-dev.db"
            with closing(sqlite3.connect(db_path)) as db:
                dm_row = db.execute(
                    """
                    SELECT path, contents FROM root_filesystem_entries
                    WHERE path LIKE '%/slack-personal-binding/dm-targets/%'
                    """
                ).fetchone()
                row = db.execute(
                    """
                    SELECT path, contents FROM root_filesystem_entries
                    WHERE path LIKE '%/outbound/communication-preferences/%'
                    """
                ).fetchone()
            self.assertIsNotNone(dm_row)
            self.assertEqual(
                dm_row[0],
                "/tenants/reborn-cli/shared/slack-personal-binding/dm-targets/"
                "aW5zdGFsbC1hbHBoYQ/VDEyMw/dXNlcjp3ZWI.json",
            )
            self.assertIsNotNone(row)
            preference = json.loads(row[1])
            self.assertTrue(row[0].endswith(".json"))
            self.assertEqual(
                preference["scope"],
                {
                    "kind": "personal",
                    "tenant_id": "reborn-cli",
                    "user_id": "user:web",
                },
            )
            self.assertIn("adapter:8:slack_v2;", preference["final_reply_target"])
            self.assertIn("installation:13:install-alpha;", preference["final_reply_target"])
            self.assertIn("space:4:T123;", preference["final_reply_target"])
            self.assertIn("conversation:4:D0QA;", preference["final_reply_target"])
            self.assertIn("actor_kind:10:slack_user;", preference["final_reply_target"])
            self.assertIn("actor:7:UQAUSER;", preference["final_reply_target"])
            self.assertEqual(preference["updated_by"], "user:web")

    def test_slack_personal_dm_lookup_requires_exact_user_id(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir)
            (home / "local-dev").mkdir(parents=True)
            run_live_qa._root_filesystem_create_table(
                home / "local-dev" / "reborn-local-dev.db"
            )
            run_live_qa._put_root_filesystem_json(
                home / "local-dev" / "reborn-local-dev.db",
                "/tenants/reborn-cli/shared/slack-setup/installation.json",
                {
                    "installation_id": "install-alpha",
                    "team_id": "T123",
                    "api_app_id": "A123",
                    "user_id": "user:web",
                    "bot_token_handle": "slack_bot_token_handle",
                    "signing_secret_handle": "slack_signing_secret_handle",
                    "revision": 1,
                    "updated_at": "2026-01-01T00:00:00Z",
                },
            )
            result = run_live_qa._seed_slack_personal_dm_target(
                home,
                "[slack]\nenabled = true\n",
                auth_user_id="user:web-extra",
                slack_user_id="UQAUSER",
                dm_channel_id="D0QA",
            )

            self.assertTrue(result["seeded"], result)
            self.assertFalse(
                run_live_qa._has_slack_delivery_target("", home, "user:web")
            )
            self.assertIsNone(
                run_live_qa._persisted_slack_personal_dm_channel_id(home, "user:web")
            )
            self.assertEqual(
                run_live_qa._persisted_slack_personal_dm_channel_id(
                    home,
                    "user:web-extra",
                ),
                "D0QA",
            )

    def test_slack_channel_route_no_longer_satisfies_delivery_target(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir)
            config = (
                '[slack]\n\n[[slack.channel_routes]]\nchannel_id = "D0QA"\n'
                'subject_user_id = "user:web"\n'
            )

            self.assertFalse(
                run_live_qa._has_slack_delivery_target(config, home, "user:web")
            )

    def test_remove_dm_slack_channel_routes_preserves_shared_routes(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = Path(tmpdir) / "config.toml"
            config_path.write_text(
                "\n".join(
                    [
                        "[slack]",
                        'enabled = true',
                        "",
                        "[[slack.channel_routes]]",
                        'channel_id = "D0STALE"',
                        'subject_user_id = "user:web"',
                        "",
                        "[[slack.channel_routes]]",
                        'channel_id = "C0SHARED"',
                        'subject_user_id = "user:web"',
                        "",
                        "[telegram]",
                        'enabled = false',
                        "",
                    ]
                ),
                encoding="utf-8",
            )

            cleanup = run_live_qa._remove_dm_slack_channel_routes(config_path)
            config = config_path.read_text(encoding="utf-8")

            self.assertEqual(cleanup, {"changed": True, "removed": 1})
            self.assertNotIn("D0STALE", config)
            self.assertIn("C0SHARED", config)
            self.assertIn("[telegram]", config)

    def test_prepare_reborn_home_removes_copied_stale_dm_channel_route(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            source_home = tmp / "source"
            source_home.mkdir()
            (source_home / "config.toml").write_text(
                "\n".join(
                    [
                        "[llm]",
                        'provider_id = "nearai"',
                        'model = "deepseek-ai/DeepSeek-V4-Flash"',
                        'api_key_env = "NEARAI_API_KEY"',
                        "",
                        "[slack]",
                        'enabled = true',
                        'installation_id = "legacy-install"',
                        'team_id = "TLEGACY"',
                        'api_app_id = "ALEGACY"',
                        'signing_secret_env = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"',
                        'bot_token_env = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"',
                        "",
                        "[[slack.channel_routes]]",
                        'channel_id = "D0STALE"',
                        'subject_user_id = "user:web"',
                        "",
                    ]
                ),
                encoding="utf-8",
            )
            args = argparse.Namespace(
                output_dir=tmp / "out",
                require_slack_live=False,
                reborn_home=source_home,
            )
            env = {
                "NEARAI_API_KEY": "fake-live-llm-key",
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
            }

            with patch.dict(os.environ, env, clear=False):
                prepared = run_live_qa.prepare_reborn_home(
                    args,
                    ["qa_3a_slack_connect"],
                )
            config = (prepared.path / "config.toml").read_text(encoding="utf-8")

            self.assertNotIn("D0STALE", config)
            self.assertNotIn("C0SHARED", config)
            for rejected in (
                "installation_id",
                "team_id",
                "api_app_id",
                "signing_secret_env",
                "bot_token_env",
                "[[slack.channel_routes]]",
            ):
                self.assertNotIn(rejected, config)
            self.assertEqual(
                prepared.preflight["slack"]["legacy_setup_cleanup"],
                {
                    "changed": True,
                    "removed_channel_routes": 1,
                    "removed_fields": [
                        "api_app_id",
                        "bot_token_env",
                        "installation_id",
                        "signing_secret_env",
                        "team_id",
                    ],
                },
            )

    def test_with_page_writes_browser_diagnostics_on_failure(self):
        class FakeTracing:
            async def start(self, **_kwargs):
                return None

            async def stop(self, *, path=None):
                if path:
                    Path(path).write_text("trace", encoding="utf-8")

        class FakeRequest:
            def url(self):
                return "https://example.test/callback?token=secret-token"

            def method(self):
                return "GET"

            def failure(self):
                return {"errorText": "net::ERR_FAILED"}

        class FakeMessage:
            def type(self):
                return "error"

            def text(self):
                return "bearer super-secret-token"

            def location(self):
                return {"url": "https://example.test/app.js"}

        class FakePage:
            def on(self, event, callback):
                if event == "console":
                    callback(FakeMessage())
                if event == "pageerror":
                    callback(RuntimeError("page exploded"))

            async def screenshot(self, *, path, full_page):
                Path(path).write_text(f"screenshot full_page={full_page}", encoding="utf-8")

        class FakeContext:
            def __init__(self):
                self.tracing = FakeTracing()
                self.page = FakePage()

            def on(self, event, callback):
                if event == "requestfailed":
                    callback(FakeRequest())
                if event == "page":
                    callback(self.page)

            async def new_page(self):
                return self.page

            async def close(self):
                return None

        class FakeBrowser:
            def __init__(self):
                self.context = FakeContext()

            async def new_context(self):
                return self.context

            async def close(self):
                return None

        class FakeChromium:
            async def launch(self, **_kwargs):
                return FakeBrowser()

        class FakePlaywright:
            def __init__(self):
                self.chromium = FakeChromium()

        class FakeAsyncPlaywright:
            async def __aenter__(self):
                return FakePlaywright()

            async def __aexit__(self, *_args):
                return None

        async def failing_action(_page):
            raise AssertionError("boom")

        fake_async_api = types.SimpleNamespace(
            async_playwright=lambda: FakeAsyncPlaywright()
        )
        with tempfile.TemporaryDirectory() as tmpdir, patch.dict(
            sys.modules,
            {
                "playwright": types.SimpleNamespace(),
                "playwright.async_api": fake_async_api,
            },
        ):
            output_dir = Path(tmpdir)
            with self.assertRaises(AssertionError):
                asyncio.run(
                    run_live_qa._with_page(output_dir, "case_a", failing_action)
                )

            diagnostics_dir = output_dir / "browser-diagnostics" / "case_a"
            events = (diagnostics_dir / "browser-events.jsonl").read_text(encoding="utf-8")
            self.assertIn("requestfailed", events)
            self.assertIn("console", events)
            self.assertIn("<REDACTED>", events)
            self.assertTrue((diagnostics_dir / "browser-summary.json").exists())
            self.assertTrue((diagnostics_dir / "playwright-trace.zip").exists())
            self.assertTrue((output_dir / "case_a.failure.png").exists())

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

    def test_non_signed_slack_cases_do_not_resolve_inbound_user(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config_path = Path(tmpdir) / "config.toml"
            config_path.write_text("[slack]\n", encoding="utf-8")

            user_id = run_live_qa._slack_inbound_user_id_for_cases(
                ["qa_3a_slack_connect"],
            )

            self.assertIsNone(user_id)
            self.assertNotIn("slack_user_id", config_path.read_text(encoding="utf-8"))

    def test_slack_event_run_id_reads_idempotency_record(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_path = home / "local-dev" / "reborn-local-dev.db"
            db_path.parent.mkdir(parents=True)
            with closing(sqlite3.connect(db_path)) as db:
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
                db.commit()

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
            with closing(sqlite3.connect(db_path)) as db:
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
            with closing(sqlite3.connect(db_path)) as db:
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

            with closing(sqlite3.connect(db_path)) as db:
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

    def test_generated_slack_seed_creates_live_user_product_auth_account(self):
        if importlib.util.find_spec("cryptography") is None:
            self.skipTest("cryptography is installed in the e2e venv, not system Python")

        class FakeResponse:
            status_code = 200

            @staticmethod
            def json():
                return {
                    "ok": True,
                    "team_id": "T123",
                    "user_id": "U123",
                    "url": "https://example.slack.com/",
                }

        class FakeHttpx:
            calls: list[dict[str, object]] = []

            @classmethod
            def post(cls, url, *, headers, timeout):
                cls.calls.append({"url": url, "headers": headers, "timeout": timeout})
                return FakeResponse()

        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            env = {
                "AUTH_LIVE_SLACK_ACCESS_TOKEN": "xoxp-live-user-token",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID": "local-dev-installation",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID": "T123",
                "REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID": "A123",
            }
            with (
                patch.dict(os.environ, env, clear=False),
                patch.dict(sys.modules, {"httpx": FakeHttpx}),
            ):
                seed = run_live_qa._seed_generated_slack_product_auth_if_configured(
                    home,
                    "qa-user",
                )
                preflight = run_live_qa._slack_personal_auth_preflight(
                    home,
                    "qa-user",
                    {},
                    requires_slack_personal_auth=True,
                )

            self.assertTrue(seed["seeded"])
            self.assertEqual(seed["token_env_source"], "AUTH_LIVE_SLACK_ACCESS_TOKEN")
            self.assertEqual(seed["slack_user_id"], "U123")
            self.assertTrue(preflight["ready"])
            self.assertEqual(preflight["configured_account_count"], 1)
            self.assertEqual(preflight["auth_test"]["user_id"], "U123")
            self.assertEqual(preflight["accounts"][0]["thread_id"], seed["thread_id"])
            self.assertEqual(
                preflight["accounts"][0]["invocation_id"],
                seed["invocation_id"],
            )
            self.assertEqual(len(FakeHttpx.calls), 2)

            db_path = home / "local-dev" / "reborn-local-dev.db"
            master_key_path = home / "local-dev" / ".reborn-local-dev-secrets-master-key"
            self.assertEqual(master_key_path.stat().st_mode & 0o777, 0o600)
            master_key = master_key_path.read_text(encoding="utf-8")
            with closing(sqlite3.connect(db_path)) as db:
                account_row = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE '%product-auth/callback/accounts/%.json'"
                ).fetchone()
            self.assertIsNotNone(account_row)
            account = json.loads(account_row[0])
            self.assertEqual(account["provider"], "slack_personal")
            self.assertEqual(account["status"], "configured")
            self.assertEqual(account["provider_identity"]["subject"], "U123")
            self.assertEqual(account["provider_identity"]["team_id"], "T123")
            self.assertEqual(account["provider_identity"]["app_id"], "A123")

            with closing(sqlite3.connect(db_path)) as db:
                secret_row = db.execute(
                    "SELECT contents FROM root_filesystem_entries "
                    "WHERE path LIKE ?",
                    (f"%/{account['access_secret']}.json",),
                ).fetchone()
            self.assertIsNotNone(secret_row)
            stored = json.loads(secret_row[0])
            self.assertEqual(
                run_live_qa._decrypt_filesystem_secret(master_key, stored),
                "xoxp-live-user-token",
            )

    def test_slack_connect_cases_require_personal_product_auth(self):
        for case_name in ("qa_3a_slack_connect", "qa_5a_slack_connect", "qa_8a_slack_connect"):
            self.assertTrue(run_live_qa.CASES[case_name].requires_slack_personal_auth)

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
            self.assertIsNone(slack["setup"]["installation_id"])
            self.assertIsNone(slack["setup"]["team_id"])
            self.assertIsNone(slack["setup"]["api_app_id"])

    def test_slack_setup_payload_uses_persisted_oauth_client_id(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            reborn_home = Path(tmpdir) / "reborn-home"
            db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
            run_live_qa._root_filesystem_create_table(db_path)
            run_live_qa._put_root_filesystem_json(
                db_path,
                "/tenants/reborn-cli/shared/slack-setup/installation.json",
                {
                    "installation_id": "local-dev-installation",
                    "team_id": "T123",
                    "api_app_id": "A123",
                    "oauth_client_id": "persisted-client-id",
                },
            )
            payload, preflight = run_live_qa._slack_setup_payload(
                reborn_home,
                "[slack]\nenabled = true\n",
                {
                    "IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-bot",
                    "IRONCLAW_REBORN_SLACK_SIGNING_SECRET": "signing-secret",
                    "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET": "oauth-secret",
                },
            )

        self.assertIsNotNone(payload)
        self.assertEqual(payload.get("oauth_client_id"), "persisted-client-id")
        self.assertTrue(preflight["personal_oauth_ready"])

    def test_slack_setup_payload_prefers_env_oauth_client_id(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            reborn_home = Path(tmpdir) / "reborn-home"
            db_path = reborn_home / "local-dev" / "reborn-local-dev.db"
            run_live_qa._root_filesystem_create_table(db_path)
            run_live_qa._put_root_filesystem_json(
                db_path,
                "/tenants/reborn-cli/shared/slack-setup/installation.json",
                {
                    "installation_id": "local-dev-installation",
                    "team_id": "T123",
                    "api_app_id": "A123",
                    "oauth_client_id": "stale-client-id",
                },
            )
            payload, preflight = run_live_qa._slack_setup_payload(
                reborn_home,
                "[slack]\nenabled = true\n",
                {
                    "IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-bot",
                    "IRONCLAW_REBORN_SLACK_SIGNING_SECRET": "signing-secret",
                    "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID": "fresh-client-id",
                    "REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET": "oauth-secret",
                },
            )

        self.assertIsNotNone(payload)
        self.assertEqual(payload.get("oauth_client_id"), "fresh-client-id")
        self.assertEqual(preflight["oauth_client_id"], "fresh-client-id")
        self.assertTrue(preflight["personal_oauth_ready"])

    def test_slack_personal_auth_preflight_rejects_account_without_user_scope(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_path = home / "local-dev" / "reborn-local-dev.db"
            run_live_qa._root_filesystem_create_table(db_path)
            run_live_qa._put_root_filesystem_json(
                db_path,
                (
                    "/tenants/reborn-cli/users/qa-user/secrets/agents/"
                    "reborn-cli-agent/product-auth/callback/accounts/account.json"
                ),
                {
                    "id": "account",
                    "provider": "slack_personal",
                    "status": "configured",
                    "scope": {
                        "resource": {
                            "tenant_id": "reborn-cli",
                            "agent_id": "reborn-cli-agent",
                        },
                    },
                    "access_secret": "slack-access-handle",
                },
            )
            env = {
                "AUTH_LIVE_SLACK_ACCESS_TOKEN": "",
                "AUTH_LIVE_SLACK_ACCESS_TOKEN_PATH": "",
            }
            with patch.dict(os.environ, env, clear=False):
                preflight = run_live_qa._slack_personal_auth_preflight(
                    home,
                    "qa-user",
                    {},
                    requires_slack_personal_auth=True,
                )

        self.assertFalse(preflight["ready"])
        self.assertEqual(preflight["configured_account_count"], 0)
        self.assertEqual(preflight["accounts"], [])
        self.assertEqual(
            preflight["reason"],
            "no configured Slack personal product-auth account",
        )

    def test_slack_setup_api_failure_omits_response_body(self):
        class FakeResponse:
            status_code = 400
            text = (
                "echoed xoxb-bot-token signing-secret-value "
                "oauth-client-secret-value"
            )

            def json(self):
                return {"ok": False}

        class FakeAsyncClient:
            def __init__(self, *args, **kwargs):
                pass

            async def __aenter__(self):
                return self

            async def __aexit__(self, _exc_type, _exc, _tb):
                return None

            async def put(self, *_args, **_kwargs):
                return FakeResponse()

        fake_httpx = types.SimpleNamespace(AsyncClient=FakeAsyncClient)
        with tempfile.TemporaryDirectory() as tmpdir:
            reborn_home = Path(tmpdir) / "reborn-home"
            reborn_home.mkdir()
            (reborn_home / "config.toml").write_text(
                "[slack]\nenabled = true\n",
                encoding="utf-8",
            )
            prepared = run_live_qa.PreparedRebornHome(
                path=reborn_home,
                env={"IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-bot-token"},
            )
            with (
                patch.dict(sys.modules, {"httpx": fake_httpx}),
                patch.object(
                    run_live_qa,
                    "_slack_setup_payload",
                    return_value=(
                        {
                            "bot_token": "xoxb-bot-token",
                            "signing_secret": "signing-secret-value",
                            "oauth_client_secret": "oauth-client-secret-value",
                        },
                        {},
                    ),
                ),
                self.assertRaises(run_live_qa.LiveQaError) as raised,
            ):
                asyncio.run(
                    run_live_qa._apply_slack_setup_api_after_start(
                        base_url="http://127.0.0.1:38555",
                        prepared_home=prepared,
                    )
                )

        error = str(raised.exception)
        self.assertIn("Slack setup API returned HTTP 400", error)
        self.assertIn("response body omitted", error)
        self.assertNotIn("xoxb-bot-token", error)
        self.assertNotIn("signing-secret-value", error)
        self.assertNotIn("oauth-client-secret-value", error)
        self.assertNotIn("echoed", error)

    def test_slack_setup_api_requires_configured_status(self):
        class FakeResponse:
            status_code = 200

            def json(self):
                return {
                    "configured": False,
                    "installation_id": "install-123",
                    "team_id": "T123",
                    "api_app_id": "A123",
                    "bot_token_configured": True,
                    "signing_secret_configured": False,
                    "oauth_client_id_configured": True,
                    "oauth_client_secret_configured": False,
                }

        class FakeAsyncClient:
            def __init__(self, *args, **kwargs):
                pass

            async def __aenter__(self):
                return self

            async def __aexit__(self, _exc_type, _exc, _tb):
                return None

            async def put(self, *_args, **_kwargs):
                return FakeResponse()

        fake_httpx = types.SimpleNamespace(AsyncClient=FakeAsyncClient)
        with tempfile.TemporaryDirectory() as tmpdir:
            reborn_home = Path(tmpdir) / "reborn-home"
            reborn_home.mkdir()
            (reborn_home / "config.toml").write_text(
                "[slack]\nenabled = true\n",
                encoding="utf-8",
            )
            prepared = run_live_qa.PreparedRebornHome(path=reborn_home)
            with (
                patch.dict(sys.modules, {"httpx": fake_httpx}),
                patch.object(
                    run_live_qa,
                    "_slack_setup_payload",
                    return_value=(
                        {
                            "installation_id": "install-123",
                            "team_id": "T123",
                            "api_app_id": "A123",
                            "bot_token": "xoxb-bot-token",
                            "signing_secret": "signing-secret-value",
                            "oauth_client_id": "oauth-client-id",
                            "oauth_client_secret": "oauth-client-secret-value",
                        },
                        {},
                    ),
                ),
                self.assertRaises(run_live_qa.LiveQaError) as raised,
            ):
                asyncio.run(
                    run_live_qa._apply_slack_setup_api_after_start(
                        base_url="http://127.0.0.1:38555",
                        prepared_home=prepared,
                    )
                )

        error = str(raised.exception)
        self.assertIn("incomplete setup status", error)
        self.assertIn("configured", error)
        self.assertIn("signing_secret_configured", error)
        self.assertIn("oauth_client_secret_configured", error)
        self.assertNotIn("xoxb-bot-token", error)
        self.assertNotIn("signing-secret-value", error)
        self.assertNotIn("oauth-client-secret-value", error)

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
            for rejected in (
                "installation_id",
                "team_id",
                "api_app_id",
                "signing_secret_env",
                "bot_token_env",
                "slack_user_id",
                "[[slack.channel_routes]]",
            ):
                self.assertNotIn(rejected, config)
            self.assertFalse((source_home / "config.toml").exists())

    def test_generated_slack_home_uses_webui_setup_only(self):
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
            self.assertIn("[slack]", config)
            self.assertIn("enabled = true", config)
            for rejected in (
                "installation_id",
                "team_id",
                "api_app_id",
                "signing_secret_env",
                "bot_token_env",
                "slack_user_id",
                "[[slack.channel_routes]]",
            ):
                self.assertNotIn(rejected, config)

    def test_marker_match_stats_counts_and_classifies_marker_authors(self):
        marker = "QA_MARKER_123"
        messages = [
            {"text": f"bot copy {marker}", "ts": "100.1", "bot_id": "B01", "user": "UBOT"},
            {"text": "unrelated", "ts": "100.2", "user": "U0HUMAN1"},
            {"text": f"human copy {marker}", "ts": "100.3", "user": "U0HUMAN1"},
            {"text": f"second bot copy {marker}", "ts": "160.5", "bot_id": "B01"},
        ]
        stats = run_live_qa._marker_match_stats(messages, marker=marker)
        self.assertTrue(stats["found"])
        self.assertEqual(stats["marker_matches"], 3)
        self.assertEqual(stats["bot_authored_marker_matches"], 2)
        self.assertEqual(stats["human_authored_marker_matches"], 1)
        authors = stats["marker_match_authors"]
        self.assertEqual([entry["bot"] for entry in authors], [True, False, True])
        self.assertEqual([entry["human"] for entry in authors], [False, True, False])

    def test_marker_match_stats_requires_required_text_on_first_match(self):
        marker = "QA_MARKER_456"
        messages = [{"text": f"{marker} without the word", "ts": "1.0", "bot_id": "B01"}]
        stats = run_live_qa._marker_match_stats(
            messages, marker=marker, required_text=["status"]
        )
        self.assertFalse(stats["found"])
        self.assertTrue(stats["marker_found"])
        self.assertEqual(stats["missing_required_text"], ["status"])

    def test_marker_match_stats_reports_not_found_without_matches(self):
        stats = run_live_qa._marker_match_stats(
            [{"text": "nothing here", "ts": "1.0", "user": "U0X"}], marker="QA_MARKER_789"
        )
        self.assertFalse(stats["found"])
        self.assertEqual(stats["message_count"], 1)

    def test_exactly_once_delivery_cases_use_one_shot_schedules(self):
        import inspect

        for case_fn in (
            run_live_qa.case_qa_9b_routine_dm_delivery_exactly_once,
            run_live_qa.case_qa_9d_routine_per_trigger_delivery_target,
        ):
            source = inspect.getsource(case_fn)
            self.assertIn(
                "one-time routine",
                source,
                "exactly-once probes must use one-shot schedules; a recurring "
                "schedule re-posts the marker on the next fire and false-fails",
            )
            self.assertIn("exactly_once_grace_seconds", source)
            self.assertIn(
                "require_slack_tools_on_surface=True",
                source,
                "exactly-once probes must ASSERT the Slack tools surface "
                "precondition; without it the duplicate arm passes vacuously",
            )
            self.assertIn(
                "expect_one_shot_schedule=True",
                source,
                "exactly-once probes must verify the PERSISTED schedule is a "
                "once-schedule; prompt wording alone cannot prevent a cron "
                "trigger from fabricating a duplicate-delivery red",
            )
            self.assertIn(
                'follow_up_timezone_instruction="Use the UTC timezone',
                source,
                "UTC-pinned one-shot probes must answer timezone clarifying "
                "questions with UTC; the default London instruction can shift "
                "the fire ~1h outside the delivery wait window",
            )

    def test_slack_delivery_case_verifies_schedule_outcome_from_trigger_record(self):
        import inspect

        source = inspect.getsource(run_live_qa._slack_delivery_routine_case)
        self.assertIn("schedule_kind", source)
        self.assertIn("_trigger_record_snapshot", source)
        self.assertIn(
            "record_count",
            source,
            "the probe must reject duplicate same-name trigger records: two "
            "live one-shots each deliver once and read as a duplicate",
        )
        self.assertIn(
            "_outbound_final_reply_targets",
            source,
            "per-trigger routing must assert the user-wide default target was "
            "NOT rewritten — otherwise a default-mutating server passes",
        )

    def test_outbound_final_reply_targets_reads_preference_rows(self):
        import sqlite3 as sqlite3_module
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3_module.connect(db_path) as db:
                db.execute(
                    "CREATE TABLE root_filesystem_entries "
                    "(path TEXT, contents TEXT, is_dir INTEGER)"
                )
                db.execute(
                    "INSERT INTO root_filesystem_entries VALUES "
                    "('/tenants/t/users/u/outbound/communication-preferences/a.json', "
                    "'{\"final_reply_target\": \"reply:adapter:slack\"}', 0)"
                )
            targets = run_live_qa._outbound_final_reply_targets(home)
            self.assertEqual(
                targets,
                {
                    "/tenants/t/users/u/outbound/communication-preferences/a.json": (
                        "reply:adapter:slack"
                    )
                },
            )
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(
                run_live_qa._outbound_final_reply_targets(Path(tmp)), {}
            )

    def test_exactly_once_case_specs_gate_on_personal_auth_and_run_by_default(self):
        for case_name in (
            "qa_9b_routine_dm_delivery_exactly_once",
            "qa_9d_routine_per_trigger_delivery_target",
        ):
            spec = run_live_qa.CASES[case_name]
            self.assertTrue(
                spec.requires_slack_personal_auth,
                f"{case_name} sweeps with the personal token; without the "
                "workspace-mismatch gate the sweep can search the wrong "
                "workspace and pass structurally blind",
            )
        for case_name in (
            "qa_9b_routine_dm_delivery_exactly_once",
            "qa_9c_slack_digest_names_not_ids",
            "qa_9d_routine_per_trigger_delivery_target",
        ):
            self.assertTrue(
                run_live_qa.CASES[case_name].default_enabled,
                f"{case_name} is promoted (delivery-routing fixes merged and "
                "live-verified green) and must run in bare local default runs",
            )

    def test_qa_9_contracts_block_but_digest_prose_is_behavioral(self):
        for case_name in (
            "qa_9a_slack_connect",
            "qa_9b_routine_dm_delivery_exactly_once",
            "qa_9d_routine_per_trigger_delivery_target",
        ):
            spec = run_live_qa.CASES[case_name]
            self.assertEqual(spec.tier, "contract", case_name)
            self.assertTrue(spec.blocking, case_name)

        digest_spec = run_live_qa.CASES["qa_9c_slack_digest_names_not_ids"]
        self.assertEqual(digest_spec.tier, "behavioral")
        self.assertFalse(
            digest_spec.blocking,
            "9C evaluates stochastic final prose without a deterministic "
            "capability-call contract, so it must stay visible without making "
            "the live-QA job randomly red",
        )

    def test_exc_text_preserves_type_for_empty_str_exceptions(self):
        self.assertEqual(run_live_qa._exc_text(ValueError("boom")), "boom")
        # asyncio timeouts stringify to "" — the previous formatting produced
        # probe failures with an empty error field.
        self.assertEqual(run_live_qa._exc_text(TimeoutError()), "TimeoutError()")

    def test_parse_epoch_seconds_handles_trigger_store_timestamps(self):
        parsed = run_live_qa._parse_epoch_seconds("2026-07-09T21:16:11.000000000Z")
        self.assertIsNotNone(parsed)
        self.assertAlmostEqual(parsed, 1783631771.0, delta=1.0)
        self.assertIsNone(run_live_qa._parse_epoch_seconds(None))
        self.assertIsNone(run_live_qa._parse_epoch_seconds("not-a-time"))

    def test_trigger_record_snapshot_reads_schedule_and_delivery_target(self):
        import sqlite3 as sqlite3_module
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3_module.connect(db_path) as db:
                db.execute(
                    "CREATE TABLE trigger_records ("
                    "name TEXT, schedule_kind TEXT, next_run_at TEXT, "
                    "delivery_target TEXT)"
                )
                db.execute(
                    "INSERT INTO trigger_records VALUES "
                    "('probe', 'once', '2026-07-09T21:16:11.000000000Z', "
                    "'slack:personal-dm:T1:me')"
                )
            snapshot = run_live_qa._trigger_record_snapshot(home, "probe")
            self.assertTrue(snapshot["checked"])
            self.assertEqual(snapshot["record_count"], 1)
            self.assertEqual(snapshot["schedule_kind"], "once")
            self.assertEqual(snapshot["delivery_target"], "slack:personal-dm:T1:me")
            self.assertFalse(snapshot["delivery_target_column_missing"])

    def test_trigger_record_snapshot_flags_missing_delivery_target_column(self):
        import sqlite3 as sqlite3_module
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            db_dir = home / "local-dev"
            db_dir.mkdir(parents=True)
            db_path = db_dir / "reborn-local-dev.db"
            with sqlite3_module.connect(db_path) as db:
                db.execute(
                    "CREATE TABLE trigger_records ("
                    "name TEXT, schedule_kind TEXT, next_run_at TEXT)"
                )
                db.execute(
                    "INSERT INTO trigger_records VALUES "
                    "('probe', 'cron', '2026-07-09T21:16:11.000000000Z')"
                )
            snapshot = run_live_qa._trigger_record_snapshot(home, "probe")
            # Pre-fix server: schedule facts still readable, delivery target
            # column reported missing (NOT an opaque sqlite error).
            self.assertTrue(snapshot["checked"])
            self.assertTrue(snapshot["delivery_target_column_missing"])
            self.assertEqual(snapshot["schedule_kind"], "cron")
            self.assertIsNone(snapshot["delivery_target"])

    def test_trigger_record_snapshot_reports_unreadable_db(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            snapshot = run_live_qa._trigger_record_snapshot(Path(tmp), "probe")
            self.assertFalse(snapshot["checked"])
            self.assertIn("error", snapshot)

    def test_raw_slack_user_id_pattern_requires_id_shape(self):
        self.assertEqual(
            run_live_qa._raw_slack_user_ids_in_text(
                "digest mentions U0FIXTURE01 and W0FIXTURE02"
            ),
            ["U0FIXTURE01", "W0FIXTURE02"],
        )
        self.assertEqual(
            run_live_qa._raw_slack_user_ids_in_text(
                "UPDATE the digest for benjamin.kurrek and u0bdc16tml3"
            ),
            [],
        )
        prose = "UNDERSTAND this WONDERFUL WATERFALL"
        self.assertEqual(
            run_live_qa.RAW_SLACK_USER_ID_PATTERN.sub("U_REDACTED", prose),
            prose,
        )

    def test_digest_scan_uses_full_reply_and_redacts_persisted_excerpt(self):
        import inspect

        source = inspect.getsource(
            run_live_qa.case_qa_9c_slack_digest_names_not_ids
        )
        self.assertIn(
            "full_reply_text",
            source,
            "the raw-id scan must read the full in-memory reply; excerpt "
            "truncation would blind it to ids early in a long digest",
        )
        self.assertIn(
            "_redact_slack_entity_ids_in_artifact_details",
            source,
            "all persisted assistant-response fields must be redacted before "
            "the digest returns",
        )
        sanitizer_source = inspect.getsource(
            run_live_qa._redact_slack_entity_ids_in_artifact_details
        )
        self.assertIn(
            "_redact_slack_user_ids_in_text",
            sanitizer_source,
            "the artifact sanitizer must redact raw user ids",
        )
        self.assertIn(
            "RAW_SLACK_CONVERSATION_ID_PATTERN.sub",
            sanitizer_source,
            "the persisted excerpt must be redacted of raw C…/D…/G… "
            "conversation ids too — scanning without redacting would still "
            "persist them",
        )
        self.assertIn(
            "_raw_slack_conversation_ids_in_text",
            source,
            "the digest must also fail on leaked raw C…/D…/G… conversation "
            "ids (the qa_10i channel-ID companion arm)",
        )
        wait_source = inspect.getsource(run_live_qa._wait_for_assistant_reply)
        self.assertIn("full_text=", wait_source)

    def test_qa_9c_digest_persists_leak_counts_not_raw_ids(self):
        # A leak verdict must not itself re-leak: the failed ProbeResult's
        # persisted details (error message included) carry only counts and
        # redacted excerpts, never the raw U…/C…/D…/G… identifiers.
        scenarios = {
            "user_id": (
                "your DMs: U0BDC16TML3 said hi REBORN_QA_9C_DIGEST",
                "U0BDC16TML3",
                "leaked_raw_user_id_count",
            ),
            "encoded_user_id": (
                "your DMs: <@UABCDEFGH> said hi REBORN_QA_9C_DIGEST",
                "UABCDEFGH",
                "leaked_raw_user_id_count",
            ),
            "conversation_id": (
                "your DMs: D0BDC16TML3 has one message REBORN_QA_9C_DIGEST",
                "D0BDC16TML3",
                "leaked_raw_conversation_id_count",
            ),
        }
        for label, (reply, raw_id, count_key) in scenarios.items():
            with self.subTest(scenario=label):
                async def fake_live_chat_case(_ctx, **kwargs):
                    return run_live_qa.ProbeResult(
                        provider="test",
                        mode=f"live:{kwargs['case_name']}",
                        success=True,
                        latency_ms=1,
                        details={
                            "full_reply_text": reply,
                            "text_excerpt": reply,
                        },
                    )

                with (
                    patch.object(
                        run_live_qa,
                        "_live_chat_case",
                        side_effect=fake_live_chat_case,
                    ),
                    patch.object(
                        run_live_qa,
                        "_slack_personal_dm_counterpart_names",
                        return_value={"names": []},
                    ),
                ):
                    result = asyncio.run(
                        run_live_qa.case_qa_9c_slack_digest_names_not_ids(
                            self._dummy_ctx()
                        )
                    )
                self.assertFalse(result.success)
                self.assertEqual(result.details[count_key], 1)
                persisted = json.dumps(result.details)
                self.assertNotIn(raw_id, persisted)
                self.assertNotIn("full_reply_text", result.details)

    def test_qa_9c_digest_redacts_failed_chat_artifacts_before_return(self):
        raw_user_id = "UABCDEFGH"
        raw_conversation_id = "D0BDC16TML3"
        reply = (
            f"latest DM from <@{raw_user_id}> in {raw_conversation_id}"
        )

        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=False,
                latency_ms=1,
                details={
                    "full_reply_text": reply,
                    "text_excerpt": reply,
                    "error": f"timed out with last_assistant={reply!r}",
                },
            )

        with patch.object(
            run_live_qa,
            "_live_chat_case",
            side_effect=fake_live_chat_case,
        ):
            result = asyncio.run(
                run_live_qa.case_qa_9c_slack_digest_names_not_ids(
                    self._dummy_ctx()
                )
            )

        self.assertFalse(result.success)
        persisted = json.dumps(result.details)
        self.assertNotIn(raw_user_id, persisted)
        self.assertNotIn(raw_conversation_id, persisted)
        self.assertNotIn("full_reply_text", result.details)

    def test_dm_counterpart_scan_paginates_to_exhaustion(self):
        import inspect

        source = inspect.getsource(
            run_live_qa._slack_personal_dm_counterpart_names
        )
        self.assertIn(
            "next_cursor",
            source,
            "conversations.list ground truth must follow next_cursor; one "
            "page can flip the verdict",
        )
        self.assertIn("checked", source)

    def test_raw_slack_conversation_id_pattern_requires_id_shape(self):
        self.assertEqual(
            run_live_qa._raw_slack_conversation_ids_in_text(
                "history for C0BDC16TML3 and D0BDC16TML3 plus G0ABCD1234"
            ),
            ["C0BDC16TML3", "D0BDC16TML3", "G0ABCD1234"],
        )
        # Second char must be a digit and the id must be long enough: all-caps
        # prose (DELIVERED / CHANNELS / GENERAL), lowercase echoes, and short
        # fragments never false-positive.
        self.assertEqual(
            run_live_qa._raw_slack_conversation_ids_in_text(
                "DELIVERED CHANNELS GENERAL c0bdc16tml3 C0SHORT CANARY"
            ),
            [],
        )

    def test_encoded_slack_mention_pattern_matches_notifying_forms_only(self):
        self.assertIsNotNone(
            run_live_qa.ENCODED_SLACK_MENTION_PATTERN.search(
                "MENTION_123 ping <@U0BDC16TML3>"
            )
        )
        # The labelled legacy form also notifies — it must count as encoded.
        self.assertIsNotNone(
            run_live_qa.ENCODED_SLACK_MENTION_PATTERN.search(
                "MENTION_123 ping <@U0BDC16TML3|ben>"
            )
        )
        self.assertIsNone(
            run_live_qa.ENCODED_SLACK_MENTION_PATTERN.search(
                "MENTION_123 ping @Benjamin Kurrek (literal, notifies nobody)"
            )
        )

    def test_encoded_mention_targets_user_pins_the_exact_target(self):
        # An encoded mention of the WRONG user (self-mention, unrelated id)
        # must not satisfy the mention-encoding probe — only the counterpart
        # the prompt named counts, in bare or labelled form.
        self.assertTrue(
            run_live_qa._encoded_mention_targets_user(
                "MENTION_123 ping <@U0TARGET01>", "U0TARGET01"
            )
        )
        self.assertTrue(
            run_live_qa._encoded_mention_targets_user(
                "MENTION_123 ping <@U0TARGET01|ben>", "U0TARGET01"
            )
        )
        self.assertFalse(
            run_live_qa._encoded_mention_targets_user(
                "MENTION_123 ping <@U0SOMEONE9>", "U0TARGET01"
            )
        )
        self.assertFalse(
            run_live_qa._encoded_mention_targets_user(
                "MENTION_123 ping @Target Name (literal)", "U0TARGET01"
            )
        )
        self.assertFalse(
            run_live_qa._encoded_mention_targets_user(
                "MENTION_123 ping <@U0TARGET01>", ""
            )
        )

    def test_classify_encoded_mention_accepts_via_app_user_post(self):
        # Granular Slack apps stamp EVERY user-token post with the app's
        # bot_id/bot_profile ("via app") while `user` stays the human author
        # — the connected user's own encoded-mention post must be FOUND, with
        # the stamp surfaced as via_app, never rejected as a bot post.
        verdict = run_live_qa._classify_encoded_mention_messages(
            [
                {
                    "user": "U0AUTHOR01",
                    "bot_id": "B0VIAAPP01",
                    "text": "hey <@U0TARGET01> MENTION_123",
                    "ts": "1.000100",
                }
            ],
            marker="MENTION_123",
            author_user_id="U0AUTHOR01",
        )
        found = verdict["found"]
        self.assertIsNotNone(found)
        self.assertEqual(found["ts"], "1.000100")
        self.assertTrue(found["via_app"])
        self.assertEqual(verdict["author_mismatch"], [])

    def test_classify_encoded_mention_ignores_unencoded_echoes(self):
        # A delivered assistant reply echoes the marker WITHOUT an encoded
        # mention: it must be ignored entirely — neither selected as the
        # post nor reported as a wrong author.
        verdict = run_live_qa._classify_encoded_mention_messages(
            [
                {
                    "user": "U0BOTUSER9",
                    "bot_id": "B0SERVBOT9",
                    "text": "I posted the mention with marker MENTION_123",
                    "ts": "2.000100",
                }
            ],
            marker="MENTION_123",
            author_user_id="U0AUTHOR01",
        )
        self.assertIsNone(verdict["found"])
        self.assertEqual(verdict["author_mismatch"], [])
        self.assertIsNone(verdict["unencoded_author_marker_ts"])

    def test_classify_encoded_mention_rejects_bot_identity_posts(self):
        # A true bot-identity post (user = the BOT user id) carrying an
        # encoded marker mention is a wrong-identity author — the check the
        # probe exists for, judged on Slack's unforgeable `user` field.
        verdict = run_live_qa._classify_encoded_mention_messages(
            [
                {
                    "user": "U0BOTUSER9",
                    "bot_id": "B0SERVBOT9",
                    "text": "hey <@U0TARGET01> MENTION_123",
                    "ts": "3.000100",
                }
            ],
            marker="MENTION_123",
            author_user_id="U0AUTHOR01",
        )
        self.assertIsNone(verdict["found"])
        self.assertEqual(
            verdict["author_mismatch"],
            [{"ts": "3.000100", "bot": True, "user_matches_author": False}],
        )

    def test_classify_encoded_mention_flags_literal_at_posts(self):
        # The connected user posting the marker WITHOUT an encoded mention is
        # the literal-@ failure this probe pins — it must be flagged, not
        # silently treated as "never posted".
        verdict = run_live_qa._classify_encoded_mention_messages(
            [
                {
                    "user": "U0AUTHOR01",
                    "bot_id": "B0VIAAPP01",
                    "text": "hey @Target Name MENTION_123",
                    "ts": "4.000100",
                }
            ],
            marker="MENTION_123",
            author_user_id="U0AUTHOR01",
        )
        self.assertIsNone(verdict["found"])
        self.assertEqual(verdict["unencoded_author_marker_ts"], "4.000100")
        self.assertIn("@Target Name", verdict["unencoded_author_text"])

    def test_email_addresses_in_text_matches_only_real_addresses(self):
        self.assertEqual(
            run_live_qa._email_addresses_in_text(
                "reach ben.kurrek+qa@near.ai or admin@example.co.uk"
            ),
            ["ben.kurrek+qa@near.ai", "admin@example.co.uk"],
        )
        self.assertEqual(
            run_live_qa._email_addresses_in_text(
                "EMAIL_UNAVAILABLE — no address, just @channel and user@localhost"
            ),
            [],
        )

    def test_name_token_in_text_matches_word_boundary_tokens(self):
        self.assertTrue(
            run_live_qa._name_token_in_text(
                "synced with Benjamin earlier", "Benjamin Kurrek"
            )
        )
        self.assertTrue(
            run_live_qa._name_token_in_text("ping kurrek about it", "Benjamin Kurrek")
        )
        self.assertFalse(
            run_live_qa._name_token_in_text("kurrekian artifacts", "Benjamin Kurrek")
        )
        # Tokens shorter than 3 chars never match — "QA X" must not match the
        # answer-marker soup, and an all-short name yields no tokens at all.
        self.assertFalse(
            run_live_qa._name_token_in_text("REBORN_QA_10I_ANSWER_123", "QA X")
        )
        self.assertEqual(run_live_qa._display_name_tokens("QA X"), [])
        self.assertEqual(
            run_live_qa._display_name_tokens("Benjamin Kurrek"),
            ["benjamin", "kurrek"],
        )

    def test_channel_name_mentioned_uses_hyphen_aware_boundaries(self):
        self.assertTrue(
            run_live_qa._channel_name_mentioned(
                "you are in #general and #eng-canary", "general"
            )
        )
        self.assertTrue(
            run_live_qa._channel_name_mentioned("member of eng-canary today", "eng-canary")
        )
        # "general" inside "general-updates" is a DIFFERENT channel: plain \b
        # matching would false-red the membership probe here.
        self.assertFalse(
            run_live_qa._channel_name_mentioned("member of general-updates only", "general")
        )
        self.assertFalse(
            run_live_qa._channel_name_mentioned("generally speaking", "general")
        )
        self.assertFalse(run_live_qa._channel_name_mentioned("anything", ""))

    def test_slack_non_member_public_channels_diffs_ground_truth(self):
        view = {
            "ok": True,
            "member_channel_ids": ["C0MEMBER01"],
            "listed": [
                {"id": "C0MEMBER01", "name": "general"},
                {"id": "C0OUTSIDE1", "name": "incidents"},
                {"id": "C0OUTSIDE2", "name": "random", "is_member": False},
                # Belt and braces: is_member=True excludes the channel even if
                # the paginated users.conversations scan missed it — the
                # negative arm must never call a real membership a lie.
                {"id": "C0FLAGGED1", "name": "flagged-member", "is_member": True},
                {"id": "", "name": "nameless"},
                {"id": "C0NONAME01"},
                "not-a-dict",
            ],
        }
        self.assertEqual(
            run_live_qa._slack_non_member_public_channels(view),
            [
                {"id": "C0OUTSIDE1", "name": "incidents"},
                {"id": "C0OUTSIDE2", "name": "random"},
            ],
        )
        self.assertEqual(run_live_qa._slack_non_member_public_channels({}), [])

    def test_status_code_tokens_match_hyphenated_codes_only(self):
        self.assertEqual(
            run_live_qa._status_code_tokens(
                "OOO-CANARY-FIXTURE — back July 20"
            ),
            ["OOO-CANARY-FIXTURE"],
        )
        # Prose hyphens, lowercase echoes, and un-hyphenated caps never count
        # as code tokens — only ALL-CAPS/digit segments joined by hyphens.
        self.assertEqual(
            run_live_qa._status_code_tokens(
                "out-of-office until JULY ooo-canary-fixture"
            ),
            [],
        )
        self.assertEqual(run_live_qa._status_code_tokens(""), [])

    def test_qa_10b_reads_manually_set_status_fixture(self):
        # Read-verify mode: the QA account carries a manually-set permanent
        # OOO status. The probe reads ground truth via users.info (it never
        # writes), fails the precondition when no status is set, and the
        # reply must quote the hyphenated code token verbatim — a paraphrase
        # that drops the code is a red even when other words match.
        fixture = "OOO-CANARY-FIXTURE — back July 20"

        def drive(
            status_text: str, reply_form: str
        ) -> tuple[run_live_qa.ProbeResult, int]:
            async def fake_auth_identity(_token):
                return {"ok": True, "user_id": "U0QAUSER01"}

            async def fake_api_get(_token, method, params=None):
                self.assertEqual(method, "users.info")
                self.assertEqual((params or {}).get("user"), "U0QAUSER01")
                return {
                    "ok": True,
                    "user": {"profile": {"status_text": status_text}},
                }

            chat_calls: list[dict[str, object]] = []

            async def fake_chat_reply(_ctx, **kwargs):
                chat_calls.append(kwargs)
                reply = f"{reply_form} {kwargs['answer_marker']}"
                return (
                    run_live_qa.ProbeResult(
                        provider="test",
                        mode=f"live:{kwargs['case_name']}",
                        success=True,
                        latency_ms=1,
                        details={"text_excerpt": reply},
                    ),
                    reply,
                )

            ctx = self._dummy_ctx()
            ctx.env = {"AUTH_LIVE_SLACK_ACCESS_TOKEN": "xoxp-unit-test"}
            with (
                patch.object(
                    run_live_qa,
                    "_slack_auth_identity",
                    side_effect=fake_auth_identity,
                ),
                patch.object(
                    run_live_qa, "_slack_api_get", side_effect=fake_api_get
                ),
                patch.object(
                    run_live_qa,
                    "_slack_correctness_chat_reply",
                    side_effect=fake_chat_reply,
                ),
            ):
                result = asyncio.run(
                    run_live_qa.case_qa_10b_slack_ooo_status(ctx)
                )
            return result, len(chat_calls)

        # No status set: the distinct precondition fires BEFORE any chat
        # turn is burned.
        empty, chat_turns = drive("", "irrelevant")
        self.assertFalse(empty.success)
        self.assertIn(
            "probe precondition failed: no status is set on the QA account "
            "— restore the OOO canary status fixture",
            str(empty.details.get("error")),
        )
        self.assertEqual(chat_turns, 0)

        # Reply quotes the fixture (code token verbatim): green, with the
        # ground truth recorded in details.
        green, _ = drive(fixture, f"Your status says: {fixture}")
        self.assertTrue(green.success)
        self.assertEqual(green.details["status_text"], fixture)
        self.assertEqual(
            green.details["status_code_tokens"], ["OOO-CANARY-FIXTURE"]
        )
        self.assertTrue(green.details["status_token_matched"])

        # Paraphrase that drops the code token: red even though word tokens
        # ("back", "july") match.
        red, _ = drive(fixture, "You are OOO and back July 20")
        self.assertFalse(red.success)
        self.assertEqual(
            red.details["missing_status_code_tokens"], ["OOO-CANARY-FIXTURE"]
        )
        self.assertIn(
            "reply lacked the exact status code token(s)",
            str(red.details.get("error")),
        )

    def test_slack_second_user_token_reads_optional_env_and_asserts_loudly(self):
        self.assertEqual(
            run_live_qa._slack_second_user_token(
                {"AUTH_LIVE_SLACK_SECOND_USER_TOKEN": "xoxp-second-identity"}
            ),
            "xoxp-second-identity",
        )
        empty_env = {
            "AUTH_LIVE_SLACK_SECOND_USER_TOKEN": "",
            "AUTH_LIVE_SLACK_SECOND_USER_TOKEN_PATH": "",
        }
        with patch.dict(os.environ, empty_env, clear=False):
            self.assertIsNone(run_live_qa._slack_second_user_token({}))
            # Arms that strictly need a second HUMAN must fail with this exact
            # precondition message — never silently skip.
            with self.assertRaisesRegex(
                AssertionError,
                r"probe precondition failed: second-identity token not "
                r"provisioned \(AUTH_LIVE_SLACK_SECOND_USER_TOKEN\)",
            ):
                run_live_qa._require_slack_second_user_token(self._dummy_ctx())

    def test_qa_10d_classifies_invalid_fixture_and_ground_truth_mismatch(self):
        valid_view = {
            "ok": True,
            "member_channels": [{"id": "C0MEMBER01", "name": "general"}],
            "member_channel_ids": ["C0MEMBER01"],
            "listed": [
                {"id": "C0MEMBER01", "name": "general", "is_member": True},
                {"id": "C0OUTSIDE1", "name": "random", "is_member": False},
            ],
        }

        async def fake_chat_reply(_ctx, **kwargs):
            reply = "random"
            return (
                run_live_qa.ProbeResult(
                    provider="test",
                    mode=f"live:{kwargs['case_name']}",
                    success=True,
                    latency_ms=1,
                    details={"text_excerpt": reply},
                ),
                reply,
            )

        with (
            patch.object(
                run_live_qa,
                "_require_slack_personal_token",
                return_value="xoxp-unit-test",
            ),
            patch.object(
                run_live_qa,
                "_slack_membership_view",
                return_value={"ok": False, "error": "fixture unavailable"},
            ),
        ):
            invalid_fixture = asyncio.run(
                run_live_qa.case_qa_10d_slack_channel_membership(self._dummy_ctx())
            )

        with (
            patch.object(
                run_live_qa,
                "_require_slack_personal_token",
                return_value="xoxp-unit-test",
            ),
            patch.object(
                run_live_qa,
                "_slack_membership_view",
                return_value=valid_view,
            ),
            patch.object(
                run_live_qa,
                "_slack_correctness_chat_reply",
                side_effect=fake_chat_reply,
            ),
        ):
            mismatch = asyncio.run(
                run_live_qa.case_qa_10d_slack_channel_membership(self._dummy_ctx())
            )

        self.assertFalse(invalid_fixture.success)
        self.assertEqual(invalid_fixture.details["failure_class"], "precondition")
        self.assertFalse(mismatch.success)
        self.assertEqual(mismatch.details["failure_class"], "product")
        self.assertEqual(mismatch.details["failure_category"], "answer_mismatch")

    def test_qa_10g_scoped_contract_requires_new_history_call_and_global_preserves_prompt(
        self,
    ):
        seeded: dict[str, str] = {}
        captured_chat: list[dict[str, object]] = []

        async def fake_seed(_token, channel_id, text, **_kwargs):
            self.assertEqual(channel_id, "D0FIXTURE1")
            seeded["text"] = text
            return {"ok": True}

        async def fake_live_chat_case(_ctx, **kwargs):
            captured_chat.append(kwargs)
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"full_reply_text": seeded["text"]},
            )

        def drive_scoped(statuses):
            seeded.clear()
            captured_chat.clear()
            evidence = {
                "accepted_message_ref": "msg:test",
                "thread_id": "thread-test",
                "turn_id": "turn-test",
                "run_id": "run-test",
                "invocation_ids": {
                    "slack.get_conversation_history": ["invocation-test"]
                    if statuses
                    else []
                },
                "statuses": {"slack.get_conversation_history": statuses},
            }
            with (
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_token",
                    return_value="xoxp-unit-test",
                ),
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_bot_dm_channel",
                    return_value="D0FIXTURE1",
                ),
                patch.object(
                    run_live_qa,
                    "_seed_slack_fixture_message",
                    side_effect=fake_seed,
                ),
                patch.object(
                    run_live_qa,
                    "_live_chat_case",
                    side_effect=fake_live_chat_case,
                ),
                patch.object(
                    run_live_qa,
                    "_current_turn_capability_evidence",
                    return_value=evidence,
                ),
            ):
                result = asyncio.run(
                    run_live_qa.case_qa_10g_slack_last_message_sent(
                        self._dummy_ctx()
                    )
                )
            return result, dict(captured_chat[0])

        missing_history, missing_chat = drive_scoped([])
        completed_history, scoped_chat = drive_scoped(["completed"])

        self.assertFalse(missing_history.success)
        self.assertEqual(missing_history.details["failure_class"], "model_quality")
        self.assertEqual(
            missing_history.details["expected_capabilities"],
            ["slack.get_conversation_history"],
        )
        self.assertTrue(completed_history.success)
        self.assertIn("Slack conversation with ID D0FIXTURE1", scoped_chat["prompt"])
        self.assertEqual(
            scoped_chat["extensions"],
            [run_live_qa.SLACK_EXTENSION_REQUIREMENT],
        )
        self.assertIs(scoped_chat["enforce_marker"], False)
        self.assertIn("D0FIXTURE1", missing_chat["prompt"])

        global_calls: list[dict[str, object]] = []

        async def fake_global_chat(_ctx, **kwargs):
            global_calls.append(kwargs)
            reply = seeded["text"]
            return (
                run_live_qa.ProbeResult(
                    provider="test",
                    mode=f"live:{kwargs['case_name']}",
                    success=True,
                    latency_ms=1,
                    details={"text_excerpt": reply},
                ),
                reply,
            )

        # The GLOBAL arm gates the turn on the seeded marker becoming
        # searchable; drive the real barrier through a mocked sweep that
        # reports the marker indexed so the assertion path (not the
        # inconclusive path) runs.
        async def fake_search_ready(_ctx, *, marker: str) -> dict[str, object]:
            return {"checked": True, "hits": [{"ts": "1.0"}]}

        seeded.clear()
        with (
            patch.object(
                run_live_qa,
                "_require_slack_personal_token",
                return_value="xoxp-unit-test",
            ),
            patch.object(
                run_live_qa,
                "_require_slack_personal_bot_dm_channel",
                return_value="D0FIXTURE1",
            ),
            patch.object(
                run_live_qa,
                "_seed_slack_fixture_message",
                side_effect=fake_seed,
            ),
            patch.object(
                run_live_qa,
                "_slack_search_marker_hits",
                side_effect=fake_search_ready,
            ),
            patch.object(
                run_live_qa,
                "_slack_correctness_chat_reply",
                side_effect=fake_global_chat,
            ),
        ):
            global_result = asyncio.run(
                run_live_qa.case_qa_10g_slack_last_message_sent_global(
                    self._dummy_ctx()
                )
            )

        self.assertTrue(global_result.success)
        self.assertTrue(
            global_result.details["search_index_readiness"]["ready"]
        )
        self.assertEqual(len(global_calls), 1)
        self.assertEqual(
            global_calls[0]["prompt"],
            "What is the exact text of the most recent message I sent in Slack? "
            f"Include the exact marker {global_calls[0]['answer_marker']} in your answer.",
        )
        self.assertNotIn("D0FIXTURE1", global_calls[0]["prompt"])
        self.assertIsNone(global_calls[0].get("expected_capability"))

        # When the seeded marker never becomes searchable within the deadline,
        # the GLOBAL arm must NOT drive the agent turn or red on answer
        # mismatch — it returns a non-blocking INCONCLUSIVE result attributing
        # the miss to external Slack search-index lag.
        lagged_chat_calls: list[dict[str, object]] = []

        async def fake_lagged_chat(_ctx, **kwargs):
            lagged_chat_calls.append(kwargs)
            raise AssertionError("agent turn must not run when index lags")

        async def fake_barrier_not_ready(_ctx, *, marker: str) -> dict[str, object]:
            return {
                "ready": False,
                "checked": True,
                "permanent": False,
                "attempts": 5,
                "waited_ms": 45000,
                "error": None,
            }

        seeded.clear()
        with (
            patch.object(
                run_live_qa,
                "_require_slack_personal_token",
                return_value="xoxp-unit-test",
            ),
            patch.object(
                run_live_qa,
                "_require_slack_personal_bot_dm_channel",
                return_value="D0FIXTURE1",
            ),
            patch.object(
                run_live_qa,
                "_seed_slack_fixture_message",
                side_effect=fake_seed,
            ),
            patch.object(
                run_live_qa,
                "_wait_for_slack_search_marker",
                side_effect=fake_barrier_not_ready,
            ),
            patch.object(
                run_live_qa,
                "_slack_correctness_chat_reply",
                side_effect=fake_lagged_chat,
            ),
        ):
            lagged_result = asyncio.run(
                run_live_qa.case_qa_10g_slack_last_message_sent_global(
                    self._dummy_ctx()
                )
            )

        self.assertFalse(lagged_result.success)
        self.assertEqual(lagged_chat_calls, [])
        self.assertFalse(lagged_result.details["blocking"])
        self.assertTrue(lagged_result.details["inconclusive"])
        self.assertEqual(
            lagged_result.details["failure_class"], "infrastructure"
        )
        self.assertEqual(
            lagged_result.details["failure_category"], "slack_search_index_lag"
        )
        self.assertFalse(
            lagged_result.details["search_index_readiness"]["ready"]
        )

    def test_qa_10i_requires_display_name_and_rejects_raw_ids_once(self):
        async def fake_identity(_token):
            return {"ok": True, "user_id": "W0FIXTURE1"}

        async def fake_display_name(_token, _user_id):
            return {"ok": True, "display_name": "Canary Person"}

        async def fake_seed(*_args, **_kwargs):
            return {"ok": True}

        def drive(reply_text: str) -> tuple[run_live_qa.ProbeResult, int]:
            calls = 0

            async def fake_chat(_ctx, **kwargs):
                nonlocal calls
                calls += 1
                return (
                    run_live_qa.ProbeResult(
                        provider="test",
                        mode=f"live:{kwargs['case_name']}",
                        success=True,
                        latency_ms=1,
                        details={"text_excerpt": reply_text},
                    ),
                    reply_text,
                )

            with (
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_token",
                    return_value="xoxp-unit-test",
                ),
                patch.object(
                    run_live_qa,
                    "_require_slack_bot_token",
                    return_value="xoxb-unit-test",
                ),
                patch.object(
                    run_live_qa,
                    "_require_slack_personal_bot_dm_channel",
                    return_value="D0FIXTURE1",
                ),
                patch.object(
                    run_live_qa,
                    "_slack_auth_identity",
                    side_effect=fake_identity,
                ),
                patch.object(
                    run_live_qa,
                    "_slack_display_name",
                    side_effect=fake_display_name,
                ),
                patch.object(
                    run_live_qa,
                    "_seed_slack_fixture_message",
                    side_effect=fake_seed,
                ),
                patch.object(
                    run_live_qa,
                    "_slack_correctness_chat_reply",
                    side_effect=fake_chat,
                ),
            ):
                result = asyncio.run(
                    run_live_qa.case_qa_10i_slack_raw_entity_hygiene(
                        self._dummy_ctx()
                    )
                )
            return result, calls

        natural, natural_calls = drive("Canary Person should sync the fixture.")
        raw_id, raw_id_calls = drive(
            "Canary Person (W0FIXTURE1) should sync the fixture."
        )
        encoded_id, encoded_id_calls = drive(
            "Canary Person (<@WABCDEFGH>) should sync the fixture."
        )
        redaction_marker, redaction_marker_calls = drive(
            "Canary Person ([Slack identifier redacted]) should sync the fixture."
        )
        missing_name, missing_name_calls = drive("Someone should sync the fixture.")

        self.assertTrue(natural.success)
        self.assertFalse(raw_id.success)
        self.assertEqual(raw_id.details["failure_class"], "product")
        self.assertFalse(encoded_id.success)
        self.assertNotIn("WABCDEFGH", json.dumps(encoded_id.details))
        self.assertTrue(redaction_marker.success)
        self.assertFalse(missing_name.success)
        self.assertEqual(missing_name.details["failure_class"], "product")
        self.assertEqual(
            (
                natural_calls,
                raw_id_calls,
                encoded_id_calls,
                redaction_marker_calls,
                missing_name_calls,
            ),
            (1, 1, 1, 1, 1),
            "10I is a one-shot behavioral observation and must never retry",
        )

    def test_qa_10i_redacts_failed_chat_artifacts_before_return(self):
        raw_user_id = "WABCDEFGH"
        reply = f"Canary Person (<@{raw_user_id}>) should sync the fixture."

        async def fake_identity(_token):
            return {"ok": True, "user_id": "W0FIXTURE1"}

        async def fake_display_name(_token, _user_id):
            return {"ok": True, "display_name": "Canary Person"}

        async def fake_seed(*_args, **_kwargs):
            return {"ok": True}

        async def fake_live_chat_case(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=False,
                latency_ms=1,
                details={
                    "full_reply_text": reply,
                    "text_excerpt": reply,
                    "error": f"terminal failure after reply {reply!r}",
                },
            )

        with (
            patch.object(
                run_live_qa,
                "_require_slack_personal_token",
                return_value="xoxp-unit-test",
            ),
            patch.object(
                run_live_qa,
                "_require_slack_bot_token",
                return_value="xoxb-unit-test",
            ),
            patch.object(
                run_live_qa,
                "_require_slack_personal_bot_dm_channel",
                return_value="D0FIXTURE1",
            ),
            patch.object(
                run_live_qa,
                "_slack_auth_identity",
                side_effect=fake_identity,
            ),
            patch.object(
                run_live_qa,
                "_slack_display_name",
                side_effect=fake_display_name,
            ),
            patch.object(
                run_live_qa,
                "_seed_slack_fixture_message",
                side_effect=fake_seed,
            ),
            patch.object(
                run_live_qa,
                "_live_chat_case",
                side_effect=fake_live_chat_case,
            ),
        ):
            result = asyncio.run(
                run_live_qa.case_qa_10i_slack_raw_entity_hygiene(
                    self._dummy_ctx()
                )
            )

        self.assertFalse(result.success)
        persisted = json.dumps(result.details)
        self.assertNotIn(raw_user_id, persisted)
        self.assertNotIn("full_reply_text", result.details)

    def test_qa_10_case_specs_gate_and_run_by_default(self):
        qa_10_cases = [
            "qa_10a_slack_self_attribution",
            "qa_10b_slack_ooo_status",
            "qa_10c_slack_thread_replies",
            "qa_10d_slack_channel_membership",
            "qa_10e_slack_error_honesty",
            "qa_10f_slack_mention_encoding",
            "qa_10g_slack_last_message_sent",
            "qa_10g_slack_last_message_sent_global",
            "qa_10h_slack_email_hallucination_guard",
            "qa_10i_slack_raw_entity_hygiene",
        ]
        seeded_dm_cases = {
            "qa_10a_slack_self_attribution",
            "qa_10c_slack_thread_replies",
            "qa_10f_slack_mention_encoding",
            "qa_10g_slack_last_message_sent",
            "qa_10g_slack_last_message_sent_global",
            "qa_10h_slack_email_hallucination_guard",
            "qa_10i_slack_raw_entity_hygiene",
        }
        behavioral_cases = {
            "qa_10g_slack_last_message_sent_global",
            "qa_10i_slack_raw_entity_hygiene",
        }
        for case_name in qa_10_cases:
            spec = run_live_qa.CASES[case_name]
            self.assertTrue(spec.requires_slack, case_name)
            self.assertTrue(
                spec.requires_slack_personal_auth,
                f"{case_name} seeds/reads with the personal token; without "
                "the workspace-mismatch gate its arms are structurally blind",
            )
            self.assertTrue(
                spec.default_enabled,
                f"{case_name} is promoted (tool-surface fixes merged and "
                "live-verified 9/9 green) and must run in bare local default "
                "runs",
            )
            self.assertEqual(
                spec.requires_slack_target,
                case_name in seeded_dm_cases,
                f"{case_name}: requires_slack_target must mark exactly the "
                "cases that seed into / read from the personal↔bot DM",
            )
            self.assertEqual(
                spec.tier,
                "behavioral" if case_name in behavioral_cases else "contract",
                case_name,
            )
            self.assertEqual(
                spec.blocking,
                case_name not in behavioral_cases,
                case_name,
            )
            self.assertIn(case_name, run_live_qa.QA_SHEET_CASES)
        self.assertEqual(
            [run_live_qa.QA_SHEET_CASES[name]["rows"] for name in qa_10_cases],
            [
                ["10A"],
                ["10B"],
                ["10C"],
                ["10D"],
                ["10E"],
                ["10F"],
                ["10G"],
                ["10G"],
                ["10H"],
                ["10I"],
            ],
        )

    def test_qa_10_cases_pin_their_audited_failure_asserts(self):
        import inspect

        pins = {
            run_live_qa.case_qa_10a_slack_self_attribution: (
                "SELFMSG_A_",
                "SELFMSG_B_",
                "OTHERMSG_C_",
                "OTHERMSG_D_",
                "no-self-identity gap",
            ),
            run_live_qa.case_qa_10b_slack_ooo_status: (
                # Read-verify mode: ground truth comes from users.info on the
                # manually-set fixture — the probe must never write a status.
                "_slack_user_status_text",
                "users.info",
                "no status is set on the QA account ",
                "restore the OOO canary status fixture",
                "_status_code_tokens",
                "_name_token_in_text",
            ),
            run_live_qa.case_qa_10c_slack_thread_replies: (
                "THREADROOT_",
                "REPLY_ONE_",
                "REPLY_TWO_",
                "REPLY_THREE_",
                "TOPLEVEL_",
                "thread_ts=root_ts",
            ),
            run_live_qa.case_qa_10d_slack_channel_membership: (
                "_slack_membership_view",
                "_slack_non_member_public_channels",
                "no non-member public channel",
                "_channel_name_mentioned",
            ),
            run_live_qa.case_qa_10e_slack_error_honesty: (
                "C0CANARYNOPE",
                "channel_not_found",
            ),
            run_live_qa.case_qa_10f_slack_mention_encoding: (
                "MENTION_",
                # The encoded-mention gate moved into
                # _classify_encoded_mention_messages (selection is
                # encoded-mention-first); the case pins its literal-@
                # failure surface instead. The pattern itself is pinned on
                # the classifier below.
                "unencoded_author_marker_ts",
                "a literal @-name notifies nobody",
                "_encoded_mention_targets_user",
                "mention_targets_counterpart",
                "_wait_for_authored_slack_message",
                "author_mismatch",
                "expected_capability_sequence",
                "slack.send_message",
            ),
            run_live_qa.case_qa_10g_slack_last_message_sent: (
                "LASTSENT_",
                "slack.get_conversation_history",
            ),
            run_live_qa.case_qa_10g_slack_last_message_sent_global: (
                "LASTSENT_",
            ),
            run_live_qa.case_qa_10h_slack_email_hallucination_guard: (
                "EMAIL_UNAVAILABLE",
                "_email_addresses_in_text",
            ),
            run_live_qa.case_qa_10i_slack_raw_entity_hygiene: (
                "ENTITYMSG_",
                "_raw_slack_user_ids_in_text",
                "<@U",
                "_name_token_in_text",
            ),
        }
        for case_fn, needles in pins.items():
            source = inspect.getsource(case_fn)
            for needle in needles:
                self.assertIn(
                    needle,
                    source,
                    f"{case_fn.__name__} must keep its {needle!r} assert — it "
                    "pins the audited failure",
                )
        # qa_10f's encoded-mention gate lives in the shared classifier now:
        # the pattern and the authoritative `user`-field authorship check
        # must stay pinned there.
        classifier_source = inspect.getsource(
            run_live_qa._classify_encoded_mention_messages
        )
        for needle in (
            "ENCODED_SLACK_MENTION_PATTERN",
            "author_user_id",
            "via_app",
        ):
            self.assertIn(
                needle,
                classifier_source,
                f"_classify_encoded_mention_messages must keep its {needle!r} "
                "gate — it pins the audited failure",
            )

    def test_qa_10_cases_fail_closed_without_slack_tokens(self):
        # Drive real case functions (not just their source): with no personal
        # token provisioned they must return a FAILED ProbeResult carrying the
        # distinct precondition message — never a vacuous pass, never an
        # exception escaping into the shard runner, and no network/playwright
        # side effects before the precondition fires. Every QA 10 entrypoint
        # that acquires the personal token is driven (qa_10e acquires none —
        # it is error-honesty over a guaranteed-nonexistent channel).
        blank_env = {}
        for name in (
            "AUTH_LIVE_SLACK_ACCESS_TOKEN",
            "AUTH_LIVE_SLACK_USER_TOKEN",
            "REBORN_WEBUI_V2_LIVE_QA_SLACK_USER_TOKEN",
        ):
            blank_env[name] = ""
            blank_env[f"{name}_PATH"] = ""
        for case_fn in (
            run_live_qa.case_qa_10a_slack_self_attribution,
            run_live_qa.case_qa_10b_slack_ooo_status,
            run_live_qa.case_qa_10c_slack_thread_replies,
            run_live_qa.case_qa_10d_slack_channel_membership,
            run_live_qa.case_qa_10f_slack_mention_encoding,
            run_live_qa.case_qa_10g_slack_last_message_sent,
            run_live_qa.case_qa_10g_slack_last_message_sent_global,
            run_live_qa.case_qa_10h_slack_email_hallucination_guard,
            run_live_qa.case_qa_10i_slack_raw_entity_hygiene,
        ):
            with self.subTest(case=case_fn.__name__):
                with patch.dict(os.environ, blank_env, clear=False):
                    result = asyncio.run(case_fn(self._dummy_ctx()))
                self.assertFalse(result.success, case_fn.__name__)
                self.assertIn(
                    "probe precondition failed: Slack personal token not "
                    "provisioned (AUTH_LIVE_SLACK_ACCESS_TOKEN)",
                    str(result.details.get("error")),
                    case_fn.__name__,
                )
                self.assertEqual(
                    result.details.get("failure_class"),
                    "precondition",
                    case_fn.__name__,
                )

    def test_qa_10_seeding_and_api_helpers_are_guarded(self):
        import inspect

        for helper in (
            run_live_qa._slack_api_get,
            run_live_qa._slack_api_post,
        ):
            source = inspect.getsource(helper)
            self.assertIn(
                "_exc_text",
                source,
                "guarded-httpx rule: transport failures must come back as "
                "{'ok': False, 'error': …}, never crash the shard runner",
            )
            self.assertIn('"ok": False', source)
        # Every precondition path fails with a DISTINCT
        # "probe precondition failed" message — never a vacuous pass, never a
        # shard crash.
        for precondition_helper in (
            run_live_qa._require_slack_personal_token,
            run_live_qa._require_slack_bot_token,
            run_live_qa._require_slack_personal_bot_dm_channel,
            run_live_qa._require_slack_second_user_token,
            run_live_qa._seed_slack_fixture_message,
        ):
            self.assertIn(
                "probe precondition failed",
                inspect.getsource(precondition_helper),
                f"{precondition_helper.__name__} must raise the distinct "
                "precondition message",
            )
        membership_source = inspect.getsource(run_live_qa._slack_membership_view)
        self.assertIn(
            "next_cursor",
            membership_source,
            "users.conversations membership ground truth must paginate; a "
            "truncated member list calls a real membership a lie",
        )
        self.assertIn("exceeded the page cap", membership_source)

    def test_qa_10_seeded_cases_anchor_on_personal_bot_dm(self):
        # Live run 29062917993: the delivery DM is the bot↔bound-webchat-user
        # conversation, and the personal (xoxp) token's human is NOT a member
        # of it, so every personal-token chat.postMessage/read there failed
        # with channel_not_found. The seeded cases must anchor on the
        # personal↔bot DM instead — never on the delivery DM.
        import inspect

        for case_fn in (
            run_live_qa.case_qa_10a_slack_self_attribution,
            run_live_qa.case_qa_10c_slack_thread_replies,
            run_live_qa.case_qa_10f_slack_mention_encoding,
            run_live_qa.case_qa_10g_slack_last_message_sent,
            run_live_qa.case_qa_10g_slack_last_message_sent_global,
            run_live_qa.case_qa_10h_slack_email_hallucination_guard,
            run_live_qa.case_qa_10i_slack_raw_entity_hygiene,
        ):
            source = inspect.getsource(case_fn)
            self.assertIn(
                "_slack_personal_bot_dm_channel",
                source,
                f"{case_fn.__name__} must seed/read via the personal↔bot DM",
            )
            self.assertNotIn(
                "_slack_delivery_channel_id",
                source,
                f"{case_fn.__name__} must not anchor on the delivery DM — "
                "the personal token's user is not a member of it",
            )

    def test_find_im_channel_for_user_selects_exact_counterpart(self):
        channels: list[object] = [
            "not-a-dict",
            {"id": "D0OTHER001", "user": "U0HUMAN001"},
            {"user": "U0BOTUSER1"},  # no id — must be skipped
            {"id": "D0BOTDM001", "user": "U0BOTUSER1"},
        ]
        self.assertEqual(
            run_live_qa._find_im_channel_for_user(channels, "U0BOTUSER1"),
            "D0BOTDM001",
        )
        self.assertIsNone(
            run_live_qa._find_im_channel_for_user(channels, "U0NOWHERE1")
        )
        self.assertIsNone(run_live_qa._find_im_channel_for_user(channels, ""))
        self.assertIsNone(run_live_qa._find_im_channel_for_user([], "U0BOTUSER1"))

    def test_slack_personal_bot_dm_channel_resolves_from_im_scan_and_caches(self):
        env = {
            "AUTH_LIVE_SLACK_ACCESS_TOKEN": "xoxp-personal",
            "IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-bot",
        }

        async def fake_auth_identity(token):
            self.assertEqual(token, "xoxb-bot", "bot user id comes from the BOT token")
            return {"ok": True, "user_id": "U0BOTUSER1"}

        pages = [
            {
                "ok": True,
                "channels": [{"id": "D0OTHER001", "user": "U0HUMAN001"}],
                "response_metadata": {"next_cursor": "cur2"},
            },
            {
                "ok": True,
                "channels": [{"id": "D0BOTDM001", "user": "U0BOTUSER1"}],
            },
        ]
        get_calls: list[dict[str, str]] = []

        async def fake_api_get(token, method, params=None):
            self.assertEqual(token, "xoxp-personal", "the im scan runs as the personal user")
            self.assertEqual(method, "conversations.list")
            self.assertEqual((params or {}).get("types"), "im")
            get_calls.append(dict(params or {}))
            return pages[len(get_calls) - 1]

        async def forbidden_api_post(_token, _method, _payload):
            raise AssertionError(
                "conversations.open must not run when the im scan finds the DM"
            )

        ctx = self._dummy_ctx()
        ctx.env = env
        with (
            patch.object(
                run_live_qa, "_slack_auth_identity", side_effect=fake_auth_identity
            ),
            patch.object(run_live_qa, "_slack_api_get", side_effect=fake_api_get),
            patch.object(
                run_live_qa, "_slack_api_post", side_effect=forbidden_api_post
            ),
        ):
            resolved = asyncio.run(run_live_qa._slack_personal_bot_dm_channel(ctx))
            self.assertEqual(
                resolved,
                {
                    "ok": True,
                    "channel_id": "D0BOTDM001",
                    "bot_user_id": "U0BOTUSER1",
                    "opened": False,
                },
            )
            self.assertEqual(
                get_calls[1].get("cursor"),
                "cur2",
                "the im scan must follow next_cursor — one page can hide the bot DM",
            )
            channel_id = asyncio.run(
                run_live_qa._require_slack_personal_bot_dm_channel(ctx)
            )
        self.assertEqual(channel_id, "D0BOTDM001")
        self.assertEqual(
            len(get_calls),
            2,
            "a successful resolution is cached on ctx — the require call "
            "must not re-hit the API",
        )

    def test_slack_personal_bot_dm_channel_open_fallback_and_failure(self):
        env = {
            "AUTH_LIVE_SLACK_ACCESS_TOKEN": "xoxp-personal",
            "IRONCLAW_REBORN_SLACK_BOT_TOKEN": "xoxb-bot",
        }

        async def fake_auth_identity(_token):
            return {"ok": True, "user_id": "U0BOTUSER1"}

        async def fake_api_get(_token, _method, _params=None):
            # No im with the bot anywhere in the (single-page) scan.
            return {
                "ok": True,
                "channels": [{"id": "D0OTHER001", "user": "U0HUMAN001"}],
            }

        open_response: dict[str, object] = {
            "ok": True,
            "channel": {"id": "D0OPENED01"},
        }

        async def fake_api_post(token, method, payload):
            self.assertEqual(token, "xoxp-personal")
            self.assertEqual(method, "conversations.open")
            self.assertEqual(payload, {"users": "U0BOTUSER1"})
            return dict(open_response)

        patches = (
            patch.object(
                run_live_qa, "_slack_auth_identity", side_effect=fake_auth_identity
            ),
            patch.object(run_live_qa, "_slack_api_get", side_effect=fake_api_get),
            patch.object(run_live_qa, "_slack_api_post", side_effect=fake_api_post),
        )
        ctx = self._dummy_ctx()
        ctx.env = env
        with patches[0], patches[1], patches[2]:
            resolved = asyncio.run(run_live_qa._slack_personal_bot_dm_channel(ctx))
            self.assertEqual(resolved["ok"], True)
            self.assertEqual(resolved["channel_id"], "D0OPENED01")
            self.assertTrue(resolved["opened"])

            # A rejected open (e.g. personal token without im:write) must come
            # back as {"ok": False, …} — and the require wrapper must raise
            # the distinct precondition failure, never a vacuous pass.
            open_response.clear()
            open_response.update({"ok": False, "error": "missing_scope"})
            fresh_ctx = self._dummy_ctx()
            fresh_ctx.env = env
            failed = asyncio.run(
                run_live_qa._slack_personal_bot_dm_channel(fresh_ctx)
            )
            self.assertEqual(failed, {"ok": False, "error": "missing_scope"})
            with self.assertRaisesRegex(
                AssertionError,
                r"probe precondition failed: could not resolve the "
                r"personal↔bot DM: missing_scope",
            ):
                asyncio.run(
                    run_live_qa._require_slack_personal_bot_dm_channel(fresh_ctx)
                )

    def test_routine_confirmation_follow_up_respects_timezone_instruction(self):
        text = "I can create that routine — should I go ahead?"
        default_reply = run_live_qa._routine_confirmation_follow_up_for_text(text)
        self.assertIn("Europe/London", default_reply)
        utc_reply = run_live_qa._routine_confirmation_follow_up_for_text(
            text,
            schedule_timezone_instruction="Use the UTC timezone for the schedule.",
        )
        self.assertIn("UTC", utc_reply)
        self.assertNotIn("Europe/London", utc_reply)

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

        self.assertEqual(len(selected_cases), 47)
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
            "qa_9b_routine_dm_delivery_exactly_once",
            "qa_9c_slack_digest_names_not_ids",
            "qa_9d_routine_per_trigger_delivery_target",
            "qa_10a_slack_self_attribution",
            "qa_10b_slack_ooo_status",
            "qa_10c_slack_thread_replies",
            "qa_10d_slack_channel_membership",
            "qa_10e_slack_error_honesty",
            "qa_10f_slack_mention_encoding",
            "qa_10g_slack_last_message_sent",
            "qa_10g_slack_last_message_sent_global",
            "qa_10h_slack_email_hallucination_guard",
            "qa_10i_slack_raw_entity_hygiene",
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
        self.assertEqual(len(shard_case_lines), 12)
        sharded_cases = [
            case_name
            for line in shard_case_lines
            for case_name in line.split(",")
            if case_name
        ]

        self.assertEqual(len(sharded_cases), len(set(sharded_cases)))
        self.assertEqual(sharded_cases, selected_cases)
        self.assertIn("qa_10g_slack_last_message_sent", sharded_cases)
        self.assertIn("qa_10g_slack_last_message_sent_global", sharded_cases)
        # QA 9/10 are promoted: no shard in the matrix is dispatch_only any
        # more, so every shard (qa-9/qa-10 included) runs on the 3-hourly
        # schedule and on a default cases=all dispatch. The resolve-step
        # guard itself stays for any FUTURE expected-red shard.
        matrix_match = re.search(
            r"(?ms)^\s+include:\n(?P<matrix>.*?)^\s+env:", match.group("body")
        )
        self.assertIsNotNone(matrix_match, "live QA shard matrix missing")
        self.assertNotIn(
            "dispatch_only",
            matrix_match.group("matrix"),
            "no live QA shard is dispatch_only; qa-9/qa-10 are promoted into "
            "the cron rotation — only add dispatch_only for a NEW "
            "expected-red shard",
        )
        self.assertIn(
            "Shard is dispatch_only; skipping on schedule.",
            match.group("body"),
            "keep the resolve-step dispatch_only guard for future "
            "expected-red shards",
        )
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
        self.assertIn("target_pr is required when target_ref is supplied", workflow)
        self.assertIn(
            "Refusing to run reborn-webui-v2-live-qa with live secrets on a forked PR",
            workflow,
        )
        self.assertIn("target_ref does not match PR #${TARGET_PR} head SHA", workflow)
        self.assertIn("approve-reborn-webui-v2-pr-live-qa:", workflow)
        self.assertIn("environment: reborn-live-canary-pr", workflow)
        self.assertIn(
            "requires either an approving review from a collaborator with write access or a trigger by a collaborator with write access",
            workflow,
        )
        self.assertIn("trigger_actor:", workflow)
        self.assertIn(
            "inputs.trigger_actor || github.triggering_actor",
            workflow,
        )
        self.assertIn("(authorized trigger)", workflow)
        self.assertIn("- prepare-reborn-webui-v2-live-qa", match.group("body"))
        self.assertIn(
            "- preflight-reborn-webui-v2-google-oauth",
            match.group("body"),
        )
        self.assertIn("always() &&", match.group("body"))
        self.assertIn(
            "needs.prepare-reborn-webui-v2-live-qa.result == 'success'",
            match.group("body"),
        )
        self.assertIn("github.event_name == 'schedule'", match.group("body"))
        self.assertNotIn("github.event.schedule ==", match.group("body"))
        self.assertIn(
            "ref: ${{ needs.prepare-reborn-webui-v2-live-qa.outputs.checkout_ref }}",
            match.group("body"),
        )
        self.assertIn('SKIP_BUILD: "1"', match.group("body"))
        self.assertIn("REBORN_WEBUI_V2_LIVE_QA_BUILD_SOURCE", match.group("body"))
        self.assertIn("Cache Playwright browsers", match.group("body"))
        self.assertIn("cache: pip", match.group("body"))
        self.assertNotIn("Build WASM channels", match.group("body"))
        self.assertNotIn("Setup OVH sccache", match.group("body"))
        self.assertIn(
            "Suppress Google cases after OAuth preflight failure",
            match.group("body"),
        )
        self.assertIn(
            "Mint fresh Google OAuth access token for selected cases",
            match.group("body"),
        )
        self.assertIn("REBORN_WEBUI_V2_GOOGLE_CASES", match.group("body"))
        self.assertEqual(
            len(re.findall(r"^    env:\s*$", match.group("body"), re.M)),
            1,
            "the job must have one env mapping so Google-case gating is not overwritten",
        )
        self.assertEqual(
            match.group("body").count(".github/scripts/google_oauth_cases.py"),
            3,
            "preflight filtering, mint selection, and mint-failure filtering "
            "must use the same executable case classifier",
        )
        self.assertNotIn("google_cases='", match.group("body"))
        self.assertIn(
            "steps.mint_reborn_webui_v2_google_token.outputs.skip_shard != '1'",
            match.group("body"),
        )
        self.assertIn(
            "AUTH_LIVE_GOOGLE_ACCESS_TOKEN_PATH=${access_token_path}",
            match.group("body"),
        )

        google_preflight_match = re.search(
            r"(?ms)^  preflight-reborn-webui-v2-google-oauth:\n"
            r"(?P<body>.*?)^  prepare-reborn-webui-v2-live-qa:",
            workflow,
        )
        self.assertIsNotNone(
            google_preflight_match,
            "shared Google OAuth preflight job missing",
        )
        google_preflight_body = google_preflight_match.group("body")
        self.assertIn("refresh_google_oauth.py", google_preflight_body)
        self.assertIn("continue-on-error: true", google_preflight_body)
        self.assertIn(
            "Google-dependent cases will be skipped; non-Google cases will continue",
            google_preflight_body,
        )
        self.assertIn(
            "REBORN_GOOGLE_OAUTH_PREFLIGHT_STATUS: "
            "${{ needs.preflight-reborn-webui-v2-google-oauth.outputs.status }}",
            workflow,
        )

        prepare_match = re.search(
            r"(?ms)^  prepare-reborn-webui-v2-live-qa:\n(?P<body>.*?)^  reborn-webui-v2-live-qa:",
            workflow,
        )
        self.assertIsNotNone(prepare_match, "shared live QA binary preparation job missing")
        prepare_body = prepare_match.group("body")
        self.assertIn("needs: approve-reborn-webui-v2-pr-live-qa", prepare_body)
        self.assertIn("reborn-webui-v2-binary-${TARGET_REF}", prepare_body)
        self.assertIn("Build fallback Reborn WebUI v2 binary once", prepare_body)
        self.assertIn("if ! (", prepare_body)
        self.assertIn("validate_reborn_binary_artifact.py", prepare_body)
        self.assertIn(
            "--features webui-v2-beta,slack-v2-host-beta",
            prepare_body,
        )
        self.assertIn(
            "webui-v2-beta,slack-v2-host-beta",
            prepare_body,
        )
        self.assertIn("cp target/debug/ironclaw target/debug/ironclaw-reborn", prepare_body)
        self.assertIn(
            "ironclaw-reborn.tar.gz ironclaw ironclaw-reborn",
            prepare_body,
        )
        self.assertIn("using the canary fallback build", prepare_body)
        self.assertIn("prepared-reborn-webui-v2-binary-${{ steps.target.outputs.checkout_ref }}", prepare_body)
        self.assertIn("path: artifacts/prepared-reborn-webui-v2-binary/", prepare_body)

        reborn_e2e_path = (
            Path(__file__).resolve().parents[2] / ".github/workflows/reborn-e2e.yml"
        )
        reborn_e2e = reborn_e2e_path.read_text(encoding="utf-8")
        self.assertIn(
            "github.event.pull_request.head.sha",
            reborn_e2e,
        )
        self.assertIn(
            "--features openai-compat-beta,slack-v2-host-beta",
            reborn_e2e,
        )
        self.assertIn(
            '["openai-compat-beta","slack-v2-host-beta","webui-v2-beta"]',
            reborn_e2e,
        )
        self.assertIn("cp target/debug/ironclaw target/debug/ironclaw-reborn", reborn_e2e)
        self.assertIn(
            "ironclaw-reborn.tar.gz\" ironclaw ironclaw-reborn",
            reborn_e2e,
        )
        self.assertIn(
            "name: reborn-webui-v2-binary-${{ steps.live_canary_binary.outputs.product_ref }}",
            reborn_e2e,
        )

        command_workflow_path = (
            Path(__file__).resolve().parents[2] / ".github/workflows/live-canary-command.yml"
        )
        command_workflow = command_workflow_path.read_text(encoding="utf-8")
        self.assertIn('-f target_ref="$HEAD_SHA"', command_workflow)
        self.assertIn('-f target_pr="$PR"', command_workflow)
        self.assertIn('TRIGGER_ACTOR: ${{ github.event.comment.user.login }}', command_workflow)
        self.assertIn('-f trigger_actor="$TRIGGER_ACTOR"', command_workflow)

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
        self.assertTrue(cases["qa_7c_slack_bug_logger_routine"]["implemented"])
        self.assertTrue(cases["qa_7c_slack_bug_logger_routine"]["requires_slack_target"])
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
            with closing(sqlite3.connect(db_path)) as db:
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
                db.commit()
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
                db.commit()

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
            with closing(sqlite3.connect(db_path)) as db:
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
                db.commit()

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

    def test_trigger_run_slack_send_evidence_is_exact_run_scoped_and_sanitized(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            home = Path(tmpdir) / "reborn-home"
            db_path = home / "local-dev" / "reborn-local-dev.db"
            run_live_qa._root_filesystem_create_table(db_path)
            expected_channel = "D_EXPECTED_PRIVATE_CHANNEL"
            marker = "PRIVATE_QA_DELIVERY_MARKER"

            def put_preview(path_suffix: str, *, run_id: str, channel: str) -> None:
                run_live_qa._put_root_filesystem_json(
                    db_path,
                    (
                        "/tenants/reborn-cli/users/test/threads/agents/reborn-cli-agent/"
                        "owners/test/threads/thread-target/messages/"
                        f"{path_suffix}.json"
                    ),
                    {
                        "turn_run_id": run_id,
                        "kind": "capability_display_preview",
                        "content": json.dumps(
                            {
                                "capability_id": "slack.send_message",
                                "status": "completed",
                                "input_summary": json.dumps(
                                    {"channel": channel, "text": f"result {marker}"}
                                ),
                                "output_preview": json.dumps(
                                    {"channel": channel, "ok": True, "ts": "1.2"}
                                ),
                            }
                        ),
                    },
                )

            put_preview("expected", run_id="run-target", channel=expected_channel)
            put_preview("other-run", run_id="run-distractor", channel="D_OTHER")

            evidence = run_live_qa._trigger_run_slack_send_evidence(
                home,
                run_id="run-target",
                thread_id="thread-target",
                expected_channel_id=expected_channel,
                marker=marker,
            )

        self.assertEqual(evidence["completed_send_count"], 1)
        self.assertEqual(evidence["marker_send_count"], 1)
        self.assertEqual(evidence["expected_channel_marker_send_count"], 1)
        self.assertEqual(evidence["expected_channel_marker_ok_count"], 1)
        self.assertEqual(evidence["wrong_channel_marker_send_count"], 0)
        serialized = json.dumps(evidence, sort_keys=True)
        self.assertNotIn(expected_channel, serialized)
        self.assertNotIn(marker, serialized)

    def test_slack_delivery_readback_inconclusive_requires_one_verified_send(self):
        history_miss = {"checked": True, "found": False, "message_count": 7}
        delivered = {"outcome": "delivered"}
        exact_send = {
            "completed_send_count": 1,
            "marker_send_count": 1,
            "expected_channel_marker_send_count": 1,
            "expected_channel_marker_ok_count": 1,
            "wrong_channel_marker_send_count": 0,
            "parse_error_count": 0,
        }

        self.assertTrue(
            run_live_qa._slack_delivery_readback_is_inconclusive(
                delivered,
                history_miss,
                exact_send,
            )
        )
        for changed in (
            {"completed_send_count": 2, "marker_send_count": 2},
            {"wrong_channel_marker_send_count": 1},
            {"expected_channel_marker_ok_count": 0},
            {"parse_error_count": 1},
        ):
            with self.subTest(changed=changed):
                self.assertFalse(
                    run_live_qa._slack_delivery_readback_is_inconclusive(
                        delivered,
                        history_miss,
                        {**exact_send, **changed},
                    )
                )
        self.assertFalse(
            run_live_qa._slack_delivery_readback_is_inconclusive(
                delivered,
                {**history_miss, "error": "missing_scope"},
                exact_send,
            )
        )

    def test_slack_delivery_routine_readback_miss_is_infrastructure_inconclusive(self):
        async def fake_creation(_ctx, **kwargs):
            return run_live_qa.ProbeResult(
                provider="test",
                mode=f"live:{kwargs['case_name']}",
                success=True,
                latency_ms=1,
                details={"creation": "ok"},
            )

        evidence = {
            "completed_send_count": 1,
            "marker_send_count": 1,
            "expected_channel_marker_send_count": 1,
            "expected_channel_marker_ok_count": 1,
            "wrong_channel_marker_send_count": 0,
            "parse_error_count": 0,
        }

        async def fake_wait(*_args, **_kwargs):
            raise run_live_qa.SlackDeliveryReadbackInconclusive(
                "Slack accepted the exact-run send but history did not expose it",
                evidence,
            )

        with (
            patch.object(
                run_live_qa,
                "_routine_creation_case",
                side_effect=fake_creation,
            ),
            patch.object(
                run_live_qa,
                "_wait_for_slack_delivery_marker",
                side_effect=fake_wait,
            ),
        ):
            result = asyncio.run(
                run_live_qa._slack_delivery_routine_case(
                    self._dummy_ctx(),
                    case_name="qa_9b_routine_dm_delivery_exactly_once",
                    routine_prefix="qa-9b",
                    marker_prefix="QA_9B",
                    routine_instruction="send the marker to Slack",
                    required_delivery_text=["status"],
                )
            )

        self.assertFalse(result.success)
        self.assertEqual(result.details["failure_class"], "infrastructure")
        self.assertEqual(
            result.details["failure_category"],
            "slack_delivery_readback_unavailable",
        )
        self.assertEqual(result.details["failure_status"], "inconclusive")
        self.assertTrue(result.details["inconclusive"])
        self.assertFalse(result.details["blocking"])
        self.assertEqual(result.details["delivery_readback_evidence"], evidence)

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

    def test_case_manifest_and_results_include_tier_and_blocking_metadata(self):
        async def fake_case(_ctx):
            return None

        cases = {
            "contract_case": run_live_qa.CaseSpec(
                fake_case,
                tier="contract",
                blocking=True,
            ),
            "behavioral_case": run_live_qa.CaseSpec(
                fake_case,
                tier="behavioral",
                blocking=False,
            ),
        }
        failure_details = {
            "error": "observed mismatch",
            "failure_class": "product",
            "failure_category": "assertion_mismatch",
            "failure_status": "failed",
        }

        with (
            tempfile.TemporaryDirectory() as tmpdir,
            patch.object(run_live_qa, "CASES", cases),
            patch.object(run_live_qa, "QA_SHEET_CASES", {}),
        ):
            manifest_path = run_live_qa.write_case_manifest(
                Path(tmpdir),
                list(cases),
            )
            contract_result = run_live_qa._result(
                "contract_case",
                False,
                run_live_qa.time.monotonic(),
                dict(failure_details),
            )
            behavioral_result = run_live_qa._result(
                "behavioral_case",
                False,
                run_live_qa.time.monotonic(),
                dict(failure_details),
            )
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

        manifest_cases = {entry["case"]: entry for entry in manifest["cases"]}
        self.assertEqual(manifest_cases["contract_case"]["case_tier"], "contract")
        self.assertTrue(manifest_cases["contract_case"]["blocking"])
        self.assertEqual(
            manifest_cases["behavioral_case"]["case_tier"],
            "behavioral",
        )
        self.assertFalse(manifest_cases["behavioral_case"]["blocking"])

        self.assertFalse(contract_result.success)
        self.assertFalse(behavioral_result.success)
        self.assertEqual(contract_result.details["case_tier"], "contract")
        self.assertTrue(contract_result.details["blocking"])
        self.assertEqual(behavioral_result.details["case_tier"], "behavioral")
        self.assertFalse(behavioral_result.details["blocking"])
        for key, value in failure_details.items():
            self.assertEqual(contract_result.details[key], value)
            self.assertEqual(behavioral_result.details[key], value)

    def test_run_cases_exits_only_for_blocking_failures(self):
        def run_failed_case(*, tier: str, blocking: bool) -> tuple[int, dict]:
            case_name = f"{tier}_case"

            async def fake_case(_ctx):
                return run_live_qa._result(
                    case_name,
                    False,
                    run_live_qa.time.monotonic(),
                    {
                        "error": "observed mismatch",
                        "failure_class": "product",
                        "failure_category": "assertion_mismatch",
                        "failure_status": "failed",
                    },
                )

            async def fake_start_reborn_server(*_args, **_kwargs):
                return object(), "http://127.0.0.1:38555"

            with tempfile.TemporaryDirectory() as tmpdir:
                root = Path(tmpdir)
                output_dir = root / "out"
                binary = root / "ironclaw"
                binary.touch()
                prepared_home = root / "prepared-home"
                prepared_home.mkdir()
                args = argparse.Namespace(
                    all_cases=False,
                    non_telegram_qa_cases=False,
                    case=[case_name],
                    output_dir=output_dir,
                    reborn_home=root / "source-home",
                    skip_build=True,
                    require_slack_live=False,
                )
                cases = {
                    case_name: run_live_qa.CaseSpec(
                        fake_case,
                        tier=tier,
                        blocking=blocking,
                    )
                }
                prepared = run_live_qa.PreparedRebornHome(
                    path=prepared_home,
                    preflight={
                        "slack": {},
                        "slack_personal_auth": {},
                        "google_product_auth": {},
                        "telegram": {},
                        "github_auth": {},
                    },
                )
                with (
                    patch.object(run_live_qa, "CASES", cases),
                    patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                    patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                    patch.object(
                        run_live_qa,
                        "prepare_reborn_home",
                        return_value=prepared,
                    ),
                    patch.object(
                        run_live_qa,
                        "start_reborn_server",
                        side_effect=fake_start_reborn_server,
                    ),
                    patch.object(run_live_qa, "stop_process"),
                ):
                    status = asyncio.run(run_live_qa.run_cases(args))
                payload = json.loads(
                    (output_dir / "results.json").read_text(encoding="utf-8")
                )
                return status, payload["results"][0]

        behavioral_status, behavioral_result = run_failed_case(
            tier="behavioral",
            blocking=False,
        )
        contract_status, contract_result = run_failed_case(
            tier="contract",
            blocking=True,
        )

        self.assertEqual(behavioral_status, 0)
        self.assertFalse(behavioral_result["success"])
        self.assertEqual(behavioral_result["details"]["case_tier"], "behavioral")
        self.assertFalse(behavioral_result["details"]["blocking"])
        self.assertEqual(contract_status, 1)
        self.assertFalse(contract_result["success"])
        self.assertEqual(contract_result["details"]["case_tier"], "contract")
        self.assertTrue(contract_result["details"]["blocking"])

    def test_model_unavailable_short_circuits_remaining_cases_as_inconclusive(self):
        started_servers: list[str] = []
        prepared_cases: list[str] = []

        async def unavailable_case(_ctx):
            return run_live_qa._result(
                "unavailable_case",
                False,
                run_live_qa.time.monotonic(),
                {
                    "error": "The configured model provider is unavailable.",
                    "failure_category": "model_unavailable",
                    "failure_status": "failed",
                },
            )

        async def later_case(_ctx):
            raise AssertionError("later case must be inconclusive without running")

        async def fake_start_reborn_server(
            _binary,
            reborn_home,
            _output_dir,
            _env,
        ):
            started_servers.append(reborn_home.name)
            return object(), "http://127.0.0.1:38555"

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw"
            binary.touch()
            args = argparse.Namespace(
                all_cases=False,
                non_telegram_qa_cases=False,
                case=["unavailable_case", "later_case"],
                output_dir=output_dir,
                reborn_home=root / "source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "unavailable_case": run_live_qa.CaseSpec(
                    unavailable_case,
                    tier="contract",
                    blocking=True,
                ),
                "later_case": run_live_qa.CaseSpec(
                    later_case,
                    tier="behavioral",
                    blocking=False,
                ),
            }

            def fake_prepare_reborn_home(_args, _selected, *, case_name):
                prepared_cases.append(case_name)
                home = output_dir / "reborn-home" / case_name
                home.mkdir(parents=True)
                return run_live_qa.PreparedRebornHome(
                    path=home,
                    preflight={
                        "slack": {},
                        "slack_personal_auth": {},
                        "google_product_auth": {},
                        "telegram": {},
                        "github_auth": {},
                    },
                )

            with (
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(
                    run_live_qa,
                    "prepare_reborn_home",
                    side_effect=fake_prepare_reborn_home,
                ),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fake_start_reborn_server,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            payload = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )

        self.assertEqual(status, 0)
        self.assertEqual(prepared_cases, ["unavailable_case"])
        self.assertEqual(started_servers, ["unavailable_case"])
        self.assertEqual(len(payload["results"]), 2)
        unavailable_result, inconclusive_result = payload["results"]
        self.assertFalse(unavailable_result["success"])
        self.assertEqual(
            unavailable_result["details"]["failure_class"],
            "infrastructure",
        )
        self.assertEqual(
            unavailable_result["details"]["failure_category"],
            "model_unavailable",
        )
        self.assertEqual(unavailable_result["details"]["failure_status"], "failed")
        self.assertFalse(unavailable_result["details"]["blocking"])
        self.assertTrue(unavailable_result["details"]["inconclusive"])

        self.assertFalse(inconclusive_result["success"])
        self.assertEqual(inconclusive_result["details"]["case"], "later_case")
        self.assertEqual(inconclusive_result["details"]["case_tier"], "behavioral")
        self.assertFalse(inconclusive_result["details"]["blocking"])
        self.assertTrue(inconclusive_result["details"]["inconclusive"])
        self.assertEqual(
            inconclusive_result["details"]["failure_class"],
            "infrastructure",
        )
        self.assertEqual(
            inconclusive_result["details"]["failure_category"],
            "model_unavailable",
        )
        self.assertEqual(
            inconclusive_result["details"]["failure_status"],
            "inconclusive",
        )
        self.assertIn("unavailable_case", inconclusive_result["details"]["error"])

    def test_model_transient_short_circuits_remaining_cases_as_inconclusive(self):
        started_servers: list[str] = []
        prepared_cases: list[str] = []

        async def transient_case(_ctx):
            return run_live_qa._result(
                "transient_case",
                False,
                run_live_qa.time.monotonic(),
                {
                    "error": "The configured model provider had a transient failure.",
                    "failure_category": "model_transient",
                    "failure_status": "failed",
                },
            )

        async def later_case(_ctx):
            raise AssertionError("later case must be inconclusive without running")

        async def fake_start_reborn_server(
            _binary,
            reborn_home,
            _output_dir,
            _env,
        ):
            started_servers.append(reborn_home.name)
            return object(), "http://127.0.0.1:38555"

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw"
            binary.touch()
            args = argparse.Namespace(
                all_cases=False,
                non_telegram_qa_cases=False,
                case=["transient_case", "later_case"],
                output_dir=output_dir,
                reborn_home=root / "source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "transient_case": run_live_qa.CaseSpec(
                    transient_case,
                    tier="contract",
                    blocking=True,
                ),
                "later_case": run_live_qa.CaseSpec(
                    later_case,
                    tier="behavioral",
                    blocking=False,
                ),
            }

            def fake_prepare_reborn_home(_args, _selected, *, case_name):
                prepared_cases.append(case_name)
                home = output_dir / "reborn-home" / case_name
                home.mkdir(parents=True)
                return run_live_qa.PreparedRebornHome(
                    path=home,
                    preflight={
                        "slack": {},
                        "slack_personal_auth": {},
                        "google_product_auth": {},
                        "telegram": {},
                        "github_auth": {},
                    },
                )

            with (
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(
                    run_live_qa,
                    "prepare_reborn_home",
                    side_effect=fake_prepare_reborn_home,
                ),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fake_start_reborn_server,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            payload = json.loads(
                (output_dir / "results.json").read_text(encoding="utf-8")
            )

        self.assertEqual(status, 0)
        self.assertEqual(prepared_cases, ["transient_case"])
        self.assertEqual(started_servers, ["transient_case"])
        self.assertEqual(len(payload["results"]), 2)
        transient_result, inconclusive_result = payload["results"]
        self.assertFalse(transient_result["success"])
        self.assertEqual(
            transient_result["details"]["failure_class"],
            "infrastructure",
        )
        self.assertEqual(
            transient_result["details"]["failure_category"],
            "model_transient",
        )
        self.assertFalse(transient_result["details"]["blocking"])
        self.assertTrue(transient_result["details"]["inconclusive"])
        self.assertFalse(inconclusive_result["success"])
        self.assertEqual(inconclusive_result["details"]["case"], "later_case")
        self.assertFalse(inconclusive_result["details"]["blocking"])
        self.assertTrue(inconclusive_result["details"]["inconclusive"])
        self.assertEqual(
            inconclusive_result["details"]["failure_category"],
            "model_transient",
        )
        self.assertEqual(
            inconclusive_result["details"]["failure_status"],
            "inconclusive",
        )
        self.assertIn("transient_case", inconclusive_result["details"]["error"])

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
            binary = root / "ironclaw"
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
            self.assertTrue((output_dir / "green-run-explanation.json").exists())
            green_explanation = json.loads(
                (output_dir / "green-run-explanation.json").read_text(encoding="utf-8")
            )
            self.assertEqual(green_explanation["successful_cases"], 2)

    def test_run_cases_blocks_slack_connect_without_personal_product_auth(self):
        async def fake_case(_ctx: run_live_qa.LiveQaContext) -> run_live_qa.ProbeResult:
            raise AssertionError("case should not run without Slack personal auth")

        async def fail_start_reborn_server(*_args, **_kwargs):
            raise AssertionError("server should not start without Slack personal auth")

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw"
            binary.touch()
            args = argparse.Namespace(
                all_cases=False,
                non_telegram_qa_cases=False,
                case=["case_slack_connect"],
                output_dir=output_dir,
                reborn_home=root / "missing-source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "case_slack_connect": run_live_qa.CaseSpec(
                    fake_case,
                    requires_slack_personal_auth=True,
                )
            }
            env = {
                "LIVE_OPENAI_COMPATIBLE_API_KEY": "fake-live-llm-key",
                "REBORN_WEBUI_V2_LIVE_QA_LLM_API_KEY_ENV": "LIVE_OPENAI_COMPATIBLE_API_KEY",
                "AUTH_LIVE_SLACK_ACCESS_TOKEN": "",
                "AUTH_LIVE_SLACK_ACCESS_TOKEN_PATH": "",
            }

            with (
                patch.dict(os.environ, env, clear=False),
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fail_start_reborn_server,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            self.assertEqual(status, 1)
            payload = json.loads((output_dir / "results.json").read_text(encoding="utf-8"))
            result = payload["results"][0]
            self.assertFalse(result["success"])
            self.assertTrue(result["details"]["blocked"])
            self.assertEqual(
                result["details"]["error"],
                "no Slack personal product-auth DB is present",
            )
            self.assertIn(
                "AUTH_LIVE_SLACK_ACCESS_TOKEN",
                result["details"]["required_env"],
            )

    def test_run_cases_blocks_slack_workspace_mismatch(self):
        async def fake_case(_ctx: run_live_qa.LiveQaContext) -> run_live_qa.ProbeResult:
            raise AssertionError("case should not run with mismatched Slack teams")

        async def fail_start_reborn_server(*_args, **_kwargs):
            raise AssertionError("server should not start with mismatched Slack teams")

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw"
            binary.touch()
            prepared_home = root / "prepared-home"
            prepared_home.mkdir()
            args = argparse.Namespace(
                all_cases=False,
                non_telegram_qa_cases=False,
                case=["case_slack_connect"],
                output_dir=output_dir,
                reborn_home=root / "missing-source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "case_slack_connect": run_live_qa.CaseSpec(
                    fake_case,
                    requires_slack=True,
                    requires_slack_personal_auth=True,
                )
            }
            prepared = run_live_qa.PreparedRebornHome(
                path=prepared_home,
                preflight={
                    "slack": {
                        "enabled_in_config": True,
                        "env_present": True,
                        "setup": {
                            "configured": True,
                            "team_id": "T-BOT",
                        },
                        "auth_test": {
                            "ok": True,
                            "team_id": "T-BOT",
                        },
                    },
                    "slack_personal_auth": {
                        "ready": True,
                        "auth_test": {
                            "ok": True,
                            "team_id": "T-PERSONAL",
                        },
                    },
                    "google_product_auth": {},
                    "telegram": {},
                    "github_auth": {},
                },
            )

            with (
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(run_live_qa, "prepare_reborn_home", return_value=prepared),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fail_start_reborn_server,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            self.assertEqual(status, 1)
            payload = json.loads((output_dir / "results.json").read_text(encoding="utf-8"))
            result = payload["results"][0]
            self.assertFalse(result["success"])
            self.assertTrue(result["details"]["blocked"])
            self.assertIn("different workspaces", result["details"]["error"])
            self.assertIn("bot_token team_id=T-BOT", result["details"]["error"])
            self.assertIn("personal_oauth team_id=T-PERSONAL", result["details"]["error"])

    def test_run_cases_blocks_when_slack_setup_api_is_not_applied(self):
        async def fake_case(_ctx: run_live_qa.LiveQaContext) -> run_live_qa.ProbeResult:
            raise AssertionError("case should not run when Slack setup was not applied")

        async def fake_start_reborn_server(*_args, **_kwargs):
            return object(), "http://127.0.0.1:38555"

        async def fake_apply_slack_setup_api_after_start(*_args, **_kwargs):
            return {
                "applied": False,
                "reason": "setup_payload_missing",
                "missing": ["bot_token"],
            }

        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_dir = root / "out"
            binary = root / "ironclaw"
            binary.touch()
            prepared_home = root / "prepared-home"
            prepared_home.mkdir()
            args = argparse.Namespace(
                all_cases=False,
                non_telegram_qa_cases=False,
                case=["case_slack"],
                output_dir=output_dir,
                reborn_home=root / "missing-source-home",
                skip_build=True,
                require_slack_live=False,
            )
            cases = {
                "case_slack": run_live_qa.CaseSpec(
                    fake_case,
                    requires_slack=True,
                )
            }
            prepared = run_live_qa.PreparedRebornHome(
                path=prepared_home,
                env={},
                preflight={
                    "slack": {
                        "enabled_in_config": True,
                        "env_present": True,
                        "delivery_target_present": True,
                        "auth_test": {
                            "ok": True,
                            "team_id": "T123",
                        },
                    },
                    "slack_personal_auth": {},
                    "google_product_auth": {},
                    "telegram": {},
                    "github_auth": {},
                },
            )

            with (
                patch.object(run_live_qa, "CASES", cases),
                patch.object(run_live_qa, "QA_SHEET_CASES", {}),
                patch.object(run_live_qa, "_reborn_binary", return_value=binary),
                patch.object(run_live_qa, "prepare_reborn_home", return_value=prepared),
                patch.object(
                    run_live_qa,
                    "start_reborn_server",
                    side_effect=fake_start_reborn_server,
                ),
                patch.object(
                    run_live_qa,
                    "_apply_slack_setup_api_after_start",
                    side_effect=fake_apply_slack_setup_api_after_start,
                ),
                patch.object(run_live_qa, "stop_process"),
            ):
                status = asyncio.run(run_live_qa.run_cases(args))

            self.assertEqual(status, 1)
            payload = json.loads((output_dir / "results.json").read_text(encoding="utf-8"))
            result = payload["results"][0]
            self.assertFalse(result["success"])
            self.assertTrue(result["details"]["blocked"])
            self.assertEqual(
                result["details"]["error"],
                "Slack setup API was not applied: setup_payload_missing",
            )
            self.assertEqual(
                result["details"]["preflight"]["setup_api"]["missing"],
                ["bot_token"],
            )


class RunCaseWithRetriesTests(unittest.TestCase):
    @staticmethod
    def _probe(success: bool, details: dict[str, object] | None = None):
        return run_live_qa.ProbeResult(
            provider="test",
            mode="live:case",
            success=success,
            latency_ms=1,
            details=dict(details or {}),
        )

    def test_retries_transient_failure_then_returns_success(self):
        calls = {"count": 0}

        async def flaky_fn(_ctx):
            calls["count"] += 1
            if calls["count"] == 1:
                return self._probe(False, {"error": "assertion mismatch"})
            return self._probe(True, {"text_excerpt": "ok"})

        result = asyncio.run(
            run_live_qa._run_case_with_retries(
                flaky_fn,
                object(),
                attempts=2,
                is_retriable=run_live_qa._is_case_retriable,
            )
        )

        self.assertTrue(result.success)
        self.assertEqual(result.details["attempts"], 2)
        self.assertEqual(calls["count"], 2)

    def test_does_not_retry_blocked_failure(self):
        calls = {"count": 0}

        async def blocked_fn(_ctx):
            calls["count"] += 1
            return self._probe(False, {"blocked": True, "error": "precondition"})

        result = asyncio.run(
            run_live_qa._run_case_with_retries(
                blocked_fn,
                object(),
                attempts=2,
                is_retriable=run_live_qa._is_case_retriable,
            )
        )

        self.assertFalse(result.success)
        self.assertEqual(result.details["attempts"], 1)
        self.assertEqual(calls["count"], 1)
        self.assertTrue(result.details["blocked"])

    def test_does_not_retry_infrastructure_failure(self):
        calls = {"count": 0}

        async def infra_fn(_ctx):
            calls["count"] += 1
            return self._probe(
                False,
                {"failure_class": "infrastructure", "inconclusive": True},
            )

        result = asyncio.run(
            run_live_qa._run_case_with_retries(
                infra_fn,
                object(),
                attempts=2,
                is_retriable=run_live_qa._is_case_retriable,
            )
        )

        self.assertFalse(result.success)
        self.assertEqual(result.details["attempts"], 1)
        self.assertEqual(calls["count"], 1)

    def test_does_not_retry_provider_incident(self):
        calls = {"count": 0}

        async def incident_fn(_ctx):
            calls["count"] += 1
            return self._probe(
                False,
                {"failure_category": "model_unavailable", "error": "503"},
            )

        # _is_provider_incident must gate the retry.
        self.assertTrue(
            run_live_qa._is_provider_incident(
                self._probe(False, {"failure_category": "model_unavailable"})
            )
        )
        result = asyncio.run(
            run_live_qa._run_case_with_retries(
                incident_fn,
                object(),
                attempts=2,
                is_retriable=run_live_qa._is_case_retriable,
            )
        )

        self.assertFalse(result.success)
        self.assertEqual(result.details["attempts"], 1)
        self.assertEqual(calls["count"], 1)

    def test_returns_last_failure_after_exhausting_attempts(self):
        calls = {"count": 0}

        async def always_fails_fn(_ctx):
            calls["count"] += 1
            return self._probe(False, {"error": f"attempt {calls['count']}"})

        result = asyncio.run(
            run_live_qa._run_case_with_retries(
                always_fails_fn,
                object(),
                attempts=3,
                is_retriable=run_live_qa._is_case_retriable,
            )
        )

        self.assertFalse(result.success)
        self.assertEqual(result.details["attempts"], 3)
        self.assertEqual(calls["count"], 3)
        self.assertEqual(result.details["error"], "attempt 3")

    def test_default_attempts_constant_allows_one_retry(self):
        self.assertGreaterEqual(run_live_qa.LIVE_QA_CASE_ATTEMPTS, 2)


if __name__ == "__main__":
    unittest.main()
