# this is a test file to test the capability of live ironclaw, org "xyzorg"
#
# (setup is done manually, out of band — boot serve, set the xyzorg tenant,
#  install the capabilities, bootstrap the operator/director. not part of this file.)
#
# ============================================================================
# SECTION 1 — org + accounts
# ============================================================================
# 1. the org "xyzorg" is the serve tenant.
# 2. director is THE owner, established via env vars ONLY
#    (IRONCLAW_REBORN_WEBUI_USER_ID=director + IRONCLAW_REBORN_WEBUI_TOKEN). It is
#    NOT created via REST and is the single owner — REST mints no owners. The
#    operator env-bearer authenticates as `director`.
# 3. director creates officer@xyzorg.com as a member (user).
# 4. director promotes officer to ADMIN:
#      PUT /api/webchat/v2/admin/users/officer@xyzorg.com/role  {"role":"admin"}  (director's bearer)
# 5. director creates members alice, bob, carl (role = member).
#
# ============================================================================
# SECTION 2 — assign per-user capabilities (allow-list = "only X, deny the rest")
# ============================================================================
# mechanism (works today): enumerate the cap list, then hide each one NOT allowed:
#   GET  /api/webchat/v2/settings/tools                            -> the capability_id list
#   PUT  /api/webchat/v2/admin/users/{user}/capabilities/{cap_id}  {"availability":"hidden"}  -> per non-allowed cap
#   (use settings/tools for cap-ids; /admin/extensions returns PACKAGE ids.)
# 6. alice: GRANT builtin.shell + nearai.web_search.
# 7. bob:   GRANT google-drive.* (+ github.*).
# 8. carl:  grant nothing (deny-all = the essential baseline only).
#   NOTE: members now default to an ESSENTIAL allowlist (extension_search,
#   extension_activate, echo, time, json, memory_read, memory_search,
#   memory_write, memory_tree) + capability_info; EVERYTHING else is Hidden by
#   default and must be GRANTED by
#   an admin (an `available` per-user delta) — the model flipped from
#   "hide-the-rest" to "grant-the-allowed". Extension caps (nearai.*,
#   google-drive.*) surface to the model only via builtin.extension_search, so
#   they are not in the static surface even when granted. The per-user OFFERED
#   surface is captured from the serve log and asserted exactly (see SECTION 4).
# 9. bob's tools are USER-KEYED — set bob's own key per provider ("set a secret"):
#      POST /api/reborn/product-auth/manual-token/setup    (bob's bearer; get challenge)
#      POST /api/reborn/product-auth/manual-token/submit   (bob's bearer; provide bob's key/PAT)
#      (gdrive = Google is OAuth-keyed instead ->
#       POST /api/webchat/v2/extensions/google-drive/setup/oauth/start)
#
# ============================================================================
# SECTION 3 — assert role privileges (owner > admin > member)
# ============================================================================
# 10. admin can create members, and can PROMOTE a member to admin:
#       PUT /api/webchat/v2/admin/users/{user}/role  {"role":"admin"}   (officer's bearer)
#     -> once admin, that user gets the ADMIN defaults: their prior per-user capability
#        limitations no longer apply (admins are not capped to a grant set).
# 11. admins + owner can access all capabilities (the default set); per-user
#     hide-limitations apply to MEMBERS only.
# 12. deletion guards (run while all exist; after each 403 assert the target still exists):
#       officer(admin)  deletes director(owner)   -> 403  (admin may not delete an owner)
#       officer(admin)  deletes another admin     -> 403  (admin may not delete a peer admin)
#       officer(admin)  deletes himself           -> 403  (no self-delete)
#       director(owner) deletes himself           -> 403  (single owner is protected)
#       director(owner) deletes officer(admin)    -> 204  (owner outranks admin)
# 13. admin may NOT change the OWNER's capabilities:
#       PUT /api/webchat/v2/admin/users/director@xyzorg.com/capabilities/{cap}  (officer's bearer)
#       -> 403  (an admin may not modify an owner's grants)
#     (the org has exactly ONE owner for now — no second owner is created.)
#
# ============================================================================
# SECTION 4 — assert enforcement at dispatch (per-user tool surface)
# ============================================================================
# 14. run the SAME request as alice, bob, carl; assert each turn's trace / tool surface
#     contains ONLY that user's tools (alice: builtin.shell+web_search; bob: gdrive+github;
#     carl: none). assert at DISPATCH (the tools the model was offered), NOT from
#     settings/tools (the availability policy bites when the loop builds the surface).
# 15. bob's gdrive/github calls:
#      - key set       -> the tool turn completes.
#      - secret absent -> FINE: the user-keyed identity gate stops it GRACEFULLY
#                         (auth-required gate / unavailable), NOT a 500/crash.
#      - per-user isolation: bob's key is bob's alone; alice/carl never inherit it.
# 16. user-scoped approval prefs: any user may set "always approve" / "ask every time" /
#     "always deny" on a capability AVAILABLE to them
#       PUT /api/webchat/v2/settings/tools/{capability_id}   (the user's own bearer)
#     and CANNOT set it on an UNAVAILABLE capability -> rejected (403/404).

# ============================================================================
# EXECUTABLE BODY (the comment outline above is the SPEC and stays intact).
# ============================================================================
#
# STANDALONE validator — NOT pytest, NO fixtures, NO mock LLM. It drives a REAL,
# already-running `ironclaw-reborn serve` (booted from your .env) and validates
# the spec above against ACTUAL responses. The real model is non-deterministic,
# so SECTION 4 is BEHAVIORAL: it drives a real chat turn and reads the live SSE
# projection (an approval gate = the tool reached dispatch).
#
# Run (see tests/e2e/run_xyzorg.sh for the full clean-slate + boot + run recipe):
#   1. rm "$IRONCLAW_REBORN_HOME/local-dev/reborn-local-dev.db"      # clean slate
#   2. IRONCLAW_REBORN_CAPABILITY_POLICY=1 cargo run -p ironclaw_reborn_cli \
#        --features webui-v2-beta,capability-policy -- serve --port 3000
#   3. python tests/e2e/test_reborn_capability_policy_xyzorg.py
#
# Config (env overrides):
#   BASE_URL                      default http://127.0.0.1:3000
#   IRONCLAW_REBORN_WEBUI_TOKEN   owner bearer (matoken); else read from repo .env
#   BOB_GITHUB_PAT                optional; default a dummy PAT (stored, not validated)
#
# Exit code: 0 = all checks passed, 1 = one or more failed, 2 = harness/preflight error.

import asyncio
import os
import sys
import uuid
from pathlib import Path

