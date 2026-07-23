from __future__ import annotations

import unittest
from collections import deque
import inspect
import os
from typing import Any
from unittest.mock import patch

from scripts.live_canary import auth_runtime
from scripts.live_canary import auth_registry
from scripts.live_canary import common


class FakeResponse:
    def __init__(self, status_code: int, body: dict[str, Any]) -> None:
        self.status_code = status_code
        self._body = body
        self.text = str(body)

    def json(self) -> dict[str, Any]:
        return self._body

    def raise_for_status(self) -> None:
        if not 200 <= self.status_code < 300:
            raise RuntimeError(f"HTTP {self.status_code}: {self.text}")


class AuthRuntimeContractTests(unittest.IsolatedAsyncioTestCase):
    async def test_install_uses_v2_package_ref_and_reads_derived_lifecycle(self) -> None:
        calls: list[tuple[str, str, dict[str, Any] | None]] = []
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "success": True,
                        "message": "Extension installed.",
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "extensions": [
                            {
                                "package_ref": {"kind": "extension", "id": "gmail"},
                                "installation_state": "setup_needed",
                                "tools": [],
                            }
                        ]
                    },
                ),
            ]
        )

        async def fake_request(
            method: str,
            base_url: str,
            path: str,
            *,
            token: str,
            json_body: Any | None = None,
            timeout: float = 30,
        ) -> FakeResponse:
            del base_url, token, timeout
            calls.append((method, path, json_body))
            return responses.popleft()

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            extension = await auth_runtime.install_extension(
                "http://ironclaw.test",
                "token",
                package_id="gmail",
                idempotency_key="auth-live-canary-install-gmail",
            )

        self.assertEqual(extension["installation_state"], "setup_needed")
        self.assertEqual(calls[0][0:2], ("POST", "/api/webchat/v2/extensions/install"))
        self.assertEqual(
            calls[0][2]["package_ref"],
            {"kind": "extension", "id": "gmail"},
        )
        self.assertEqual(
            calls[0][2]["idempotency_key"],
            "auth-live-canary-install-gmail",
        )
        self.assertEqual(calls[1], ("GET", "/api/webchat/v2/extensions", None))

    async def test_wait_for_lifecycle_requires_derived_active_and_tools(self) -> None:
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "extensions": [
                            {
                                "package_ref": {"kind": "extension", "id": "github"},
                                "installation_state": "setup_needed",
                                "tools": [],
                                "active": True,
                                "authenticated": True,
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "extensions": [
                            {
                                "package_ref": {"kind": "extension", "id": "github"},
                                "installation_state": "active",
                                "tools": ["github.get_repo"],
                                "active": False,
                                "authenticated": False,
                            }
                        ]
                    },
                ),
            ]
        )

        async def fake_request(*args: Any, **kwargs: Any) -> FakeResponse:
            del args, kwargs
            return responses.popleft()

        async def no_sleep() -> None:
            return None

        with (
            patch.object(auth_runtime, "api_request", side_effect=fake_request),
            patch.object(auth_runtime, "_sleep", side_effect=no_sleep),
        ):
            extension = await auth_runtime.wait_for_extension_lifecycle(
                "http://ironclaw.test",
                "token",
                "github",
                state="active",
                required_tools=("github.get_repo",),
            )

        self.assertEqual(extension["installation_state"], "active")

    async def test_manual_setup_uses_manifest_declared_requirement_name(self) -> None:
        calls: list[tuple[str, str, dict[str, Any] | None]] = []
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "package_ref": {"kind": "extension", "id": "github"},
                        "phase": "setup_needed",
                        "secrets": [
                            {
                                "name": "github_runtime_token",
                                "provider": "github",
                                "provided": False,
                                "setup": {"kind": "manual_token"},
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "package_ref": {"kind": "extension", "id": "github"},
                        "phase": "active",
                        "secrets": [
                            {
                                "name": "github_runtime_token",
                                "provider": "github",
                                "provided": True,
                                "setup": {"kind": "manual_token"},
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "extensions": [
                            {
                                "package_ref": {"kind": "extension", "id": "github"},
                                "installation_state": "active",
                                "tools": ["github.get_repo"],
                            }
                        ]
                    },
                ),
            ]
        )

        async def fake_request(
            method: str,
            base_url: str,
            path: str,
            *,
            token: str,
            json_body: Any | None = None,
            timeout: float = 30,
        ) -> FakeResponse:
            del base_url, token, timeout
            calls.append((method, path, json_body))
            return responses.popleft()

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            extension = await auth_runtime.complete_manual_token_setup(
                "http://ironclaw.test",
                "token",
                package_id="github",
                value="test-token",
                required_tools=("github.get_repo",),
            )

        self.assertEqual(extension["installation_state"], "active")
        self.assertEqual(
            calls[1],
            (
                "POST",
                "/api/webchat/v2/extensions/github/setup",
                {
                    "action": "submit",
                    "payload": {
                        "secrets": {"github_runtime_token": "test-token"},
                        "fields": {},
                    },
                },
            ),
        )

    async def test_oauth_setup_uses_descriptor_provider_and_flow_status(self) -> None:
        calls: list[tuple[str, str, dict[str, Any] | None]] = []
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "package_ref": {"kind": "extension", "id": "gmail"},
                        "phase": "setup_needed",
                        "secrets": [
                            {
                                "name": "google_credential",
                                "provider": "google",
                                "provided": False,
                                "setup": {
                                    "kind": "oauth",
                                    "invocation_id": "invocation-from-manifest",
                                },
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "authorization_url": (
                            "https://accounts.example/authorize?state=opaque-state"
                        ),
                        "flow_id": "flow-123",
                        "status": "awaiting_user",
                        "provider": "google",
                        "callback_scope": {
                            "invocation_id": "invocation-from-manifest"
                        },
                    },
                ),
                FakeResponse(
                    200,
                    {"flow_id": "flow-123", "status": "completed"},
                ),
                FakeResponse(200, {"status": "completed"}),
                FakeResponse(
                    200,
                    {
                        "accounts": [
                            {
                                "id": "account-123",
                                "provider": "google",
                                "status": "configured",
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "id": "account-123",
                        "provider": "google",
                        "status": "configured",
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "extensions": [
                            {
                                "package_ref": {"kind": "extension", "id": "gmail"},
                                "installation_state": "active",
                                "tools": ["gmail.list_messages"],
                            }
                        ]
                    },
                ),
            ]
        )

        async def fake_request(
            method: str,
            base_url: str,
            path: str,
            *,
            token: str,
            json_body: Any | None = None,
            timeout: float = 30,
        ) -> FakeResponse:
            del base_url, token, timeout
            calls.append((method, path, json_body))
            return responses.popleft()

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            extension = await auth_runtime.complete_oauth_flow(
                "http://ironclaw.test",
                "token",
                package_id="gmail",
                code="mock-code",
                callback_params={"scope": "scope-a scope-b"},
                required_tools=("gmail.list_messages",),
            )

        self.assertEqual(extension["installation_state"], "active")
        self.assertEqual(
            calls[1],
            (
                "POST",
                "/api/webchat/v2/extensions/gmail/setup/oauth/start",
                {
                    "requirement": "google_credential",
                    "expires_at": unittest.mock.ANY,
                    "invocation_id": "invocation-from-manifest",
                },
            ),
        )
        self.assertTrue(
            calls[2][1].startswith(
                "/api/reborn/product-auth/oauth/google/callback?"
            )
        )
        self.assertEqual(
            calls[3][1],
            (
                "/api/reborn/product-auth/oauth/flow/flow-123/status"
                "?invocation_id=invocation-from-manifest"
            ),
        )
        self.assertEqual(
            calls[4],
            (
                "POST",
                "/api/reborn/product-auth/accounts/list",
                {
                    "provider": "google",
                    "requester_extension": "gmail",
                    "invocation_id": "invocation-from-manifest",
                },
            ),
        )
        self.assertEqual(
            calls[5],
            (
                "POST",
                "/api/reborn/product-auth/accounts/select",
                {
                    "provider": "google",
                    "requester_extension": "gmail",
                    "account_id": "account-123",
                    "invocation_id": "invocation-from-manifest",
                },
            ),
        )

    async def test_admin_configuration_uses_group_revision_and_declared_handles(self) -> None:
        calls: list[tuple[str, str, dict[str, Any] | None]] = []
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "groups": [
                            {
                                "group_id": "vendor.google",
                                "revision": 7,
                                "fields": [
                                    {"handle": "google_oauth_client_id"},
                                    {"handle": "google_oauth_client_secret"},
                                ],
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "group_id": "vendor.google",
                        "revision": 8,
                        "complete": True,
                        "fields": [
                            {
                                "handle": "google_oauth_client_id",
                                "provided": True,
                            },
                            {
                                "handle": "google_oauth_client_secret",
                                "provided": True,
                            },
                        ],
                    },
                ),
            ]
        )

        async def fake_request(
            method: str,
            base_url: str,
            path: str,
            *,
            token: str,
            json_body: Any | None = None,
            timeout: float = 30,
        ) -> FakeResponse:
            del base_url, token, timeout
            calls.append((method, path, json_body))
            return responses.popleft()

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            await auth_runtime.configure_admin_group(
                "http://ironclaw.test",
                "token",
                group_id="vendor.google",
                values={
                    "google_oauth_client_id": "client-id",
                    "google_oauth_client_secret": "client-secret",
                },
                idempotency_key="canary-setup",
            )

        self.assertEqual(
            calls[1],
            (
                "PUT",
                "/api/webchat/v2/operator/extension-configuration/vendor.google",
                {
                    "values": [
                        {"handle": "google_oauth_client_id", "value": "client-id"},
                        {
                            "handle": "google_oauth_client_secret",
                            "value": "client-secret",
                        },
                    ],
                    "expected_revision": 7,
                    "idempotency_key": "canary-setup",
                },
            ),
        )

    async def test_canceled_oauth_flow_is_terminal(self) -> None:
        async def fake_request(*args: Any, **kwargs: Any) -> FakeResponse:
            del args, kwargs
            return FakeResponse(200, {"status": "canceled"})

        async def no_sleep() -> None:
            return None

        with (
            patch.object(auth_runtime, "api_request", side_effect=fake_request),
            patch.object(auth_runtime, "_sleep", side_effect=no_sleep),
        ):
            with self.assertRaisesRegex(
                common.CanaryError,
                "terminal status 'canceled'",
            ):
                await auth_runtime.wait_for_oauth_flow_completed(
                    "http://ironclaw.test",
                    "token",
                    package_id="gmail",
                    flow_id="flow-123",
                    invocation_id="invocation-123",
                    timeout=0.01,
                )

    async def test_install_rejects_unsuccessful_action_projection(self) -> None:
        async def fake_request(*args: Any, **kwargs: Any) -> FakeResponse:
            del args, kwargs
            return FakeResponse(
                200,
                {"success": False, "message": "Install did not complete."},
            )

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            with self.assertRaisesRegex(
                common.CanaryError,
                "Install action did not succeed",
            ):
                await auth_runtime.install_extension(
                    "http://ironclaw.test",
                    "token",
                    package_id="gmail",
                    idempotency_key="auth-live-canary-install-gmail",
                )

    async def test_setup_rejects_invalid_phase_projection(self) -> None:
        async def fake_request(*args: Any, **kwargs: Any) -> FakeResponse:
            del args, kwargs
            return FakeResponse(
                200,
                {
                    "package_ref": {"kind": "extension", "id": "gmail"},
                    "phase": "installed",
                    "secrets": [],
                },
            )

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            with self.assertRaisesRegex(common.CanaryError, "invalid phase"):
                await auth_runtime.get_extension_setup(
                    "http://ironclaw.test",
                    "token",
                    package_id="gmail",
                )

    async def test_oauth_start_rejects_provider_mismatch(self) -> None:
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "package_ref": {"kind": "extension", "id": "gmail"},
                        "phase": "setup_needed",
                        "secrets": [
                            {
                                "name": "google_credential",
                                "provider": "google",
                                "provided": False,
                                "setup": {
                                    "kind": "oauth",
                                    "invocation_id": "invocation-from-manifest",
                                },
                            }
                        ],
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "authorization_url": (
                            "https://accounts.example/authorize?state=opaque-state"
                        ),
                        "flow_id": "flow-123",
                        "status": "awaiting_user",
                        "provider": "github",
                        "callback_scope": {
                            "invocation_id": "invocation-from-manifest"
                        },
                    },
                ),
            ]
        )

        async def fake_request(*args: Any, **kwargs: Any) -> FakeResponse:
            del args, kwargs
            return responses.popleft()

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            with self.assertRaisesRegex(common.CanaryError, "expected 'google'"):
                await auth_runtime.start_oauth_setup(
                    "http://ironclaw.test",
                    "token",
                    package_id="gmail",
                )

    async def test_account_selection_rejects_wrong_projection(self) -> None:
        responses = deque(
            [
                FakeResponse(
                    200,
                    {
                        "accounts": [
                            {
                                "id": "account-123",
                                "provider": "google",
                                "status": "configured",
                            }
                        ]
                    },
                ),
                FakeResponse(
                    200,
                    {
                        "id": "account-other",
                        "provider": "google",
                        "status": "configured",
                    },
                ),
            ]
        )

        async def fake_request(*args: Any, **kwargs: Any) -> FakeResponse:
            del args, kwargs
            return responses.popleft()

        with patch.object(auth_runtime, "api_request", side_effect=fake_request):
            with self.assertRaisesRegex(
                common.CanaryError,
                "unexpected projection",
            ):
                await auth_runtime.select_single_oauth_account(
                    "http://ironclaw.test",
                    "token",
                    package_id="gmail",
                    provider="google",
                    invocation_id="invocation-123",
                )


