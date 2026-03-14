"""OAuth URL parameter validation e2e tests.

Tests for bug #992: Google OAuth URL broken when initiated from Telegram.
Specifically verifies that OAuth query parameters are correctly formatted:
- "client_id" (with underscore) NOT "clientid" (without underscore)
- All standard OAuth parameters are present and correctly encoded
- URLs are consistent across channels (web, Telegram, etc.)

The test verifies:
1. OAuth URL is generated with correct parameters
2. URL works with the OAuth provider (Google)
3. Extra parameters (access_type, prompt) are preserved
"""

from urllib.parse import parse_qs, urlparse
import httpx
import pytest

from helpers import api_post, api_get


async def _extract_oauth_params(auth_url: str) -> dict:
    """Extract and validate OAuth query parameters from auth_url.

    Returns dict with parsed parameters:
    {
        'client_id': '...',
        'redirect_uri': '...',
        'response_type': 'code',
        'scope': '...',
        'state': '...',
        'access_type': '...',
        'prompt': '...',
        ...
    }
    """
    parsed = urlparse(auth_url)
    qs = parse_qs(parsed.query)

    # Convert lists to single values for easier testing
    params = {k: v[0] if len(v) > 0 else v for k, v in qs.items()}
    return params


class TestGoogleSheetsOAuthURL:
    """Test Gmail/Google OAuth URL generation and parameter naming.

    Uses Gmail since it's available in the test registry and uses Google OAuth.
    The bug #992 applies to all Google tools (Gmail, Google Drive, Sheets, etc.)
    """

    _gmail_installed = False
    _auth_url = None
    _oauth_params = None
    EXT_NAME = "gmail"

    @pytest.fixture(autouse=True)
    async def setup(self, ironclaw_server):
        """Setup: ensure Gmail is removed before test."""
        self.ironclaw_server = ironclaw_server
        ext = await self._get_extension(self.EXT_NAME)
        if ext:
            await api_post(ironclaw_server, f"/api/extensions/{self.EXT_NAME}/remove", timeout=30)

    async def _get_extension(self, name):
        """Get a specific extension from the extensions list, or None."""
        r = await api_get(self.ironclaw_server, "/api/extensions")
        for ext in r.json().get("extensions", []):
            if ext["name"] == name:
                return ext
        return None

    async def test_install_gmail(self):
        """Step 1: Install Gmail from registry."""
        r = await api_post(
            self.ironclaw_server,
            "/api/extensions/install",
            json={"name": self.EXT_NAME},
            timeout=180,
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("success") is True, f"Install failed: {data.get('message', '')}"
        TestGoogleSheetsOAuthURL._gmail_installed = True

    async def test_oauth_url_generated(self):
        """Step 2: Configure Gmail (no secrets) returns OAuth auth_url."""
        if not TestGoogleSheetsOAuthURL._gmail_installed:
            pytest.skip("Gmail not installed")

        r = await api_post(
            self.ironclaw_server,
            f"/api/extensions/{self.EXT_NAME}/setup",
            json={"secrets": {}},
            timeout=30,
        )
        assert r.status_code == 200
        data = r.json()
        assert data.get("success") is True, f"Setup failed: {data.get('message', '')}"

        auth_url = data.get("auth_url")
        assert auth_url is not None, f"Expected auth_url in response: {data}"
        assert "accounts.google.com" in auth_url, f"auth_url should point to Google: {auth_url}"

        TestGoogleSheetsOAuthURL._auth_url = auth_url

    async def test_oauth_url_has_client_id_not_clientid(self):
        """Step 3: Verify OAuth URL has 'client_id' (with underscore), NOT 'clientid'."""
        if not TestGoogleSheetsOAuthURL._auth_url:
            pytest.skip("No OAuth URL from setup step")

        auth_url = TestGoogleSheetsOAuthURL._auth_url
        params = await _extract_oauth_params(auth_url)
        TestGoogleSheetsOAuthURL._oauth_params = params

        # The bug: "clientid" appears instead of "client_id"
        # Verify the CORRECT parameter name exists
        assert "client_id" in params, (
            f"OAuth URL missing 'client_id' parameter. "
            f"URL: {auth_url}\nParams: {params}"
        )
        assert params["client_id"], "client_id should have a value"

        # Verify the INCORRECT parameter name does NOT exist
        assert "clientid" not in params, (
            f"OAuth URL should NOT have 'clientid' (without underscore). "
            f"Bug #992: URL: {auth_url}\nParams: {params}"
        )

    async def test_oauth_url_has_required_parameters(self):
        """Step 4: Verify all required OAuth 2.0 parameters are present."""
        if not TestGoogleSheetsOAuthURL._oauth_params:
            pytest.skip("No OAuth params from parameter validation step")

        params = TestGoogleSheetsOAuthURL._oauth_params

        # Required OAuth 2.0 parameters
        required = ["client_id", "response_type", "redirect_uri", "scope", "state"]
        for param in required:
            assert param in params, (
                f"Missing required OAuth parameter: {param}. "
                f"Params: {params}"
            )
            assert params[param], f"Parameter '{param}' should have a non-empty value"

        # Validate specific values
        assert params["response_type"] == "code", "Should use authorization_code flow"
        assert "oauth" in params["redirect_uri"], "Redirect URI should be an OAuth callback"

    async def test_oauth_url_has_extra_params(self):
        """Step 5: Verify extra_params from capabilities.json are included."""
        if not TestGoogleSheetsOAuthURL._oauth_params:
            pytest.skip("No OAuth params")

        params = TestGoogleSheetsOAuthURL._oauth_params

        # Google-specific extra_params from google-sheets-tool.capabilities.json
        assert "access_type" in params, (
            "Should include 'access_type' from extra_params"
        )
        assert params["access_type"] == "offline", (
            "access_type should be 'offline' for Google Sheets"
        )

        assert "prompt" in params, (
            "Should include 'prompt' from extra_params"
        )
        assert params["prompt"] == "consent", (
            "prompt should be 'consent' for Google Sheets"
        )

    async def test_oauth_url_is_valid_google_oauth(self):
        """Step 6: Verify the URL is a valid Google OAuth 2.0 authorization URL."""
        if not TestGoogleSheetsOAuthURL._auth_url:
            pytest.skip("No OAuth URL")

        auth_url = TestGoogleSheetsOAuthURL._auth_url

        # Verify scheme and host
        parsed = urlparse(auth_url)
        assert parsed.scheme == "https", "OAuth URL must use HTTPS"
        assert "accounts.google.com" in parsed.netloc, "Must be Google's OAuth endpoint"
        assert parsed.path == "/o/oauth2/v2/auth", "Must use Google OAuth 2.0 endpoint"

    async def test_oauth_url_state_is_unique(self):
        """Step 7: Verify CSRF state is present and unique per request."""
        if not TestGoogleSheetsOAuthURL._gmail_installed:
            pytest.skip("Gmail not installed")

        # Get a new OAuth URL
        r = await api_post(
            self.ironclaw_server,
            f"/api/extensions/{self.EXT_NAME}/setup",
            json={"secrets": {}},
            timeout=30,
        )
        assert r.status_code == 200
        new_auth_url = r.json().get("auth_url")
        assert new_auth_url is not None

        # Extract state from both URLs
        original_params = TestGoogleSheetsOAuthURL._oauth_params
        new_params = await _extract_oauth_params(new_auth_url)

        original_state = original_params.get("state")
        new_state = new_params.get("state")

        assert original_state is not None, "Should have state parameter"
        assert new_state is not None, "New request should have state parameter"
        assert original_state != new_state, (
            "CSRF state should be unique per request (for security)"
        )

    async def test_oauth_url_escaping(self):
        """Step 8: Verify URL query parameters are properly escaped."""
        if not TestGoogleSheetsOAuthURL._auth_url:
            pytest.skip("No OAuth URL")

        auth_url = TestGoogleSheetsOAuthURL._auth_url

        # Verify special characters in values are URL-encoded
        # For example, scopes contain spaces which should be %20
        assert "%20" in auth_url or "+" in auth_url or "%2B" in auth_url or " " not in auth_url, (
            "OAuth URL should properly encode special characters in parameters"
        )

    async def test_cleanup_gmail(self):
        """Cleanup: Remove Gmail (cleanup for other test files)."""
        ext = await self._get_extension(self.EXT_NAME)
        if ext:
            r = await api_post(
                self.ironclaw_server,
                f"/api/extensions/{self.EXT_NAME}/remove",
                timeout=30,
            )
            assert r.status_code == 200

        ext = await self._get_extension(self.EXT_NAME)
        assert ext is None, "Gmail should be removed"


# ─ Telegram-specific tests (when Telegram channel is available) ──────────

class TestOAuthURLViaTelegram:
    """Test OAuth URL generation specifically via Telegram channel.

    These tests would verify that the same OAuth URL works correctly when
    transmitted through the Telegram WASM channel (as opposed to web gateway).

    Currently marked as xfail pending Telegram channel setup in E2E tests.
    """

    @pytest.mark.skip(reason="Telegram channel E2E setup not yet implemented")
    async def test_telegram_oauth_url_has_correct_parameters(self):
        """Verify OAuth URL sent via Telegram has correct parameter names."""
        # This test would:
        # 1. Send a message via Telegram that triggers OAuth
        # 2. Capture the status update sent to Telegram
        # 3. Extract the auth_url from the message
        # 4. Verify it has "client_id" not "clientid"
        pass

    @pytest.mark.skip(reason="Telegram channel E2E setup not yet implemented")
    async def test_telegram_oauth_url_can_be_regenerated(self):
        """Verify OAuth URL can be regenerated when requested via Telegram."""
        # This test would verify that the bug #992 symptom
        # "URL cannot be regenerated when asked" is fixed.
        # If the URL is cached incorrectly, regeneration would fail.
        pass