try:
    import aiohttp
    import httpx
except ImportError as exc:  # pragma: no cover - environment guidance
    sys.stderr.write(
        f"missing dependency ({exc}); install the e2e extras:\n"
        "  cd tests/e2e && python -m venv .venv && source .venv/bin/activate && pip install -e .\n"
    )
    sys.exit(2)

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

BASE_URL = os.environ.get("BASE_URL", "http://127.0.0.1:3000").rstrip("/")
API = f"{BASE_URL}/api/webchat/v2"
PAUTH = f"{BASE_URL}/api/reborn/product-auth"

# The serve stderr/stdout log (the runner exports it). The ONLY place that
# records the per-user OFFERED model surface ("resolved provider tool
# definitions") — there is no API for it — so SECTION 4 reads it to capture and
# assert exactly which tools each user was offered. Without it, §4 skips.
SERVE_LOG = os.environ.get("SERVE_LOG", "")

# Builtin/extension cap-ids (verified against the live catalog).
SHELL_CAP_ID = "builtin.shell"
EXTENSION_SEARCH_CAP_ID = "builtin.extension_search"
# Provider tool names as the model sees them (capability_id `.` -> `__`).
SHELL_TOOL = "builtin__shell"
EXTENSION_SEARCH_TOOL = "builtin__extension_search"
# The provider tool names every MEMBER is offered by default (the essential
# allowlist; everything else is default-DENY and must be granted). Mirrors
# ESSENTIAL_MEMBER_CAPABILITIES in capability_policy_engine.rs. capability_info
# is the always-present host meta-tool and is asserted separately (not a
# `builtin__`). KEEP IN SYNC with the product list.
ESSENTIAL_BUILTINS = {
    "builtin__echo",
    "builtin__extension_activate",
    "builtin__extension_search",
    "builtin__json",
    "builtin__memory_read",
    "builtin__memory_search",
    "builtin__memory_tree",
    "builtin__memory_write",
    "builtin__time",
}

# Per-user allow-lists. Capabilities NOT in a member's set are hidden via the
# per-user policy delta surface. KEY FACT: extension capabilities (nearai.*,
# google-drive.*) are NEVER in the static model surface — the agent discovers
# them via builtin.extension_search — so every limited user gets extension_search
# so their granted extension tools are reachable at all. bob's grants are the
# real google-drive.* cap-ids (computed from the live catalog), not "gdrive".
def allow_set_for(member, caps):
    if member == "alice":
        return {SHELL_CAP_ID, "nearai.web_search", EXTENSION_SEARCH_CAP_ID}
    if member == "bob":
        ext = {c for c in caps if c.startswith(("google-drive.", "github."))}
        return ext | {EXTENSION_SEARCH_CAP_ID}
    if member == "carl":
        # deny-all EXCEPT extension_search (added per "everyone limited gets it").
        return {EXTENSION_SEARCH_CAP_ID}
    return set()

# bob's user-keyed GitHub PAT (manual-token). Stored only; never validated
# against GitHub, so any well-formed string proves the credential-storage path.
BOB_GITHUB_PAT = os.environ.get("BOB_GITHUB_PAT", "ghp_bob_personal_access_token_0123456789")

# A natural request used to populate the offered surface in the log and to check
# a user answers gracefully. NOT a forcing prompt: forcing a tool a user lacks
# sends the model into an unproductive capability_info loop that never replies.
SURFACE_PROBE_PROMPT = "Hello! In one short sentence, what can you help me with?"


class Skip(Exception):
    """A check that cannot run headlessly (interactive OAuth, model-dependent)."""


def _read_env_value(name: str) -> str:
    """Read `name` from the process env, else the FIRST exact-key match in the
    repo .env. Used for the owner bearer + owner user-id so the validator agrees
    with whatever the running serve was given."""
    val = os.environ.get(name)
    if val:
        return val
    env_path = Path(__file__).resolve().parents[2] / ".env"
    if env_path.exists():
        for raw in env_path.read_text(encoding="utf-8", errors="replace").splitlines():
            line = raw.strip()
            key, sep, rest = line.partition("=")
            if sep and key.strip() == name:
                v = rest.strip().strip('"').strip("'")
                if v:
                    return v
    return ""


# The owner is the env-configured user (matoken authenticates as this user_id).
# Read it so owner-self URLs target the real id rather than a hardcoded label.
OWNER_USER_ID = _read_env_value("IRONCLAW_REBORN_WEBUI_USER_ID") or "director"


def _bearer(token: str) -> dict:
    return {"Authorization": f"Bearer {token}"}


# ---------------------------------------------------------------------------
# REST helpers (every admin write goes through the owner or a minted admin
# bearer, mirroring the real /admin/* contract).
# ---------------------------------------------------------------------------

async def _create_user(client, admin_token, user_id, role):
    """POST /admin/users -> {user_id, role, token}. Asserts 200, returns token."""
    resp = await client.post(
        f"{API}/admin/users",
        headers=_bearer(admin_token),
        json={"user_id": user_id, "role": role},
        timeout=15,
    )
    assert resp.status_code == 200, f"create {user_id}({role}): {resp.status_code} {resp.text}"
    body = resp.json()
    assert body["user_id"] == user_id, body
    assert body["role"] == role, body
    assert body["token"], "create must return a non-empty bearer token"
    return body["token"]


async def _set_role(client, admin_token, user_id, role):
    return await client.put(
        f"{API}/admin/users/{user_id}/role",
        headers=_bearer(admin_token),
        json={"role": role},
        timeout=15,
    )


async def _delete_user(client, admin_token, user_id):
    return await client.request(
        "DELETE", f"{API}/admin/users/{user_id}", headers=_bearer(admin_token), timeout=15
    )


async def _enumerate_capability_ids(client, token):
    """GET /settings/tools -> capability_ids by stripping the `tool.` prefix.

    settings/tools returns operator-config entries keyed `tool.<capability_id>`
    plus an auto-approve entry; strip the prefix and drop non-`tool.` keys.
    """
    resp = await client.get(f"{API}/settings/tools", headers=_bearer(token), timeout=15)
    resp.raise_for_status()
    caps = set()
    for entry in resp.json().get("entries", []):
        key = entry.get("key", "")
        if key.startswith("tool."):
            caps.add(key[len("tool."):])
    return caps