class AuthCanarySourceContractTests(unittest.TestCase):
    def test_auth_canary_does_not_call_retired_activation_surface(self) -> None:
        root = auth_runtime.ROOT
        sources = [
            root / "scripts" / "live_canary" / "auth_runtime.py",
            root / "scripts" / "auth_live_canary" / "run_live_canary.py",
        ]
        combined = "\n".join(path.read_text(encoding="utf-8") for path in sources)
        self.assertNotIn("/api/extensions", combined)
        self.assertNotIn("activate_extension", combined)
        self.assertNotIn("/activate", combined)
        self.assertNotIn("authenticated=True", combined)
        self.assertNotIn("active=True", combined)

    def test_auth_canary_builds_and_starts_shipping_reborn_binary(self) -> None:
        build_source = inspect.getsource(common.cargo_build_reborn)
        start_source = inspect.getsource(common.start_reborn_gateway_stack)
        self.assertIn('"ironclaw"', build_source)
        self.assertNotIn("ironclaw_legacy", build_source)
        self.assertIn('"debug" / "ironclaw"', start_source)
        self.assertNotIn("ironclaw-legacy", start_source)

    def test_seeded_calendar_uses_current_package_id(self) -> None:
        self.assertEqual(
            auth_registry.SEEDED_CASES["google_calendar"].extension_install_name,
            "google-calendar",
        )

    def test_seeded_notion_is_rejected_in_favor_of_declared_browser_oauth(self) -> None:
        with patch.dict(
            os.environ,
            {"AUTH_LIVE_NOTION_ACCESS_TOKEN": "obsolete-seeded-token"},
            clear=False,
        ):
            with self.assertRaisesRegex(
                common.CanaryError,
                "browser --case notion",
            ):
                auth_registry.configured_seeded_cases(["notion"])


if __name__ == "__main__":
    unittest.main()
