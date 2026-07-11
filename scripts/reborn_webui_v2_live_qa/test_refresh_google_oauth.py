import io
import os
from pathlib import Path
import sys
import tempfile
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


if __name__ == "__main__":
    unittest.main()