async def _grant_capability(client, admin_token, user_id, capability_id):
    """PUT /admin/users/{user}/capabilities/{cap} {"availability":"available"}.

    The new model is GRANT-based: members default-DENY, so an admin GRANTS the
    allowed caps via an `available` delta (rather than hiding the rest). The
    per-user caps route is rate-limited PerCaller (60 / 60s); back off and retry
    on 429 (real product behavior, not a write failure)."""
    resp = None
    for attempt in range(14):
        resp = await client.put(
            f"{API}/admin/users/{user_id}/capabilities/{capability_id}",
            headers=_bearer(admin_token),
            json={"availability": "available"},
            timeout=15,
        )
        if resp.status_code != 429:
            return resp
        await asyncio.sleep(min(2.0 + attempt * 1.5, 12.0))
    return resp


async def _create_thread(client, token):
    resp = await client.post(
        f"{API}/threads",
        headers=_bearer(token),
        json={"client_action_id": str(uuid.uuid4())},
        timeout=15,
    )
    resp.raise_for_status()
    body = resp.json()
    thread = body.get("thread", body)
    # Tolerate either field name across serve versions (thread_id | id).
    thread_id = thread.get("thread_id") or thread.get("id") or body.get("thread_id")
    assert thread_id, f"create-thread response missing thread id: {body}"
    return thread_id


# ---------------------------------------------------------------------------
# SSE projection draining (the real, mock-free dispatch signal).
# ---------------------------------------------------------------------------

async def _drain_projection_items(thread_id, token, *, timeout: float = 30.0):
    """Open the v2 SSE stream (via the ?token= shim) and return the items of the
    LAST projection state frame seen, stopping early once the run settles.

    Each `projection_snapshot`/`projection_update` carries the full renderable
    `ProductProjectionState`. We read until the run blocks on a gate or reaches a
    terminal state, so a caller can tell whether a capability reached dispatch.
    """
    import json as _json

    events_url = f"{API}/threads/{thread_id}/events?token={token}"
    last_items: list = []
    client_timeout = aiohttp.ClientTimeout(total=timeout + 2, sock_read=timeout + 2)
    try:
        async with aiohttp.ClientSession(timeout=client_timeout) as session:
            async with session.get(events_url, headers={"Accept": "text/event-stream"}) as response:
                if response.status != 200:
                    return last_items
                try:
                    async with asyncio.timeout(timeout):
                        async for raw in response.content:
                            line = raw.decode("utf-8", "replace").strip()
                            if not line.startswith("data:") or line == "data:":
                                continue
                            try:
                                frame = _json.loads(line[5:].strip())
                            except _json.JSONDecodeError:
                                continue
                            if frame.get("type") in ("projection_snapshot", "projection_update"):
                                items = (frame.get("state") or {}).get("items")
                                if items is not None:
                                    last_items = items
                            if _items_run_settled(last_items):
                                break
                except (asyncio.TimeoutError, TimeoutError):
                    pass
    except Exception:
        return last_items
    return last_items


def _items_run_settled(items: list) -> bool:
    """True once a projection item set shows the run blocked on a gate or done."""
    for item in items:
        if item.get("gate"):
            return True
        run_status = item.get("run_status")
        if run_status and run_status.get("status") in (
            "blocked_approval",
            "blocked_auth",
            "completed",
            "failed",
            "cancelled",
        ):
            return True
    return False


def _items_have_approval_gate(items: list) -> bool:
    """True if the projection shows the run parked on an approval gate.

    builtin.shell carries an approval requirement in the serve profile, so a
    granted member's shell call dispatches and parks at an approval gate. The
    gate's presence is the proof the capability passed the policy filter and
    reached dispatch (the v2 GatePromptView does not surface the raw cap-id).
    """
    return any((item.get("gate") or {}).get("gate_kind") == "approval" for item in items)


def _items_run_status(items: list):
    status = None
    for item in items:
        run_status = item.get("run_status")
        if run_status and run_status.get("status"):
            status = run_status["status"]
    return status


async def _drive_probe(client, token, prompt=SURFACE_PROBE_PROMPT):
    """Send a chat turn as `token`; return (thread_id, final SSE projection items).

    Sending the turn makes the loop resolve + log the per-user OFFERED tool
    surface (read back from SERVE_LOG by thread_id). The drained items give the
    run outcome (gate / completed / failed)."""
    thread_id = await _create_thread(client, token)
    send = await client.post(
        f"{API}/threads/{thread_id}/messages",
        headers=_bearer(token),
        json={"client_action_id": str(uuid.uuid4()), "content": prompt},
        timeout=30,
    )
    assert send.status_code in (200, 202), f"send: {send.status_code} {send.text}"
    await asyncio.sleep(1.0)
    items = await _drain_projection_items(thread_id, token)
    return thread_id, items


# ---------------------------------------------------------------------------
# SECTION 5 helpers — live memory write (two real turns) + cross-member /fs read.
# ---------------------------------------------------------------------------

# Local-dev memory virtual path:
#   /memory/tenants/<tenant>/users/<user_id>/agents/<agent>/projects/<project>/<doc>
# tenant/agent/project are local-dev constants; the user segment is the directory
# user_id. `tenants/reborn-cli/users/bob` is exactly the prefix the Workspace tab
# shows for member "bob".
MEMORY_TENANT = "reborn-cli"


def _member_memory_base(user_id: str) -> str:
    return f"tenants/{MEMORY_TENANT}/users/{user_id}"


async def _post_message(client, thread_id, token, prompt):
    """POST one chat turn to an existing thread; assert it was accepted."""
    send = await client.post(
        f"{API}/threads/{thread_id}/messages",
        headers=_bearer(token),
        json={"client_action_id": str(uuid.uuid4()), "content": prompt},
        timeout=30,
    )
    assert send.status_code in (200, 202), f"send: {send.status_code} {send.text}"


async def _send_and_settle(client, thread_id, token, prompt, *, timeout: float = 60.0):
    """POST a turn and drain its SSE projection until the run settles, so the poem
    turn is finished (in context, thread no longer busy) before the next turn."""
    await _post_message(client, thread_id, token, prompt)
    await asyncio.sleep(1.0)
    return await _drain_projection_items(thread_id, token, timeout=timeout)


async def _poll_member_memory_files(client, token, base, *, timeout: float = 45.0, interval: float = 2.5):
    """Poll the caller's OWN memory (via the caller-confined mount) until at least
    one non-empty file appears, or `timeout` elapses. Returns the files (possibly
    empty).

    This is the live, mock-free, race-free signal that a `memory_write` turn
    actually persisted: a real-LLM turn takes a few seconds, so we re-read until the
    document lands rather than draining the run's SSE stream (whose tool-lifecycle
    frames the drain-only local-dev projection does not surface)."""
    waited = 0.0
    while True:
        files = await _walk_member_memory_files(client, token, base)
        if files or waited >= timeout:
            return files
        await asyncio.sleep(interval)
        waited += interval


