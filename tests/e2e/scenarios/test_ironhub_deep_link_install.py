"""Deep-link install flow: hash parsing, verify-intent, error surfaces.

Covers the browser-side flow that the in-process handler tests cannot reach:
the user lands on `#/install/<slug>?slug=&version=&uid=&aid=&ts=&nonce=&sig=`,
the JS in `static/js/core/routing.js` parses the params, `install.js`
calls `POST /api/ironhub/verify-intent`, and the install card renders the
verify result. This locks in the wire contract end to end against
regressions in either the URL parser or the verify-intent handler.
"""

import hashlib
import hmac
import time
import uuid

from helpers import AUTH_TOKEN, api_post

SHARED_KEY = "ihub_sk_e2e_test_shared_key_padding_xx"
SLUG = "clickup"
VERSION = "0.1.0"
UID = "e2e-user"
AID = "e2e-agent"


def install_payload(slug: str, version: str, uid: str, aid: str, ts: int, nonce: str) -> str:
    return f"install:{slug}:{version}:{uid}:{aid}:{ts}:{nonce}"


def sign_install(key: str, slug: str, version: str, uid: str, aid: str, ts: int, nonce: str) -> str:
    msg = install_payload(slug, version, uid, aid, ts, nonce)
    return hmac.new(key.encode("utf-8"), msg.encode("utf-8"), hashlib.sha256).hexdigest()


def install_hash(*, slug: str, version: str, uid: str, aid: str, ts: int, nonce: str, sig: str) -> str:
    return (
        f"#/install/{slug}"
        f"?slug={slug}"
        f"&version={version}"
        f"&uid={uid}"
        f"&aid={aid}"
        f"&ts={ts}"
        f"&nonce={nonce}"
        f"&sig={sig}"
    )


async def _seed_signing_key(server: str) -> None:
    """Idempotently set the IronHub signing key for the test user."""
    response = await api_post(
        server,
        "/api/ironhub/signing-key",
        json={"shared_key": SHARED_KEY},
        timeout=10,
    )
    assert response.status_code == 200, (
        f"signing-key POST failed: {response.status_code} {response.text}"
    )


async def test_deep_link_install_valid_signature_renders_confirm(page, ironclaw_server):
    await _seed_signing_key(ironclaw_server)

    ts = int(time.time())
    nonce = uuid.uuid4().hex
    sig = sign_install(SHARED_KEY, SLUG, VERSION, UID, AID, ts, nonce)
    hash_fragment = install_hash(
        slug=SLUG, version=VERSION, uid=UID, aid=AID, ts=ts, nonce=nonce, sig=sig
    )

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}{hash_fragment}")

    confirm_btn = page.locator("#ironhub-install-confirm-btn")
    await confirm_btn.wait_for(state="visible", timeout=10000)
    assert await confirm_btn.is_enabled(), "valid sig must yield an enabled confirm button"

    card = page.locator("#tab-install .ironhub-install-card")
    card_text = (await card.inner_text()).lower()
    assert SLUG in card_text, f"expected slug '{SLUG}' on confirm card, got: {card_text!r}"


async def test_deep_link_install_confirm_click_posts_to_install_endpoint(page, ironclaw_server):
    await _seed_signing_key(ironclaw_server)

    ts = int(time.time())
    nonce = uuid.uuid4().hex
    sig = sign_install(SHARED_KEY, SLUG, VERSION, UID, AID, ts, nonce)
    hash_fragment = install_hash(
        slug=SLUG, version=VERSION, uid=UID, aid=AID, ts=ts, nonce=nonce, sig=sig
    )

    install_requests = []

    async def capture_install(route):
        install_requests.append(route.request)
        await route.fulfill(
            status=502,
            content_type="application/json",
            body='{"error": "catalog unreachable in e2e"}',
        )

    await page.route("**/api/ironhub/install", capture_install)
    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}{hash_fragment}")

    confirm_btn = page.locator("#ironhub-install-confirm-btn")
    await confirm_btn.wait_for(state="visible", timeout=10000)
    assert await confirm_btn.is_enabled(), "verified deep-link must yield an enabled confirm button"

    await confirm_btn.click()

    deadline = time.time() + 5
    while not install_requests and time.time() < deadline:
        await page.wait_for_timeout(100)

    assert install_requests, "clicking confirm must POST /api/ironhub/install"
    body = install_requests[0].post_data_json or {}
    assert body.get("name") == SLUG, f"install body must carry slug, got: {body!r}"


async def test_deep_link_install_tampered_signature_shows_mismatch(page, ironclaw_server):
    await _seed_signing_key(ironclaw_server)

    ts = int(time.time())
    nonce = uuid.uuid4().hex
    bad_sig = "deadbeef" * 8
    hash_fragment = install_hash(
        slug=SLUG, version=VERSION, uid=UID, aid=AID, ts=ts, nonce=nonce, sig=bad_sig
    )

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}{hash_fragment}")

    error = page.locator("#tab-install .ironhub-install-error")
    await error.wait_for(state="visible", timeout=10000)
    text = (await error.inner_text()).lower()
    assert "mismatch" in text, f"expected 'mismatch' in error, got: {text!r}"


async def test_deep_link_install_stale_timestamp_shows_drift(page, ironclaw_server):
    await _seed_signing_key(ironclaw_server)

    ts = int(time.time()) - 4000
    nonce = uuid.uuid4().hex
    sig = sign_install(SHARED_KEY, SLUG, VERSION, UID, AID, ts, nonce)
    hash_fragment = install_hash(
        slug=SLUG, version=VERSION, uid=UID, aid=AID, ts=ts, nonce=nonce, sig=sig
    )

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}{hash_fragment}")

    error = page.locator("#tab-install .ironhub-install-error")
    await error.wait_for(state="visible", timeout=10000)
    text = (await error.inner_text()).lower()
    assert "drift" in text, f"expected 'drift' in error, got: {text!r}"


async def test_deep_link_install_missing_params_shows_error(page, ironclaw_server):
    """JS short-circuits before calling verify-intent when any required param is absent."""
    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}#/install/{SLUG}")

    error = page.locator("#tab-install .ironhub-install-error")
    await error.wait_for(state="visible", timeout=10000)
    confirm_btn = page.locator("#ironhub-install-confirm-btn")
    assert not await confirm_btn.is_visible(), (
        "confirm button must NOT render when params are missing"
    )


async def test_deep_link_install_replayed_nonce_is_rejected(page, ironclaw_server):
    """The same nonce can only succeed once per user; replay returns the nonce error."""
    await _seed_signing_key(ironclaw_server)

    ts = int(time.time())
    nonce = uuid.uuid4().hex
    sig = sign_install(SHARED_KEY, SLUG, VERSION, UID, AID, ts, nonce)
    hash_fragment = install_hash(
        slug=SLUG, version=VERSION, uid=UID, aid=AID, ts=ts, nonce=nonce, sig=sig
    )

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}{hash_fragment}")
    confirm_btn = page.locator("#ironhub-install-confirm-btn")
    await confirm_btn.wait_for(state="visible", timeout=10000)

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}{hash_fragment}")
    error = page.locator("#tab-install .ironhub-install-error")
    await error.wait_for(state="visible", timeout=10000)
    text = (await error.inner_text()).lower()
    assert "nonce" in text, f"expected 'nonce' in replay error, got: {text!r}"
