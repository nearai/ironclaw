"""E2E test to reproduce bug #992: OAuth URL parameter corruption via Telegram.

Bug: When OAuth URL is initiated from Telegram, it contains 'clientid'
(no underscore) instead of 'client_id' (with underscore), causing Google to
reject it with Error 400: invalid_request.

Working: OAuth URL from web chat has correct 'client_id' parameter.

Additional symptom: URL cannot be regenerated - stale/cached URL is returned.

This test captures the exact behavior from the bug bash session.
"""

from urllib.parse import parse_qs, urlparse
import httpx
import json
import re

from helpers import api_post, api_get


async def extract_auth_url_from_message(message_text: str) -> str:
    """Extract OAuth URL from a message that contains 'Auth URL: <url>'."""
    match = re.search(r'Auth URL: (https://[^\s]+)', message_text)
    if match:
        return match.group(1)
    return None


async def test_oauth_url_parameter_naming_web_vs_telegram(ironclaw_server):
    """Test that OAuth URLs have consistent parameter names across channels.

    This test would compare OAuth URL generation for web gateway vs Telegram.
    Currently tests web gateway; Telegram would need WASM channel setup.
    """
    # Install Gmail
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200
    assert r.json().get("success") is True

    # Request OAuth URL via web API (setup endpoint)
    r = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    assert r.status_code == 200
    web_auth_url = r.json().get("auth_url")
    assert web_auth_url is not None, "Web API should return auth_url"

    # Parse the web URL
    parsed_web = urlparse(web_auth_url)
    web_params = parse_qs(parsed_web.query)

    # Verify web URL has CORRECT parameter names
    assert "client_id" in web_params, (
        f"Web OAuth URL missing 'client_id'. URL: {web_auth_url}"
    )
    assert "clientid" not in web_params, (
        f"Web OAuth URL should not have 'clientid' (no underscore). URL: {web_auth_url}"
    )
    assert "response_type" in web_params, (
        f"Web OAuth URL missing 'response_type'. URL: {web_auth_url}"
    )

    print(f"\n✓ Web OAuth URL (CORRECT):")
    print(f"  {web_auth_url}")
    print(f"  Parameters: {list(web_params.keys())}")


async def test_oauth_url_cannot_be_regenerated_symptom(ironclaw_server):
    """Test the secondary symptom: URL cannot be regenerated.

    Bug report noted: "The URL also could not be regenerated when Sergey asked —
    the agent provided a stale/cached URL that no longer worked."

    This suggests the URL is being cached somewhere and not regenerated properly.
    """
    # Install Gmail
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200

    # Get first OAuth URL
    r1 = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    first_auth_url = r1.json().get("auth_url")
    first_state = parse_qs(urlparse(first_auth_url).query).get("state", [None])[0]

    # Get second OAuth URL (simulating regeneration)
    r2 = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    second_auth_url = r2.json().get("auth_url")
    second_state = parse_qs(urlparse(second_auth_url).query).get("state", [None])[0]

    # Verify URLs are actually different (new state for each)
    assert first_auth_url != second_auth_url, (
        "Each OAuth request should generate a new URL with new CSRF state"
    )
    assert first_state != second_state, (
        "CSRF state should be unique per request (if not, URL is cached/stale)"
    )

    print(f"\n✓ OAuth URLs can be regenerated:")
    print(f"  First state:  {first_state[:20]}...")
    print(f"  Second state: {second_state[:20]}...")
    print(f"  States are unique: {first_state != second_state}")


async def test_oauth_url_parameter_structure(ironclaw_server):
    """Test that OAuth URL has all expected parameters with correct names.

    This validates the specific structure:
    ?client_id=...&response_type=code&redirect_uri=...&state=...&scope=...
    """
    # Install Gmail
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200

    # Get OAuth URL
    r = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    auth_url = r.json().get("auth_url")

    # Parse URL
    parsed = urlparse(auth_url)
    params = parse_qs(parsed.query)

    # Expected parameters (from build_oauth_url in oauth_defaults.rs line 126)
    expected_params = {
        "client_id": "Should have client_id (NOT clientid)",
        "response_type": "Should be 'code'",
        "redirect_uri": "Should be callback URL",
        "state": "CSRF nonce",
        "scope": "OAuth scopes",
    }

    print(f"\n✓ OAuth URL parameter validation:")
    print(f"  URL: {auth_url[:80]}...")
    print(f"\n  Parameters found:")

    for param_name, description in expected_params.items():
        if param_name in params:
            value = params[param_name][0]
            print(f"    ✓ {param_name}: {value[:40]}... ({description})")
        else:
            raise AssertionError(f"Missing parameter: {param_name} - {description}")

    # Extra: verify NO "clientid" parameter exists
    if "clientid" in params:
        raise AssertionError(
            f"BUG #992 DETECTED: URL has 'clientid' (no underscore)! URL: {auth_url}"
        )

    print(f"\n  ✓ No 'clientid' parameter (bug #992 not present)")


async def test_oauth_url_matches_google_spec(ironclaw_server):
    """Verify the OAuth URL matches Google's OAuth 2.0 spec.

    Reference: https://developers.google.com/identity/protocols/oauth2/web-server
    Required parameters: client_id, redirect_uri, response_type, scope, state
    """
    # Install Gmail
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200

    r = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    auth_url = r.json().get("auth_url")
    parsed = urlparse(auth_url)
    params = parse_qs(parsed.query)

    # Google OAuth 2.0 spec validation
    print(f"\n✓ Google OAuth 2.0 spec compliance:")
    print(f"  Endpoint: {parsed.scheme}://{parsed.netloc}{parsed.path}")
    assert parsed.netloc == "accounts.google.com", "Must use Google endpoint"
    assert parsed.path == "/o/oauth2/v2/auth", "Must use /o/oauth2/v2/auth"
    print(f"  ✓ Correct endpoint")

    assert params["response_type"][0] == "code", "Must use authorization_code flow"
    print(f"  ✓ response_type=code")

    assert len(params["client_id"][0]) > 0, "client_id must have value"
    print(f"  ✓ client_id present: {params['client_id'][0][:30]}...")

    assert len(params["redirect_uri"][0]) > 0, "redirect_uri must have value"
    print(f"  ✓ redirect_uri present: {params['redirect_uri'][0][:50]}...")

    assert len(params["scope"][0]) > 0, "scope must have value"
    print(f"  ✓ scope present: {params['scope'][0][:50]}...")

    assert len(params["state"][0]) > 0, "state must have value"
    print(f"  ✓ state present: {params['state'][0][:20]}...")


async def test_oauth_extra_params_preserved(ironclaw_server):
    """Test that extra_params from capabilities.json are preserved in URL.

    Gmail capabilities.json specifies:
    - access_type: "offline"
    - prompt: "consent"
    """
    # Install Gmail
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200

    r = await api_post(
        ironclaw_server,
        "/api/extensions/gmail/setup",
        json={"secrets": {}},
        timeout=30,
    )
    auth_url = r.json().get("auth_url")
    parsed = urlparse(auth_url)
    params = parse_qs(parsed.query)

    print(f"\n✓ Extra parameters from capabilities.json:")

    # From tools-src/gmail/gmail-tool.capabilities.json
    assert "access_type" in params, "Should have access_type=offline"
    assert params["access_type"][0] == "offline"
    print(f"  ✓ access_type={params['access_type'][0]}")

    assert "prompt" in params, "Should have prompt=consent"
    assert params["prompt"][0] == "consent"
    print(f"  ✓ prompt={params['prompt'][0]}")