async def _walk_member_memory_files(client, token, base, *, max_depth: int = 6, max_reads: int = 25):
    """As `token`, walk the `memory` mount under `base` via GET /fs/list and read
    each file via GET /fs/content. Returns [{path, bytes, snippet}] for every
    NON-EMPTY file the caller could actually read. An empty list means the caller
    reached no file under `base` — the isolated outcome."""
    found: list = []
    queue = [(base, 0)]
    seen = set()
    while queue and len(found) < max_reads:
        path, depth = queue.pop(0)
        if path in seen or depth > max_depth:
            continue
        seen.add(path)
        resp = await client.get(
            f"{API}/fs/list",
            params={"mount": "memory", "path": path},
            headers=_bearer(token),
            timeout=15,
        )
        if resp.status_code != 200:
            continue
        for entry in (resp.json().get("entries") or []):
            epath = entry.get("path")
            if not epath:
                name = entry.get("name")
                epath = f"{path}/{name}" if name else None
            if not epath:
                continue
            if entry.get("kind") == "directory":
                queue.append((epath, depth + 1))
            elif entry.get("kind") == "file":
                content = await client.get(
                    f"{API}/fs/content",
                    params={"mount": "memory", "path": epath},
                    headers=_bearer(token),
                    timeout=15,
                )
                if content.status_code == 200 and content.content:
                    found.append(
                        {"path": epath, "bytes": len(content.content), "snippet": content.text[:160]}
                    )
                    if len(found) >= max_reads:
                        break
    return found


# ---------------------------------------------------------------------------
# SECTION 6 helpers — mocked SSO login (forged session) + email provisioning.
# ---------------------------------------------------------------------------

# Two SSO identities, keyed by email (token-less — they authenticate via Google,
# not a minted bearer): a new admin, and carl, who has moved to SSO.
# SSO test emails are ENV-DRIVEN so no real address is committed. Defaults use the
# fictional test org (xyzorg.com); set SSO_ADMIN_EMAIL / SSO_MEMBER_EMAIL to your
# own Google accounts for a real-Google manual run.
SSO_ADMIN_EMAIL = os.environ.get("SSO_ADMIN_EMAIL", "product@xyzorg.com")  # NEW SSO admin
SSO_MEMBER_EMAIL = os.environ.get("SSO_MEMBER_EMAIL", "carl@xyzorg.com")  # carl, now an SSO member
# Distinct SSO identity for the memory-write check so it never collides (409)
# with the member-surface check that already provisions SSO_MEMBER_EMAIL.
SSO_MEMORY_EMAIL = os.environ.get("SSO_MEMORY_EMAIL", "ssomem@xyzorg.com")
SSO_TENANT = "reborn-cli"  # local-dev tenant the SSO session token binds to


def _mint_sso_bearer(secret, tenant, user, *, lifetime_s=3600):
    """Mock an SSO session bearer: the stateless HMAC-signed token a Google login
    WOULD mint (SignedTokenSessionStore). Lets the validator authenticate as an
    SSO user with no Google/browser. `secret` is the serve's SSO signing key,
    which is IRONCLAW_REBORN_WEBUI_TOKEN. The close-to-life upgrade is a
    mock-Google token endpoint driving /auth/callback; this forge is the
    deterministic stand-in over the SAME SessionAuthenticator. (Real SSO assigns
    a UUID user_id; the validator keys the SSO user by email to match the
    email-based provisioning.)"""
    import base64 as _b64
    import hashlib
    import hmac
    import json as _json
    import struct
    import time

    key = hashlib.sha256(
        b"ironclaw-reborn-webui-session-v1::"
        + struct.pack("<Q", len(tenant.encode()))
        + tenant.encode()
        + b"::"
        + secret.encode()
    ).digest()
    now = int(time.time())
    payload = {
        "sid": str(uuid.uuid4()),
        "tenant": tenant,
        "user": user,
        "iat": now,
        "exp": now + lifetime_s,
    }

    def enc(raw):
        return _b64.urlsafe_b64encode(raw).rstrip(b"=").decode()

    payload_b64 = enc(_json.dumps(payload, separators=(",", ":")).encode())
    sig = enc(hmac.new(key, payload_b64.encode(), hashlib.sha256).digest())
    return f"{payload_b64}.{sig}"


async def _provision_sso_user(client, admin_token, email, role):
    """Designate an SSO user BY EMAIL via the admin REST (the email surface):
    token-less + role-bearing. The email-based endpoint is NOT built yet, so the
    caller treats any non-2xx as 'not wired' and Skips."""
    return await client.post(
        f"{API}/admin/users",
        headers=_bearer(admin_token),
        json={"email": email, "role": role, "sso": True},
        timeout=15,
    )


async def _sso_login_or_skip(client, owner, email, role):
    """Provision an SSO user by email, then mock-login (forged bearer). Raises
    Skip until BOTH the email-provisioning endpoint and SSO session auth are
    wired — so SECTION 6 stays inert (skipped, not red) until the backend lands:
    change A (role-derived admin) + the admin email surface + a configured SSO
    provider (so SessionAuthenticator is active)."""
    resp = await _provision_sso_user(client, owner, email, role)
    if resp.status_code not in (200, 201):
        raise Skip(
            f"email-based SSO provisioning not wired ({resp.status_code} for {email}); "
            "needs the admin REST email surface"
        )
    # The backend derives a valid user_id from the email (emails aren't valid
    # user ids); forge the SSO session with THAT id so it matches the record.
    user_id = (resp.json() or {}).get("user_id")
    if not user_id:
        raise Skip(f"SSO provisioning returned no user_id for {email}: {resp.text}")
    bearer = _mint_sso_bearer(owner, SSO_TENANT, user_id)
    probe = await client.get(f"{API}/session", headers=_bearer(bearer), timeout=10)
    if probe.status_code in (401, 403):
        raise Skip(
            f"forged SSO bearer rejected ({probe.status_code}); SSO session auth not wired "
            "(serve needs an SSO provider configured to activate SessionAuthenticator)"
        )
    return bearer


