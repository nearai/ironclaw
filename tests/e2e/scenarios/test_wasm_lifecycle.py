"""Comprehensive WASM extension lifecycle e2e tests.

Tests the full extension pipeline: registry → install → fields → configure →
activate → tools → remove → reinstall. Validates response fields, not just
status codes, to catch production bugs like missing capabilities, wrong
activation state, and stale registry flags.

Tests are ordered with dependencies tracked via module-level state.
"""

from pathlib import Path

import httpx
import pytest

from helpers import AUTH_TOKEN, SEL, api_get, api_post

# Module-level state for dependent tests
_ws_installed = False
_ws_configured = False
_gmail_installed = False


async def _get_extension(base_url, name):
    """Get a specific extension from the extensions list, or None."""
    r = await api_get(base_url, "/api/extensions")
    for ext in r.json().get("extensions", []):
        if ext["name"] == name:
            return ext
    return None


async def _ensure_removed(base_url, name):
    """Remove extension if already installed (idempotent cleanup)."""
    ext = await _get_extension(base_url, name)
    if ext:
        await api_post(base_url, f"/api/extensions/{name}/remove", timeout=30)


# ── Section A: Registry Validation ──────────────────────────────────────


async def test_registry_lists_extensions(ironclaw_server):
    """Registry endpoint returns entries from the embedded catalog."""
    r = await api_get(ironclaw_server, "/api/extensions/registry")
    assert r.status_code == 200
    data = r.json()
    assert "entries" in data
    names = [e["name"] for e in data["entries"]]
    assert "web-search" in names
    assert "gmail" in names


async def test_registry_entry_fields(ironclaw_server):
    """Every registry entry has all required fields with correct types."""
    r = await api_get(ironclaw_server, "/api/extensions/registry")
    entries = r.json()["entries"]
    assert len(entries) > 0, "Registry should have entries"
    for entry in entries:
        assert "name" in entry and isinstance(entry["name"], str) and entry["name"]
        assert "display_name" in entry and isinstance(entry["display_name"], str)
        assert "kind" in entry and isinstance(entry["kind"], str)
        assert "description" in entry and isinstance(entry["description"], str)
        assert "installed" in entry and isinstance(entry["installed"], bool)
        assert "keywords" in entry and isinstance(entry["keywords"], list)


async def test_registry_installed_flag_false_initially(ironclaw_server):
    """Before any install, all registry entries have installed=False."""
    # Clean up in case previous test run left extensions installed
    await _ensure_removed(ironclaw_server, "web-search")
    await _ensure_removed(ironclaw_server, "gmail")

    r = await api_get(ironclaw_server, "/api/extensions/registry")
    entries = r.json()["entries"]
    for entry in entries:
        if entry["name"] in ("web-search", "gmail"):
            assert entry["installed"] is False, (
                f"{entry['name']} should not be installed yet"
            )


async def test_registry_search_filters(ironclaw_server):
    """Search query filters registry results."""
    r = await api_get(
        ironclaw_server, "/api/extensions/registry", params={"query": "search"}
    )
    assert r.status_code == 200
    entries = r.json()["entries"]
    names = [e["name"] for e in entries]
    assert "web-search" in names


async def test_registry_search_no_match(ironclaw_server):
    """Nonsense query returns empty results."""
    r = await api_get(
        ironclaw_server,
        "/api/extensions/registry",
        params={"query": "xyznonexistent999"},
    )
    assert r.status_code == 200
    assert len(r.json()["entries"]) == 0


# ── Section B: Install Lifecycle (web-search) ───────────────────────────


async def test_install_web_search(ironclaw_server):
    """Install web-search from registry. Asserts success — failure here means
    the registry/download/build pipeline is broken."""
    global _ws_installed
    # Clean slate
    await _ensure_removed(ironclaw_server, "web-search")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "web-search"},
        timeout=180,
    )
    assert r.status_code == 200, f"Install HTTP error: {r.status_code} {r.text[:300]}"
    data = r.json()
    assert data.get("success") is True, f"Install failed: {data.get('message', '')}"
    assert "message" in data
    _ws_installed = True


