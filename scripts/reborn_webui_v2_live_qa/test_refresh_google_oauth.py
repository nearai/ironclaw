import io
import os
from pathlib import Path
import sys
import tempfile
from typing import NoReturn
import unittest
import urllib.error
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import refresh_google_oauth  # noqa: E402


class FakeResponse:
    def __init__(self, body: str):
        self.body = body

    def __enter__(self):
        return io.StringIO(self.body)

    def __exit__(self, *_args):
        return False


class RefreshGoogleOauthTests(unittest.TestCase):
    def setUp(self):
        self.env = {
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id",
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET": "client-secret",
            "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "refresh-token",
        }

    def test_refresh_returns_fresh_access_token(self):
        env = {
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id",
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET": "client-secret",
            "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "refresh-token",
        }
        with patch.dict(os.environ, env, clear=True), patch.object(
            refresh_google_oauth.urllib.request,
            "urlopen",
            return_value=FakeResponse('{"access_token":"fresh-token"}'),
        ):
            token, status = refresh_google_oauth.refresh_access_token()

        self.assertEqual(token, "fresh-token")
        self.assertEqual(status, "healthy")

    def test_refresh_reports_invalid_grant_without_response_details(self):
        error = urllib.error.HTTPError(
            refresh_google_oauth.TOKEN_URL,
            400,
            "Bad Request",
            {},
            io.BytesIO(b'{"error":"invalid_grant","error_description":"secret detail"}'),
        )
        env = {
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id",
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET": "client-secret",
            "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "refresh-token",
        }
        with patch.dict(os.environ, env, clear=True), patch.object(
            refresh_google_oauth.urllib.request,
            "urlopen",
            side_effect=error,
        ):
            token, status = refresh_google_oauth.refresh_access_token()

        self.assertIsNone(token)
        self.assertEqual(status, "invalid_grant")

    def test_secret_file_takes_precedence_and_output_mode_is_private(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            secret_path = Path(tmpdir) / "refresh"
            output_path = Path(tmpdir) / "access"
            secret_path.write_text("file-refresh-token", encoding="utf-8")
            env = {
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID": "client-id",
                "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET": "client-secret",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN": "env-refresh-token",
                "AUTH_LIVE_GOOGLE_REFRESH_TOKEN_PATH": str(secret_path),
            }
            with patch.dict(os.environ, env, clear=True), patch.object(
                refresh_google_oauth.urllib.request,
                "urlopen",
                return_value=FakeResponse('{"access_token":"fresh-token"}'),
            ) as urlopen:
                token, _ = refresh_google_oauth.refresh_access_token()
                refresh_google_oauth._write_output(output_path, token or "")

            request = urlopen.call_args.args[0]
            self.assertIn(b"refresh_token=file-refresh-token", request.data)
            self.assertEqual(output_path.read_text(encoding="utf-8"), "fresh-token")
            self.assertEqual(output_path.stat().st_mode & 0o777, 0o600)

    def test_refresh_reports_sanitized_failure_statuses(self):
        scenarios = (
            (FakeResponse("not-json"), "invalid_json"),
            (FakeResponse("[]"), "invalid_json"),
            (FakeResponse("{}"), "missing_access_token"),
            (urllib.error.URLError("private network detail"), "network:URLError"),
        )
        for result, expected_status in scenarios:
            with self.subTest(status=expected_status), patch.dict(
                os.environ, self.env, clear=True
            ), patch.object(
                refresh_google_oauth.urllib.request,
                "urlopen",
                return_value=result if isinstance(result, FakeResponse) else None,
                side_effect=result if isinstance(result, BaseException) else None,
            ):
                token, status = refresh_google_oauth.refresh_access_token()
            self.assertIsNone(token)
            self.assertEqual(status, expected_status)

        with patch.dict(os.environ, {}, clear=True):
            token, status = refresh_google_oauth.refresh_access_token()
        self.assertIsNone(token)
        self.assertTrue(status.startswith("missing:"))
        self.assertNotIn("client-id", status)

    def test_unreadable_secret_path_becomes_missing_status(self):
        env = {
            **self.env,
            "AUTH_LIVE_GOOGLE_REFRESH_TOKEN_PATH": "/does/not/exist",
        }
        with patch.dict(os.environ, env, clear=True):
            token, status = refresh_google_oauth.refresh_access_token()
        self.assertIsNone(token)
        self.assertEqual(status, "missing:AUTH_LIVE_GOOGLE_REFRESH_TOKEN")

    def test_malformed_http_error_body_is_sanitized(self):
        error = urllib.error.HTTPError(
            refresh_google_oauth.TOKEN_URL,
            503,
            "secret response detail",
            {},
            io.BytesIO(b"not-json"),
        )
        with patch.dict(os.environ, self.env, clear=True), patch.object(
            refresh_google_oauth.urllib.request, "urlopen", side_effect=error
        ):
            token, status = refresh_google_oauth.refresh_access_token()
        self.assertIsNone(token)
        self.assertEqual(status, "http_503")

    def test_non_object_and_unsafe_http_errors_use_status_fallback(self):
        bodies = (
            b'[]',
            b'null',
            b'"invalid_grant"',
            b'{"error":"contains a space"}',
            b'{"error":"line\\nbreak"}',
            b'{"error":"' + (b"x" * 129) + b'"}',
        )
        for body in bodies:
            with self.subTest(body=body):
                error = urllib.error.HTTPError(
                    refresh_google_oauth.TOKEN_URL,
                    429,
                    "secret response detail",
                    {},
                    io.BytesIO(body),
                )
                with patch.dict(os.environ, self.env, clear=True), patch.object(
                    refresh_google_oauth.urllib.request, "urlopen", side_effect=error
                ):
                    token, status = refresh_google_oauth.refresh_access_token()
                self.assertIsNone(token)
                self.assertEqual(status, "http_429")

    def test_unreadable_http_error_body_is_sanitized(self):
        class UnreadableBody:
            def read(self) -> NoReturn:
                raise OSError

            def close(self) -> None:
                pass

        error = urllib.error.HTTPError(
            refresh_google_oauth.TOKEN_URL,
            502,
            "secret response detail",
            {},
            UnreadableBody(),
        )
        with patch.dict(os.environ, self.env, clear=True), patch.object(
            refresh_google_oauth.urllib.request, "urlopen", side_effect=error
        ):
            token, status = refresh_google_oauth.refresh_access_token()
        self.assertIsNone(token)
        self.assertEqual(status, "http_502")

    def test_main_writes_status_and_token_only_on_success(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = Path(tmpdir)
            output_path = root / "github-output"
            token_path = root / "access-token"
            argv = [
                "refresh_google_oauth.py",
                "--github-output",
                str(output_path),
                "--access-token-path",
                str(token_path),
            ]
            with patch.object(sys, "argv", argv), patch.object(
                refresh_google_oauth,
                "refresh_access_token",
                return_value=("fresh-token", "healthy"),
            ):
                self.assertEqual(refresh_google_oauth.main(), 0)
            self.assertEqual(output_path.read_text(encoding="utf-8"), "status=healthy\n")
            self.assertEqual(token_path.read_text(encoding="utf-8"), "fresh-token")
            self.assertEqual(token_path.stat().st_mode & 0o777, 0o600)

            token_path.unlink()
            with patch.object(sys, "argv", argv), patch.object(
                refresh_google_oauth,
                "refresh_access_token",
                return_value=(None, "invalid_grant"),
            ):
                self.assertEqual(refresh_google_oauth.main(), 1)
            self.assertEqual(
                output_path.read_text(encoding="utf-8"),
                "status=healthy\nstatus=invalid_grant\n",
            )
            self.assertFalse(token_path.exists())


if __name__ == "__main__":
    unittest.main()