def _offered_provider_tools_from_log(thread_id):
    """Read the OFFERED model surface for `thread_id` from SERVE_LOG.

    The serve logs `resolved provider tool definitions tool_definition_count=N
    tool_name_sample=[...]` inside a span carrying `thread_id=...`. There is no
    API for the per-user offered surface, so this log line is the source of
    truth. Returns (count, names) for the LAST resolution seen for the thread;
    `names` is the first-20 sample (complete for our <=20-tool member surfaces).
    Returns (None, None) if SERVE_LOG is unset/unreadable or no line is found.
    """
    if not SERVE_LOG or not Path(SERVE_LOG).exists():
        return None, None
    import re

    ansi = re.compile(r"\x1b\[[0-9;]*m")
    count = None
    names = None
    try:
        for raw in Path(SERVE_LOG).read_text(encoding="utf-8", errors="replace").splitlines():
            line = ansi.sub("", raw)
            if thread_id not in line or "resolved provider tool definitions" not in line:
                continue
            m = re.search(r"tool_definition_count=(\d+)", line)
            s = re.search(r'tool_name_sample=\[([^\]]*)\]', line)
            if m:
                count = int(m.group(1))
            if s:
                names = set(re.findall(r'"([^"]+)"', s.group(1)))
    except OSError:
        return None, None
    return count, names


# ---------------------------------------------------------------------------
# Bootstrap the org (SECTION 1 + 2) against the live, clean instance.
# ---------------------------------------------------------------------------

async def bootstrap_org(client, owner):
    """Create the directory + apply allow-lists, returning shared state. Raises on
    a hard failure (a clean DB is assumed — run after dropping the local-dev db)."""
    state = {"owner": owner, "owner_user_id": OWNER_USER_ID, "tokens": {OWNER_USER_ID: owner}}

    # SECTION 1: director (env owner) creates officer (member->admin) + members.
    state["tokens"]["officer"] = await _create_user(client, owner, "officer", "member")
    promote = await _set_role(client, owner, "officer", "admin")
    assert promote.status_code == 200, f"promote officer->admin: {promote.status_code} {promote.text}"
    for member in ("alice", "bob", "carl"):
        state["tokens"][member] = await _create_user(client, owner, member, "member")

    # SECTION 2: GRANT each member's allow-set (Available deltas). Under the
    # default-DENY member model, anything NOT granted is already Hidden, so we no
    # longer "hide the rest" — we GRANT the allowed caps on top of the essential
    # baseline. Only a handful of writes per member, so no rate-limit fan-out is
    # needed; _grant_capability still backs off on 429 as a safety net.
    caps = await _enumerate_capability_ids(client, owner)
    state["capabilities"] = caps
    allow_by_user = {m: allow_set_for(m, caps) for m in ("alice", "bob", "carl")}
    state["allow_by_user"] = allow_by_user
    grant_results = {}
    for member, allow in allow_by_user.items():
        for cap in sorted(allow):
            resp = await _grant_capability(client, owner, member, cap)
            grant_results[(member, cap)] = resp.status_code
    state["grant_results"] = grant_results
    return state


# ---------------------------------------------------------------------------
# Section checks (each raises AssertionError on failure, Skip when not runnable).
# ---------------------------------------------------------------------------

async def check_section1_accounts(client, state):
    """Steps 1-5: the directory has the env owner + four REST users, distinct bearers."""
    tokens = state["tokens"]
    for who in (OWNER_USER_ID, "officer", "alice", "bob", "carl"):
        assert tokens.get(who), f"missing bearer for {who}"
    assert len(set(tokens.values())) == len(tokens), "bearers must be distinct"
    # the owner is the env bearer; it is NOT a directory row.
    listed = await client.get(f"{API}/admin/users", headers=_bearer(state["owner"]), timeout=15)
    listed.raise_for_status()
    ids = {u["user_id"] for u in listed.json().get("users", [])}
    assert {"officer", "alice", "bob", "carl"} <= ids, f"directory missing REST users: {ids}"
    assert OWNER_USER_ID not in ids, f"owner {OWNER_USER_ID!r} is env-only and must NOT be a directory row"


async def check_section2_grants(client, state):
    """Steps 6-8: every per-user `available` grant write succeeded against the
    live admin surface (200). (New model: admin GRANTS the allow-set on top of
    the essential baseline, rather than hiding the rest.)"""
    assert isinstance(state["capabilities"], set)
    bad = {k: v for k, v in state["grant_results"].items() if v != 200}
    assert not bad, f"some grant writes did not return 200: {bad}"


async def check_section2_bob_github_key(client, state):
    """Step 9 (github): set bob's own GitHub PAT via the REAL manual-token flow.

    setup -> {interaction_id, invocation_id}; secret-submit carries both back plus
    the raw token and returns a redacted credential_ref. The credential is owned
    at BOB's user_id (scope derived server-side from bob's bearer)."""
    bob = state["tokens"]["bob"]
    setup = await client.post(
        f"{PAUTH}/manual-token/setup",
        headers=_bearer(bob),
        json={"provider": "github", "account_label": "bob-github"},
        timeout=15,
    )
    assert setup.status_code == 200, f"manual-token setup: {setup.status_code} {setup.text}"
    sb = setup.json()
    interaction_id, invocation_id = sb.get("interaction_id"), sb.get("invocation_id")
    assert interaction_id and invocation_id, sb
    submit = await client.post(
        f"{PAUTH}/manual-token/secret-submit",
        headers=_bearer(bob),
        json={
            "interaction_id": interaction_id,
            "invocation_id": invocation_id,
            "token": BOB_GITHUB_PAT,
        },
        timeout=15,
    )
    assert submit.status_code == 200, f"manual-token secret-submit: {submit.status_code} {submit.text}"
    body = submit.json()
    assert body.get("credential_ref"), body
    assert body.get("status"), body


async def check_section2_bob_gdrive_oauth(client, state):
    """Step 9 (gdrive): Google Drive is OAuth-keyed; completing the credential
    needs a real browser consent + provider callback, not driveable headlessly."""
    raise Skip("gdrive is Google-OAuth-keyed; consent + callback need a real browser")


async def check_section3_step10_admin_creates_and_promotes(client, state):
    """Step 10: an ADMIN (officer) can create a member and promote it to admin —
    proving the create + role endpoints work via a non-owner admin bearer."""
    officer = state["tokens"]["officer"]
    made = await _create_user(client, officer, "admin-made-member", "member")
    assert made, "admin-created member must mint a bearer"
    promote = await _set_role(client, officer, "admin-made-member", "admin")
    assert promote.status_code == 200, f"admin promote member->admin: {promote.status_code} {promote.text}"
    assert promote.json().get("role") == "admin", promote.text


async def check_section3_step12_owner_deletes_admin(client, state):
    """Step 12 (working case): owner outranks admin -> 204."""
    owner = state["owner"]
    await _create_user(client, owner, "del-target", "member")
    promote = await _set_role(client, owner, "del-target", "admin")
    assert promote.status_code == 200, promote.text
    deleted = await _delete_user(client, owner, "del-target")
    assert deleted.status_code == 204, f"owner->admin delete: {deleted.status_code} {deleted.text}"