async def test_installed_extension_fields(ironclaw_server):
    """After install, extension list shows correct fields."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    ext = await _get_extension(ironclaw_server, "web-search")
    assert ext is not None, "web-search not in extensions list after install"
    assert ext["kind"] == "wasm_tool"
    assert ext["needs_setup"] is True, "Should need setup (has brave_api_key secret)"
    assert ext["authenticated"] is False, "Should not be authenticated before configure"


async def test_installed_in_registry(ironclaw_server):
    """Registry marks installed extension with installed=True."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_get(ironclaw_server, "/api/extensions/registry")
    entries = r.json()["entries"]
    ws_entry = next((e for e in entries if e["name"] == "web-search"), None)
    assert ws_entry is not None
    assert ws_entry["installed"] is True, "Registry should show installed=True"


async def test_setup_schema_has_secrets(ironclaw_server):
    """Setup schema returns brave_api_key with correct field info."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_get(ironclaw_server, "/api/extensions/web-search/setup")
    assert r.status_code == 200
    data = r.json()
    assert "secrets" in data
    secrets = {s["name"]: s for s in data["secrets"]}
    assert "brave_api_key" in secrets, (
        f"brave_api_key not in setup schema secrets: {list(secrets.keys())}"
    )
    key_info = secrets["brave_api_key"]
    assert key_info["provided"] is False, "Should not be provided yet"


async def test_extension_not_authenticated_before_configure(ironclaw_server):
    """Installed but not configured extension is not authenticated."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    ext = await _get_extension(ironclaw_server, "web-search")
    assert ext is not None
    # Before configuring secrets, extension shouldn't be fully authenticated
    assert ext["needs_setup"] is True, "Should still need setup before configure"


async def test_activate_before_configure_rejected(ironclaw_server):
    """Activating a tool that needs setup secrets is rejected."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_post(
        ironclaw_server, "/api/extensions/web-search/activate", timeout=30
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is False, (
        f"Activate should fail before configure: {data}"
    )
    msg = data.get("message", "").lower()
    assert "requires configuration" in msg or "setup" in msg, (
        f"Error should mention configuration: {data.get('message')}"
    )


# ── Section C: Configure + Activate (web-search) ────────────────────────


async def test_configure_rejects_unknown_secret(ironclaw_server):
    """Submitting an unknown secret name is rejected."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/web-search/setup",
        json={"secrets": {"fake_unknown_key": "value"}},
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is False, f"Should reject unknown secret: {data}"
    assert "unknown" in data.get("message", "").lower() or "not found" in data.get(
        "message", ""
    ).lower(), f"Error should mention unknown secret: {data.get('message')}"


async def test_configure_with_valid_secret(ironclaw_server):
    """Configure with valid brave_api_key succeeds and auto-activates."""
    global _ws_configured
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/web-search/setup",
        json={"secrets": {"brave_api_key": "test-key-123"}},
        timeout=30,
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, f"Configure failed: {data.get('message', '')}"
    assert data.get("activated") is True, "Should auto-activate after configure"
    _ws_configured = True


async def test_extension_active_after_configure(ironclaw_server):
    """After configure, extension shows authenticated=True and active=True."""
    if not _ws_configured:
        pytest.skip("web-search not configured")

    ext = await _get_extension(ironclaw_server, "web-search")
    assert ext is not None
    assert ext["authenticated"] is True, "Should be authenticated after configure"
    assert ext["active"] is True, "Should be active after auto-activation"
    assert len(ext.get("tools", [])) > 0, "Should have tools registered"


async def test_setup_shows_provided(ironclaw_server):
    """After configure, setup schema shows secret as provided."""
    if not _ws_configured:
        pytest.skip("web-search not configured")

    r = await api_get(ironclaw_server, "/api/extensions/web-search/setup")
    assert r.status_code == 200
    secrets = {s["name"]: s for s in r.json()["secrets"]}
    assert "brave_api_key" in secrets
    assert secrets["brave_api_key"]["provided"] is True


async def test_tools_registered_after_activate(ironclaw_server):
    """After activation, extension tools appear in the tools endpoint."""
    if not _ws_configured:
        pytest.skip("web-search not configured")

    r = await api_get(ironclaw_server, "/api/extensions/tools")
    assert r.status_code == 200
    tool_names = [t["name"] for t in r.json()["tools"]]
    assert "web-search" in tool_names, (
        f"web-search tool not found in tools list: {tool_names}"
    )