async def check_section3_step12_guards(client, state):
    """Step 12 (guard cases): self-delete, admin->owner, admin->peer-admin, owner self.

    director is the ENV owner (not a directory row), so admin->owner resolves the
    target as unknown and rejects with 404 (vs 403 if it were a row) — either way
    an admin cannot delete the owner.
    """
    owner = state["owner"]
    guard_admin = await _create_user(client, owner, "guard-admin", "admin")
    await _create_user(client, owner, "guard-peer", "admin")

    # admin -> owner: rejected (404 since the env owner is not a directory row).
    r1 = await _delete_user(client, guard_admin, OWNER_USER_ID)
    assert r1.status_code in (403, 404), f"admin->owner delete must be rejected, got {r1.status_code}"

    # admin -> peer admin: 403 (no peer-admin delete).
    r2 = await _delete_user(client, guard_admin, "guard-peer")
    assert r2.status_code == 403, f"admin->peer-admin delete must be 403, got {r2.status_code}"

    # admin -> self: 403 (no self-delete).
    r3 = await _delete_user(client, guard_admin, "guard-admin")
    assert r3.status_code == 403, f"admin self-delete must be 403, got {r3.status_code}"

    # owner -> self: 403 (self-delete guard fires before the directory lookup).
    r4 = await _delete_user(client, owner, OWNER_USER_ID)
    assert r4.status_code == 403, f"owner self-delete must be 403, got {r4.status_code}"


async def check_section3_step13_admin_cannot_change_owner_caps(client, state):
    """Step 13: an admin editing the owner's caps is rejected (403/404).

    director is the env owner, not a directory row, so the per-user-caps route
    cannot resolve its role for the rank check and rejects with 404 (target
    unknown) rather than 403 — either way an admin cannot write the owner's caps.
    (A clean 403-because-owner needs the env owner recognized by the directory —
    the still-open owner-model gap.)
    """
    owner = state["owner"]
    cap = next(iter(state["capabilities"]), SHELL_CAP_ID)
    admin = await _create_user(client, owner, "caps-admin", "admin")
    resp = await client.put(
        f"{API}/admin/users/{OWNER_USER_ID}/capabilities/{cap}",
        headers=_bearer(admin),
        json={"availability": "hidden"},
        timeout=15,
    )
    assert resp.status_code in (403, 404), (
        f"admin editing owner's caps must be rejected (403/404), got {resp.status_code}"
    )


async def check_section3_rest_cannot_create_owner(client, state):
    """Owner-model invariant: REST mints no owners (THE owner is env-only)."""
    owner = state["owner"]
    create = await client.post(
        f"{API}/admin/users",
        headers=_bearer(owner),
        json={"user_id": "usurper", "role": "owner"},
        timeout=15,
    )
    assert create.status_code == 403, f"REST create owner must be 403, got {create.status_code} {create.text}"
    promote = await _set_role(client, owner, "alice", "owner")
    assert promote.status_code == 403, f"REST promote-to-owner must be 403, got {promote.status_code}"


async def _offered_builtins(client, token, *, thread_id_only=False):
    """Drive a turn as `token` and return the set of OFFERED `builtin__*` tool
    names (from the serve log) plus (count, all_names, items). Raises Skip when
    the offered surface cannot be captured (no SERVE_LOG)."""
    tid, items = await _drive_probe(client, token)
    count, names = _offered_provider_tools_from_log(tid)
    if names is None:
        raise Skip(
            "SERVE_LOG not set/readable — cannot capture the offered surface "
            "(run via tests/e2e/run_xyzorg.sh, which exports it)"
        )
    builtins = {n for n in names if n.startswith("builtin__")}
    return builtins, count, names, items


async def check_section4_alice_surface(client, state):
    """Step 14 (alice): under the default-DENY member model, every member is
    offered the ESSENTIAL_BUILTINS baseline; alice is additionally GRANTED
    builtin.shell, so her offered builtins must be EXACTLY essentials ∪ {shell}.
    FAILS if her grant is missing OR an un-granted, non-essential builtin leaked.
    (nearai.web_search is an extension cap reached via extension_search, so it is
    not in the static surface even though granted.)"""
    builtins, count, _, _ = await _offered_builtins(client, state["tokens"]["alice"])
    expected = ESSENTIAL_BUILTINS | {SHELL_TOOL}
    assert builtins == expected, (
        f"alice's offered builtins must be essentials ∪ {{shell}}, got {sorted(builtins)} "
        f"(count={count}); missing={sorted(expected - builtins)} leaked={sorted(builtins - expected)}"
    )


async def check_section4_bob_surface(client, state):
    """Step 14 (bob): bob's grants are extension caps (google-drive.*), which are
    NOT in the static surface (reached via extension_search), so bob's offered
    builtins must be EXACTLY the essential baseline — no shell, no leaked builtin."""
    builtins, count, _, _ = await _offered_builtins(client, state["tokens"]["bob"])
    assert builtins == ESSENTIAL_BUILTINS, (
        f"bob's offered builtins must be exactly the essential baseline, got {sorted(builtins)} "
        f"(count={count}); diff={sorted(builtins ^ ESSENTIAL_BUILTINS)}"
    )


async def check_section4_carl_surface_and_graceful(client, state):
    """Step 14/15 (carl): deny-all (granted nothing beyond essentials). Offered
    builtins must be EXACTLY the essential baseline (this is what catches a leak
    like skill_install), AND a normal request must be handled GRACEFULLY — not a
    terminal run failure, no approval gate."""
    builtins, count, _, items = await _offered_builtins(client, state["tokens"]["carl"])
    assert builtins == ESSENTIAL_BUILTINS, (
        f"carl's offered builtins must be exactly the essential baseline, got {sorted(builtins)} "
        f"(count={count}); a leaked builtin (e.g. skill_install) fails here — "
        f"diff={sorted(builtins ^ ESSENTIAL_BUILTINS)}"
    )
    status = _items_run_status(items)
    assert status != "failed", (
        f"carl must answer a normal request gracefully, not a terminal failure; run_status={status}"
    )
    assert not _items_have_approval_gate(items), "carl must not park on an approval gate"


async def check_section4_admin_surface(client, state):
    """Step 11/14 (admin): an admin bypasses per-user hides and is offered the
    FULL builtin surface — shell present, and far more than a capped member."""
    admin = await _create_user(client, state["owner"], "surface-admin", "admin")
    builtins, count, names, _ = await _offered_builtins(client, admin)
    assert SHELL_TOOL in builtins, f"admin must be offered shell (bypass); got {sorted(builtins)}"
    assert (count or 0) >= 30, f"admin must see the full builtin surface (~35), got count={count}"


async def check_section4_step16_approval_pref_available(client, state):
    """Step 16 (available): a member may set an approval pref on a cap AVAILABLE to
    them. alice keeps builtin.shell, so the write succeeds (200)."""
    alice = state["tokens"]["alice"]
    resp = await client.put(
        f"{API}/settings/tools/{SHELL_CAP_ID}",
        headers=_bearer(alice),
        json={"state": "always_allow"},
        timeout=15,
    )
    assert resp.status_code == 200, f"approval pref on available cap: {resp.status_code} {resp.text}"


async def check_section4_step16_approval_pref_unavailable(client, state):
    """Step 16 (rejection): setting an approval pref on a cap UNAVAILABLE to the
    member is rejected (403/404). carl has builtin.shell hidden."""
    carl = state["tokens"]["carl"]
    resp = await client.put(
        f"{API}/settings/tools/{SHELL_CAP_ID}",
        headers=_bearer(carl),
        json={"state": "always_allow"},
        timeout=15,
    )
    assert resp.status_code in (403, 404), (
        f"approval pref on unavailable cap must be rejected, got {resp.status_code}"
    )


async def check_section5_member_memory_isolation(client, state):
    """SECTION 5 — cross-member memory isolation (live; the leak this PR targets).

    NO mock. Two real turns as bob: write a poem, then persist it to memory. We
    confirm the write actually LANDED by reading bob's OWN memory back through the
    caller-confined mount (the live, race-free signal that memory_write succeeded) —
    else the isolation probe is moot and we FAIL. Then alice — a DIFFERENT member —
    tries to read bob's memory through the WebUI filesystem browser (GET /fs/list +
    /fs/content, mount=memory) at bob's known absolute path. If alice can read ANY of
    bob's memory bytes, that is a cross-user leak -> FAIL. After the isolation fix
    alice reaches no file under bob's path and this PASSES.

    bob's/alice's user_id are the directory ids bootstrap_org created via
    `_create_user(...)` — the same values keyed in `state["tokens"]` — and they are
    exactly the `users/<id>` segment in the memory mount path.
    """
    bob_uid, alice_uid = "bob", "alice"
    bob, alice = state["tokens"][bob_uid], state["tokens"][alice_uid]

    # Steps 1-2: bob writes a poem (turn 1, settled) then asks to save it (turn 2).
    thread_id = await _create_thread(client, bob)
    await _send_and_settle(client, thread_id, bob, "Write a short, original four-line poem about apples.")
    await _post_message(
        client,
        thread_id,
        bob,
        "Save that poem into your long-term memory now, using your memory_write tool, "
        "so you can recall it later.",
    )

    # Steps 3-4: confirm the write landed by reading bob's OWN memory back. The
    # caller-confined memory root lists only bob's subtree; empty after the poll
    # window => memory_write never persisted => the probe is moot -> FAIL.
    bob_files = await _poll_member_memory_files(client, bob, "")
    assert bob_files, (
        "after asking bob to save the poem, bob has no readable memory document — "
        "memory_write did not persist within the poll window, so the cross-member "
        "isolation probe cannot run (re-run; if it recurs the model may not be "
        "calling memory_write)."
    )

    # Steps 5-7: alice must NOT be able to read bob's memory through /fs.
    bob_base = _member_memory_base(bob_uid)
    leaked = await _walk_member_memory_files(client, alice, bob_base)
    assert not leaked, (
        f"MEMORY LEAK: member '{alice_uid}' read member '{bob_uid}' memory via {API}/fs "
        f"(mount=memory, base={bob_base!r}). Files alice could read: "
        f"{[(f['path'], f['bytes']) for f in leaked]}. First snippet: {leaked[0]['snippet']!r}"
    )

    # Strict isolation applies to EVERY role — admins/owner deliberately get NO
    # cross-member memory oversight. A fresh admin and the env owner (director) must
    # ALSO be confined to their own subtree and reach none of bob's memory.
    mem_admin = await _create_user(client, state["owner"], "memprobe-admin", "admin")
    for role_label, role_token in (("admin", mem_admin), ("owner", state["owner"])):
        role_leaked = await _walk_member_memory_files(client, role_token, bob_base)
        assert not role_leaked, (
            f"MEMORY LEAK: {role_label} read member '{bob_uid}' memory via {API}/fs "
            f"(mount=memory, base={bob_base!r}) — strict isolation must apply to all roles "
            f"(no admin/owner oversight). Files read: {[(f['path'], f['bytes']) for f in role_leaked]}."
        )


async def check_section6_sso_admin_parity(client, state):
    """SECTION 6 (SSO ADMIN = SSO_ADMIN_EMAIL): an SSO admin must do
    everything a token (non-SSO) admin can — reach the admin command plane
    (manage members over REST) AND get the full tool surface. Mocked SSO login.
    SKIPs until the SSO backend lands; FAILs if an SSO admin authenticates but is
    denied the command plane — exactly the gap change A (role-derived operator
    capability) closes."""
    bearer = await _sso_login_or_skip(client, state["owner"], SSO_ADMIN_EMAIL, "admin")
    # 1) Command plane: list users + change a member's capability, like a token admin.
    listing = await client.get(f"{API}/admin/users", headers=_bearer(bearer), timeout=15)
    assert listing.status_code == 200, (
        f"SSO admin must reach the admin command plane (GET /admin/users), got "
        f"{listing.status_code} — role-derived operator capability (change A) is missing"
    )
    grant = await client.put(
        f"{API}/admin/users/alice/capabilities/{SHELL_CAP_ID}",
        headers=_bearer(bearer),
        json={"availability": "available"},
        timeout=15,
    )
    assert grant.status_code == 200, (
        f"SSO admin must manage members like a token admin, got {grant.status_code} {grant.text}"
    )
    # 2) Full tool surface (token-admin parity: shell present, ~full builtin set).
    builtins, count, _, _ = await _offered_builtins(client, bearer)
    assert SHELL_TOOL in builtins, f"SSO admin must get the full surface (shell); got {sorted(builtins)}"
    assert (count or 0) >= 30, f"SSO admin must see the full builtin surface (~35), got count={count}"