async def test_activate_already_active_idempotent(ironclaw_server):
    """Activating an already-active extension succeeds (idempotent)."""
    if not _ws_configured:
        pytest.skip("web-search not configured")

    r = await api_post(
        ironclaw_server, "/api/extensions/web-search/activate", timeout=30
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, (
        f"Re-activation should succeed: {data.get('message', '')}"
    )


async def test_configure_empty_secret_skipped(ironclaw_server):
    """Submitting an empty string for a secret skips it (doesn't overwrite)."""
    if not _ws_configured:
        pytest.skip("web-search not configured")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/web-search/setup",
        json={"secrets": {"brave_api_key": ""}},
        timeout=30,
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True

    # Verify the secret is still provided (not cleared)
    r2 = await api_get(ironclaw_server, "/api/extensions/web-search/setup")
    secrets = {s["name"]: s for s in r2.json()["secrets"]}
    assert secrets["brave_api_key"]["provided"] is True, (
        "Empty value should not clear existing secret"
    )


# ── Section D: Install gmail (multi-extension) ──────────────────────────


async def test_install_gmail(ironclaw_server):
    """Install gmail from registry (second extension, tests isolation)."""
    global _gmail_installed
    await _ensure_removed(ironclaw_server, "gmail")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "gmail"},
        timeout=180,
    )
    assert r.status_code == 200, f"Install HTTP error: {r.status_code} {r.text[:300]}"
    data = r.json()
    assert data.get("success") is True, f"Install failed: {data.get('message', '')}"
    _gmail_installed = True


async def test_gmail_fields(ironclaw_server):
    """Gmail extension has correct field values (OAuth-based auth)."""
    if not _gmail_installed:
        pytest.skip("gmail not installed")

    ext = await _get_extension(ironclaw_server, "gmail")
    assert ext is not None, "gmail not in extensions list"
    assert ext["kind"] == "wasm_tool"
    assert ext["has_auth"] is True, "Gmail should have OAuth auth"


async def test_both_extensions_listed(ironclaw_server):
    """Both web-search and gmail appear in extensions list (no clobbering)."""
    if not _ws_installed or not _gmail_installed:
        pytest.skip("Both extensions not installed")

    r = await api_get(ironclaw_server, "/api/extensions")
    names = [e["name"] for e in r.json()["extensions"]]
    assert "web-search" in names, f"web-search missing from: {names}"
    assert "gmail" in names, f"gmail missing from: {names}"


async def test_gmail_setup_schema_auto_resolves(ironclaw_server):
    """Gmail setup schema returns empty secrets (builtin creds auto-resolve)."""
    if not _gmail_installed:
        pytest.skip("gmail not installed")

    r = await api_get(ironclaw_server, "/api/extensions/gmail/setup")
    assert r.status_code == 200
    data = r.json()
    secrets = data.get("secrets", [])
    # Builtin Google credentials auto-resolve client_id/client_secret via
    # is_auto_resolved_oauth_field(), so the setup schema should have no
    # user-facing secrets (or only auto-generated ones).
    user_facing = [s for s in secrets if not s.get("auto_generate", False)]
    assert len(user_facing) == 0, (
        f"Gmail should have no user-facing secrets (auto-resolved), got: "
        f"{[s['name'] for s in user_facing]}"
    )


# ── Section E: Remove + Cleanup ─────────────────────────────────────────


async def test_remove_web_search(ironclaw_server):
    """Remove web-search succeeds."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_post(
        ironclaw_server, "/api/extensions/web-search/remove", timeout=30
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, (
        f"Remove failed: {data.get('message', '')}"
    )


async def test_removed_not_in_extensions(ironclaw_server):
    """Removed extension no longer appears in extensions list."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    ext = await _get_extension(ironclaw_server, "web-search")
    assert ext is None, "web-search should not be in extensions list after removal"


async def test_removed_extension_not_listed(ironclaw_server):
    """Removed extension should not appear in the extension tools list."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_get(ironclaw_server, "/api/extensions/tools")
    assert r.status_code == 200
    tool_names = [t["name"] for t in r.json()["tools"]]
    assert "web-search" not in tool_names, (
        f"Removed web-search tool should not remain registered: {tool_names}"
    )


async def test_removed_not_in_registry_installed(ironclaw_server):
    """Registry shows removed extension as installed=False."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    r = await api_get(ironclaw_server, "/api/extensions/registry")
    ws_entry = next(
        (e for e in r.json()["entries"] if e["name"] == "web-search"), None
    )
    assert ws_entry is not None
    assert ws_entry["installed"] is False, "Registry should show installed=False"