async def check_section6_sso_carl_still_member(client, state):
    """SECTION 6 (carl, SSO MEMBER = SSO_MEMBER_EMAIL): carl moved to SSO but
    still does what carl does — EXACTLY the essential baseline, graceful, no
    leaked builtin, no approval gate. Mocked SSO login. SKIPs until the SSO
    backend lands."""
    bearer = await _sso_login_or_skip(client, state["owner"], SSO_MEMBER_EMAIL, "member")
    builtins, count, _, items = await _offered_builtins(client, bearer)
    assert builtins == ESSENTIAL_BUILTINS, (
        f"SSO carl must be exactly the essential baseline, got {sorted(builtins)} "
        f"(count={count}); diff={sorted(builtins ^ ESSENTIAL_BUILTINS)}"
    )
    status = _items_run_status(items)
    assert status != "failed", f"SSO carl must answer gracefully, not a terminal failure; run_status={status}"
    assert not _items_have_approval_gate(items), "SSO carl must not park on an approval gate"


async def check_section6_sso_memory_write_persists(client, state):
    """SECTION 6: an SSO user can actually WRITE persistent memory — the exact
    path the `sso-<id>` colon bug broke (memory failed with invalid_input for SSO
    users while token users were fine, because the derived user_id had a reserved
    ':' segment). Drive a real memory_write turn as the SSO member, then confirm
    the doc landed by reading the member's OWN confined memory back via /fs.
    SKIPs until SSO is wired; FAILS if the write doesn't persist."""
    bearer = await _sso_login_or_skip(client, state["owner"], SSO_MEMORY_EMAIL, "member")
    thread_id = await _create_thread(client, bearer)
    await _post_message(
        client,
        thread_id,
        bearer,
        "Save a short note to your long-term memory now, using your memory_write "
        "tool, so you can recall it later.",
    )
    files = await _poll_member_memory_files(client, bearer, "")
    assert files, (
        "SSO user's memory_write did not persist: after asking to save a note, the "
        "user's own memory is still empty. This is the SSO-scoped memory break "
        "(e.g. a reserved ':' in the derived user_id segment) — token users are "
        "unaffected, so it only surfaces for an SSO identity."
    )


CHECKS = [
    ("S1 accounts + roles", check_section1_accounts),
    ("S2 allow-list grants", check_section2_grants),
    ("S2 bob github key (manual-token)", check_section2_bob_github_key),
    ("S2 bob gdrive (oauth)", check_section2_bob_gdrive_oauth),
    ("S3.10 admin creates + promotes", check_section3_step10_admin_creates_and_promotes),
    ("S3.12 owner deletes admin (204)", check_section3_step12_owner_deletes_admin),
    ("S3.12 deletion guards (403/404)", check_section3_step12_guards),
    ("S3.13 admin cannot change owner caps", check_section3_step13_admin_cannot_change_owner_caps),
    ("S3 REST cannot create/promote owner", check_section3_rest_cannot_create_owner),
    ("S4.14 alice offered surface", check_section4_alice_surface),
    ("S4.14 bob offered surface", check_section4_bob_surface),
    ("S4.14 carl offered surface + graceful", check_section4_carl_surface_and_graceful),
    ("S4.11 admin full surface (bypass)", check_section4_admin_surface),
    ("S4.16 approval pref on available cap", check_section4_step16_approval_pref_available),
    ("S4.16 approval pref on unavailable cap", check_section4_step16_approval_pref_unavailable),
    ("S5 memory isolation: bob hidden from alice + admin + owner", check_section5_member_memory_isolation),
    ("S6 SSO admin parity (SSO_ADMIN_EMAIL)", check_section6_sso_admin_parity),
    ("S6 SSO carl still member (SSO_MEMBER_EMAIL)", check_section6_sso_carl_still_member),
    ("S6 SSO memory_write persists (real turn)", check_section6_sso_memory_write_persists),
]


# ---------------------------------------------------------------------------
# Preflight + main
# ---------------------------------------------------------------------------

async def _preflight(client, owner):
    try:
        health = await client.get(f"{BASE_URL}/api/health", timeout=5)
    except Exception as exc:
        return f"cannot reach {BASE_URL} — is `serve` running? ({exc})"
    if health.status_code != 200:
        return f"{BASE_URL}/api/health -> {health.status_code}; start serve first"
    probe = await client.get(f"{API}/admin/users", headers=_bearer(owner), timeout=10)
    if probe.status_code == 404:
        return "/admin/users is 404 — serve was not built with --features capability-policy"
    if probe.status_code in (401, 403):
        return (
            f"owner bearer rejected ({probe.status_code}) — IRONCLAW_REBORN_WEBUI_TOKEN must "
            "match the running serve"
        )
    if probe.status_code != 200:
        return f"/admin/users preflight -> {probe.status_code} {probe.text}"
    users = probe.json().get("users", [])
    if users:
        return (
            f"the directory is not empty ({len(users)} users) — drop the local-dev db for a "
            "clean run (see run_xyzorg.sh)"
        )
    return None


async def main() -> int:
    owner = _read_env_value("IRONCLAW_REBORN_WEBUI_TOKEN")
    if not owner:
        sys.stderr.write(
            "ERROR: no owner bearer. Set IRONCLAW_REBORN_WEBUI_TOKEN or put it in the repo .env\n"
        )
        return 2

    print(f"xyzorg capability-policy validator -> {BASE_URL}")
    async with httpx.AsyncClient() as client:
        problem = await _preflight(client, owner)
        if problem:
            sys.stderr.write(f"PREFLIGHT FAILED: {problem}\n")
            return 2

        print("Bootstrapping org (SECTION 1 + 2)...")
        try:
            state = await bootstrap_org(client, owner)
        except Exception as exc:  # noqa: BLE001 - report and stop; nothing downstream can run
            sys.stderr.write(f"BOOTSTRAP FAILED: {exc!r}\n")
            return 1

        print("Running checks:")
        passed = failed = skipped = 0
        failures = []
        for name, check in CHECKS:
            try:
                await check(client, state)
            except Skip as exc:
                skipped += 1
                print(f"  SKIP  {name} — {exc}")
            except AssertionError as exc:
                failed += 1
                failures.append((name, str(exc)))
                print(f"  FAIL  {name}\n        {exc}")
            except Exception as exc:  # noqa: BLE001 - surface any unexpected error per check
                failed += 1
                failures.append((name, repr(exc)))
                print(f"  ERROR {name}\n        {exc!r}")
            else:
                passed += 1
                print(f"  PASS  {name}")

    print(f"\n{passed} passed, {failed} failed, {skipped} skipped")
    if failures:
        print("Failures:")
        for name, detail in failures:
            print(f"  - {name}: {detail}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