async def test_activate_after_remove_uses_replacement_bytes_not_cached_module(
    ironclaw_server, wasm_tools_dir
):
    """After removal, activation must use the replacement bytes rather than a stale cache."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    wasm_path = Path(wasm_tools_dir) / "web-search.wasm"
    wasm_path.write_bytes(b"not-a-valid-wasm-component")

    r = await api_post(
        ironclaw_server, "/api/extensions/web-search/activate", timeout=30
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is False, (
        f"Activation should fail against replacement bytes, got: {data}"
    )


async def test_reinstall_after_remove(ironclaw_server):
    """Extension can be reinstalled after removal without stale activation errors."""
    if not _ws_installed:
        pytest.skip("web-search not installed")

    await _ensure_removed(ironclaw_server, "web-search")

    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "web-search"},
        timeout=180,
    )
    assert r.status_code == 200
    data = r.json()
    assert data.get("success") is True, (
        f"Reinstall failed: {data.get('message', '')}"
    )

    ext = await _get_extension(ironclaw_server, "web-search")
    assert ext is not None, "web-search not found after reinstall"
    assert ext["active"] is True, "Reinstalled tool should auto-activate via saved secrets"
    assert ext["authenticated"] is True, "Saved secret should still authenticate on reinstall"
    # Verify no stale activation error from previous install
    assert ext.get("activation_error") is None or ext.get("activation_error") == "", (
        f"Reinstalled extension should have no stale activation error: {ext}"
    )


async def test_cleanup_all(ironclaw_server):
    """Clean up all installed extensions for subsequent test files."""
    await _ensure_removed(ironclaw_server, "web-search")
    await _ensure_removed(ironclaw_server, "gmail")


# ── Section F: Error Paths ──────────────────────────────────────────────


async def test_install_nonexistent(ironclaw_server):
    """Installing a nonexistent extension returns an error."""
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": "nonexistent-tool-xyz-999"},
        timeout=30,
    )
    if r.status_code == 200:
        assert r.json().get("success") is False
    else:
        assert r.status_code >= 400


async def test_install_empty_name(ironclaw_server):
    """Installing with empty name returns an error."""
    r = await api_post(
        ironclaw_server,
        "/api/extensions/install",
        json={"name": ""},
        timeout=10,
    )
    if r.status_code == 200:
        assert r.json().get("success") is False
    else:
        assert r.status_code >= 400


async def test_remove_noninstalled(ironclaw_server):
    """Removing a non-installed extension returns an error."""
    r = await api_post(
        ironclaw_server, "/api/extensions/nonexistent-xyz/remove", timeout=10
    )
    if r.status_code == 200:
        assert r.json().get("success") is False
    else:
        assert r.status_code >= 400


async def test_activate_noninstalled(ironclaw_server):
    """Activating a non-installed extension returns an error."""
    r = await api_post(
        ironclaw_server, "/api/extensions/nonexistent-xyz/activate", timeout=10
    )
    if r.status_code == 200:
        assert r.json().get("success") is False
    else:
        assert r.status_code >= 400


async def test_setup_noninstalled(ironclaw_server):
    """Setup for non-installed extension returns an error."""
    r = await api_get(ironclaw_server, "/api/extensions/nonexistent-xyz/setup")
    # May return 500 or a JSON error
    assert r.status_code >= 400 or r.json().get("success") is False


async def test_configure_noninstalled(ironclaw_server):
    """Configure for non-installed extension returns an error."""
    r = await api_post(
        ironclaw_server,
        "/api/extensions/nonexistent-xyz/setup",
        json={"secrets": {}},
        timeout=10,
    )
    if r.status_code == 200:
        assert r.json().get("success") is False
    else:
        assert r.status_code >= 400


# ── Section G: Browser UI ──────────────────────────────────────────────


async def test_extensions_tab_shows_registry(page):
    """Extensions tab loads and shows available extensions from registry."""
    tab_btn = page.locator(SEL["tab_button"].format(tab="extensions"))
    await tab_btn.click()
    panel = page.locator(SEL["tab_panel"].format(tab="extensions"))
    await panel.wait_for(state="visible", timeout=5000)

    available_section = page.locator(SEL["available_wasm_list"])
    await available_section.wait_for(state="visible", timeout=10000)
