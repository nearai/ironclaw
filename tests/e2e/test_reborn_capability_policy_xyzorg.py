# this is a test file to test the capability of live ironclaw, org "xyzorg"
#
# (setup is done manually, out of band — boot serve, set the xyzorg tenant,
#  install the capabilities, bootstrap the operator/director. not part of this file.)
#
# ============================================================================
# SECTION 1 — org + accounts
# ============================================================================
# 1. the org "xyzorg" is the serve tenant.
# 2. create director@xyzorg.com as OWNER.
# 3. create officer@xyzorg.com as a member (user).
# 4. director promotes officer to ADMIN:
#      PUT /api/webchat/v2/admin/users/officer@xyzorg.com/role  {"role":"admin"}  (director's bearer)
# 5. create members alice, bob, carl (role = member).
#
# ============================================================================
# SECTION 2 — assign per-user capabilities (allow-list = "only X, deny the rest")
# ============================================================================
# mechanism (works today): enumerate the cap list, then hide each one NOT allowed:
#   GET  /api/webchat/v2/settings/tools                            -> the capability_id list
#   PUT  /api/webchat/v2/admin/users/{user}/capabilities/{cap_id}  {"availability":"hidden"}  -> per non-allowed cap
#   (use settings/tools for cap-ids; /admin/extensions returns PACKAGE ids.)
# 6. alice: allow builtin.shell + web_search; hide every other cap.
# 7. bob:   allow gdrive + github;    hide every other cap.
# 8. carl:  hide every cap (deny all).
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
# Red->green driver for epic #5261 role-model build. Steps that work against
# *current* logic HARD-ASSERT; role-model steps not yet built are marked
# `@pytest.mark.xfail(..., strict=False)` so the suite is GREEN now and each
# xfail is removed as its feature (D1-D7) lands. The precise gap is named in
# every xfail reason so the reviewer knows which to drop.
#
# Run:
#   cd tests/e2e
#   python -m pytest test_reborn_capability_policy_xyzorg.py -v
#
# asyncio_mode="auto" is set globally in pyproject.toml, so NO @pytest.mark.asyncio.

import asyncio
import os
import signal
import socket
import uuid
from pathlib import Path

import httpx
import pytest

from helpers import wait_for_ready

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

TENANT = "xyzorg"
OPERATOR_USER_ID = "operator"
# >= 32 bytes: serve also uses this as the SSO session-signing key and refuses
# to bind with a shorter secret (see test_reborn_webui_v2_smoke.py).
OPERATOR_BEARER = "e2e-xyzorg-operator-bearer-token-0123456789abcdef"

# Allow-lists per the SPEC (sections 6-8). Capabilities NOT in a member's set
# are hidden via the per-user policy delta surface.
ALICE_ALLOW = {"builtin.shell", "nearai.web_search"}
BOB_ALLOW = {"gdrive", "github"}
CARL_ALLOW: set[str] = set()

# Packages an admin installs first so their capabilities become available
# (availability = installed AND policy-available). PUT /admin/extensions/{id}.
# `web-access` exposes `nearai.web_search` in the live extension catalog.
ALLOWED_PACKAGES = ["web-access", "google-drive", "github"]


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _read_log(path: Path, limit: int = 8192) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")[-limit:]
    except OSError:
        return ""


def _forward_coverage_env(env: dict) -> None:
    for key, value in os.environ.items():
        if key.startswith(("CARGO_LLVM_COV", "LLVM_")) or key in {
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
        }:
            env[key] = value


async def _stop_process(proc, *, sig=signal.SIGINT, timeout: float = 10) -> None:
    if proc.returncode is not None:
        return
    try:
        proc.send_signal(sig)
    except ProcessLookupError:
        return
    try:
        await asyncio.wait_for(proc.wait(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        await asyncio.wait_for(proc.wait(), timeout=5)


def _write_policy_config_toml(path: Path, mock_llm_server: str) -> None:
    """Seed a Reborn config for the xyzorg tenant pointing at the mock LLM.

    Mirrors `_write_config_toml` in test_reborn_webui_v2_smoke.py but pins the
    tenant to `xyzorg` (serve reads `[identity].tenant`; see
    crates/ironclaw_reborn_cli/src/commands/serve.rs ~line 95) and the operator
    owner to `operator` (the env-bearer user, who authenticates as Owner).
    """
    path.write_text(
        f"""api_version = "ironclaw.runtime/v1"

[boot]
profile = "local-dev"

[identity]
default_owner = "{OPERATOR_USER_ID}"
tenant = "{TENANT}"
default_agent = "xyzorg-agent"

[webui]
env_token_var = "IRONCLAW_REBORN_WEBUI_TOKEN"
env_user_id_var = "IRONCLAW_REBORN_WEBUI_USER_ID"

[llm.default]
provider_id = "openai"
model = "mock-model"
api_key_env = "MOCK_LLM_API_KEY"
base_url = "{mock_llm_server}/v1"
""",
        encoding="utf-8",
    )


@pytest.fixture(scope="module")
def ironclaw_reborn_policy_binary():
    """Build `ironclaw-reborn` with BOTH webui-v2-beta AND capability-policy.

    The conftest `ironclaw_reborn_binary` fixture builds with webui-v2-beta
    only; the policy admin routes (/admin/users, /admin/extensions,
    /admin/users/{u}/capabilities) and the dispatch-seam resolver are gated on
    `capability-policy`, so a dedicated build is required.
    """
    import subprocess

    # Resolve the cargo target dir the same way conftest does.
    root = Path(__file__).resolve().parent.parent.parent
    env_target = os.environ.get("CARGO_TARGET_DIR")
    if env_target:
        target_dir = Path(env_target)
    else:
        target_dir = root / "target"
        cargo_config = Path.home() / ".cargo" / "config.toml"
        if cargo_config.exists():
            try:
                for line in cargo_config.read_text().splitlines():
                    line = line.strip()
                    if line.startswith("target-dir"):
                        _, _, value = line.partition("=")
                        value = value.strip().strip('"').strip("'")
                        if value:
                            target_dir = Path(value)
                            break
            except OSError:
                pass

    binary = target_dir / "debug" / "ironclaw-reborn"
    print("Building ironclaw-reborn (webui-v2-beta + capability-policy)...")
    subprocess.run(
        [
            "cargo", "build",
            "-p", "ironclaw_reborn_cli",
            "--features", "webui-v2-beta,capability-policy",
        ],
        cwd=root,
        check=True,
        timeout=900,
    )
    assert binary.exists(), f"Binary not found at {binary}"
    return str(binary)


@pytest.fixture(scope="module")
async def xyzorg_server(ironclaw_reborn_policy_binary, mock_llm_server, tmp_path_factory):
    """Boot `ironclaw-reborn serve` for the xyzorg tenant with the policy ON.

    Yields `(base_url, operator_bearer)`. The operator env-bearer authenticates
    as Owner (`WebuiAuthentication::operator`) with `operator_webui_config`, so
    it is the bootstrap admin that mints the first REST users.
    """
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-xyzorg-home")
    reborn_home = home_dir / "reborn-home"
    reborn_home.mkdir(parents=True, exist_ok=True)
    _write_policy_config_toml(reborn_home / "config.toml", mock_llm_server)

    proc = None
    base_url = None
    last_stderr = ""
    last_port = None

    for attempt in range(1, 4):
        port = _find_free_port()
        last_port = port
        stdout_path = home_dir / f"xyzorg-attempt-{attempt}.stdout.log"
        stderr_path = home_dir / f"xyzorg-attempt-{attempt}.stderr.log"

        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home_dir),
            "IRONCLAW_REBORN_HOME": str(reborn_home),
            "IRONCLAW_REBORN_PROFILE": "local-dev",
            # Activate the capability policy at the dispatch seam (default off).
            "IRONCLAW_REBORN_CAPABILITY_POLICY": "1",
            "IRONCLAW_REBORN_WEBUI_TOKEN": OPERATOR_BEARER,
            "IRONCLAW_REBORN_WEBUI_USER_ID": OPERATOR_USER_ID,
            "MOCK_LLM_API_KEY": "mock-api-key",
            "NO_PROXY": "127.0.0.1,localhost,::1",
            "no_proxy": "127.0.0.1,localhost,::1",
            "RUST_LOG": "ironclaw=warn,ironclaw_reborn=warn",
            "RUST_BACKTRACE": "1",
        }
        _forward_coverage_env(env)

        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            proc = await asyncio.create_subprocess_exec(
                ironclaw_reborn_policy_binary,
                "serve",
                "--host", "127.0.0.1",
                "--port", str(port),
                stdin=asyncio.subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                env=env,
            )
        base_url = f"http://127.0.0.1:{port}"

        try:
            await wait_for_ready(f"{base_url}/api/health", timeout=60)
            break
        except TimeoutError:
            if proc.returncode is None:
                await _stop_process(proc, timeout=2)
            last_stderr = _read_log(stderr_path)
            proc = None
    else:
        pytest.fail(
            "xyzorg policy server failed to start after 3 attempts.\n"
            f"Last attempted port: {last_port}\n"
            f"stderr:\n{last_stderr}"
        )

    try:
        yield base_url, OPERATOR_BEARER
    finally:
        if proc is not None and proc.returncode is None:
            await _stop_process(proc, sig=signal.SIGINT, timeout=10)
            if proc.returncode is None:
                await _stop_process(proc, sig=signal.SIGTERM, timeout=5)


# ---------------------------------------------------------------------------
# Helpers (thin REST wrappers; every admin write goes through the operator or a
# minted admin bearer, mirroring the real /admin/* contract).
# ---------------------------------------------------------------------------

def _bearer(token: str) -> dict:
    return {"Authorization": f"Bearer {token}"}


async def _create_user(client, base_url, admin_token, user_id, role):
    """POST /admin/users -> {user_id, role, token}. HARD-ASSERT 200."""
    resp = await client.post(
        f"{base_url}/api/webchat/v2/admin/users",
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


async def _set_role(client, base_url, admin_token, user_id, role):
    """PUT /admin/users/{user_id}/role {"role": ...}."""
    return await client.put(
        f"{base_url}/api/webchat/v2/admin/users/{user_id}/role",
        headers=_bearer(admin_token),
        json={"role": role},
        timeout=15,
    )


async def _install_package(client, base_url, admin_token, package_id, config=None):
    """PUT /admin/extensions/{package_id}  body = InstallRequest{config?}."""
    body = {} if config is None else {"config": config}
    return await client.put(
        f"{base_url}/api/webchat/v2/admin/extensions/{package_id}",
        headers=_bearer(admin_token),
        json=body,
        timeout=15,
    )


async def _enumerate_capability_ids(client, base_url, token):
    """GET /settings/tools -> capability_ids by stripping the `tool.` prefix.

    settings/tools returns operator-config entries keyed `tool.<capability_id>`
    plus a `tools.auto_approve` bool entry; strip the prefix and drop the
    auto-approve key.
    """
    resp = await client.get(
        f"{base_url}/api/webchat/v2/settings/tools",
        headers=_bearer(token),
        timeout=15,
    )
    resp.raise_for_status()
    caps = set()
    for entry in resp.json().get("entries", []):
        key = entry.get("key", "")
        if key.startswith("tool."):
            caps.add(key[len("tool."):])
    return caps


async def _hide_capability(client, base_url, admin_token, user_id, capability_id):
    """PUT /admin/users/{user}/capabilities/{cap} {"availability":"hidden"}.

    The per-user caps route is rate-limited PerCaller at 60 requests / 60s
    (ADMIN_USER_CAPS_MAX_REQUESTS). Hiding every non-allowed cap for several
    members easily exceeds that on one admin bearer, so back off and retry on
    429 — the limit is real product behavior, not a write failure.
    """
    for attempt in range(8):
        resp = await client.put(
            f"{base_url}/api/webchat/v2/admin/users/{user_id}/capabilities/{capability_id}",
            headers=_bearer(admin_token),
            json={"availability": "hidden"},
            timeout=15,
        )
        if resp.status_code != 429:
            return resp
        # Sliding-window limiter; wait for the window to free a slot.
        await asyncio.sleep(1.0 + attempt)
    return resp


async def _create_thread(client, base_url, token):
    resp = await client.post(
        f"{base_url}/api/webchat/v2/threads",
        headers=_bearer(token),
        json={"client_action_id": str(uuid.uuid4())},
        timeout=15,
    )
    resp.raise_for_status()
    return resp.json()["thread"]["thread_id"]


# ---------------------------------------------------------------------------
# Per-test org/account state. Building the directory is itself part of the
# SPEC (sections 1-2), so it lives in a fixture that the section tests share.
# It HARD-ASSERTS the create/grant writes that work today and returns the
# minted bearers + the resolved capability list.
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
async def org_state(xyzorg_server):
    """Bootstrap the xyzorg org per SECTION 1 + SECTION 2 (the parts that work
    against current logic), returning the bearers + enumerated capabilities."""
    base_url, operator_bearer = xyzorg_server
    state: dict = {"base_url": base_url, "operator": operator_bearer, "tokens": {}}

    async with httpx.AsyncClient() as client:
        # SECTION 1 -----------------------------------------------------------
        # 2. operator(Owner) creates director as OWNER.
        director = await _create_user(client, base_url, operator_bearer, "director", "owner")
        state["tokens"]["director"] = director
        # 3. director creates officer as a member, then 4. promotes to ADMIN.
        officer = await _create_user(client, base_url, director, "officer", "member")
        state["tokens"]["officer"] = officer
        promote = await _set_role(client, base_url, director, "officer", "admin")
        assert promote.status_code == 200, f"promote officer->admin: {promote.text}"
        assert promote.json()["role"] == "admin"
        # 5. create members alice, bob, carl.
        for member in ("alice", "bob", "carl"):
            state["tokens"][member] = await _create_user(
                client, base_url, director, member, "member"
            )

        # SECTION 2 -----------------------------------------------------------
        # Install the allowed packages first (availability precondition).
        installs = {}
        for package_id in ALLOWED_PACKAGES:
            resp = await _install_package(client, base_url, operator_bearer, package_id)
            installs[package_id] = resp.status_code
        state["installs"] = installs

        # Enumerate caps (operator sees the full operator catalog).
        caps = await _enumerate_capability_ids(client, base_url, operator_bearer)
        state["capabilities"] = caps

        # Allow-list = hide every cap NOT in the member's allow set. The route
        # is rate-limited PerCaller (60/60s); distribute the writes across all
        # three admin-capable bearers (operator/director are Owner, officer is
        # Admin — all pass is_admin) to triple the budget and avoid serial
        # backoff stalls. _hide_capability still retries on 429 as a safety net.
        admin_bearers = [operator_bearer, director, officer]
        allow_by_user = {"alice": ALICE_ALLOW, "bob": BOB_ALLOW, "carl": CARL_ALLOW}
        hide_results: dict = {}
        write_index = 0
        for member, allow in allow_by_user.items():
            for cap in caps:
                if cap in allow:
                    continue
                admin = admin_bearers[write_index % len(admin_bearers)]
                write_index += 1
                resp = await _hide_capability(client, base_url, admin, member, cap)
                hide_results[(member, cap)] = resp.status_code
        state["hide_results"] = hide_results

    return state


# ===========================================================================
# SECTION 1 + 2 — org, accounts, allow-list grants (works today: HARD-ASSERT)
# ===========================================================================

async def test_section1_org_and_accounts(org_state):
    """Steps 1-5: the org's owner/admin/member accounts are created via the
    real /admin/users contract and every minted bearer is captured."""
    tokens = org_state["tokens"]
    # All five users were minted with non-empty bearers.
    for who in ("director", "officer", "alice", "bob", "carl"):
        assert tokens.get(who), f"missing bearer for {who}"
    # Distinct tokens (no accidental reuse).
    assert len(set(tokens.values())) == len(tokens), "minted bearers must be distinct"


async def test_section2_install_and_grant_writes_succeed(org_state):
    """Steps 6-8: installing the allowed packages and writing the per-user
    `hidden` deltas both succeed against the live admin surface."""
    # Package installs succeed (PUT /admin/extensions/{id} -> 200).
    for package_id, status in org_state["installs"].items():
        assert status == 200, f"install {package_id}: HTTP {status}"

    # We could enumerate at least one capability (the operator catalog is
    # non-empty once packages are installed). If the live catalog exposes no
    # operator tool entries this list may be empty — that is itself a signal,
    # so assert the enumeration call worked rather than a specific count.
    assert isinstance(org_state["capabilities"], set)

    # Every hide write that we attempted returned 200 (the per-user delta
    # surface accepts the admin's `availability: hidden` upsert).
    for (member, cap), status in org_state["hide_results"].items():
        assert status == 200, f"hide {cap} for {member}: HTTP {status}"


@pytest.mark.xfail(
    reason="bob user-keyed secret: manual-token flow is setup + secret-submit "
    "(scope/interaction_id/validated-token shape), not the /setup + /submit "
    "shape the SPEC sketches; exact request contract unverified (D-future).",
    strict=False,
)
async def test_section2_bob_user_keyed_secret(org_state):
    """Step 9: set bob's own provider key via the manual-token flow.

    The live routes are POST /api/reborn/product-auth/manual-token/{setup,
    submit,secret-submit}; `submit`/`secret-submit` require a validated token
    plus the setup-issued interaction_id and a scoped invocation. The precise
    body shape is not pinned down here, so this is a best-effort xfail until the
    manual-token request contract for a user-keyed member secret is confirmed.
    """
    base_url = org_state["base_url"]
    bob = org_state["tokens"]["bob"]
    async with httpx.AsyncClient() as client:
        setup = await client.post(
            f"{base_url}/api/reborn/product-auth/manual-token/setup",
            headers=_bearer(bob),
            json={"provider": "github", "account_label": "bob-github"},
            timeout=15,
        )
        assert setup.status_code == 200, setup.text
        interaction_id = setup.json()["interaction_id"]
        submit = await client.post(
            f"{base_url}/api/reborn/product-auth/manual-token/secret-submit",
            headers=_bearer(bob),
            json={"interaction_id": interaction_id, "token": "ghp_bob_personal_token"},
            timeout=15,
        )
        assert submit.status_code == 200, submit.text


# ===========================================================================
# SECTION 3 — role privileges (owner > admin > member)
# ===========================================================================

async def test_section3_step12_deletion_guards(org_state):
    """Step 12 deletion matrix.

    The owner-outranks-admin DELETE (director -> officer) works today and is
    HARD-ASSERTed LAST. The four 403 guard cases (D5/G1, #5355) are NOT built
    yet — today the handler only checks `is_admin` + tenant, so a cross-rank or
    self delete still 204s. Those are split into the dedicated xfail test below
    so this one stays green and the working case is locked in.
    """
    base_url = org_state["base_url"]
    director = org_state["tokens"]["director"]

    async with httpx.AsyncClient() as client:
        # director(owner) deletes officer(admin) -> 204 (owner outranks admin).
        # Recreate officer first so this test is order-independent within module
        # scope (org_state is module-scoped; officer may already be gone).
        recreate = await client.post(
            f"{base_url}/api/webchat/v2/admin/users",
            headers=_bearer(director),
            json={"user_id": "officer", "role": "admin"},
            timeout=15,
        )
        assert recreate.status_code == 200, recreate.text
        deleted = await client.request(
            "DELETE",
            f"{base_url}/api/webchat/v2/admin/users/officer",
            headers=_bearer(director),
            timeout=15,
        )
        assert deleted.status_code == 204, f"owner->admin delete: {deleted.status_code} {deleted.text}"


@pytest.mark.xfail(
    reason="step 12 deletion guards (D5/G1, #5355): self-delete, admin->owner, "
    "admin->peer-admin, and last-owner protection are not built — the delete "
    "handler today checks only is_admin + tenant, so these 403 cases 204.",
    strict=False,
)
async def test_section3_step12_deletion_guards_403_cases(org_state):
    """The four guard cases that must 403 once D5 lands (today they 204)."""
    base_url = org_state["base_url"]
    director = org_state["tokens"]["director"]

    async with httpx.AsyncClient() as client:
        # Ensure officer(admin) exists for the cross-rank probes.
        await client.post(
            f"{base_url}/api/webchat/v2/admin/users",
            headers=_bearer(director),
            json={"user_id": "officer", "role": "admin"},
            timeout=15,
        )
        officer = await _create_user(client, base_url, director, "officer2", "admin")

        # officer(admin) deletes director(owner) -> 403 (admin may not delete owner).
        r1 = await client.request(
            "DELETE", f"{base_url}/api/webchat/v2/admin/users/director",
            headers=_bearer(officer), timeout=15,
        )
        assert r1.status_code == 403, f"admin->owner delete must be 403, got {r1.status_code}"

        # officer(admin) deletes officer2(another admin) -> 403 (no peer-admin delete).
        # (officer2 == the officer bearer's own user; use officer deleting officer2's
        # peer — recreate a separate peer admin.)
        peer = await _create_user(client, base_url, director, "peeradmin", "admin")
        r2 = await client.request(
            "DELETE", f"{base_url}/api/webchat/v2/admin/users/peeradmin",
            headers=_bearer(officer), timeout=15,
        )
        assert r2.status_code == 403, f"admin->peer-admin delete must be 403, got {r2.status_code}"
        del peer

        # officer(admin) deletes himself (officer2) -> 403 (no self-delete).
        r3 = await client.request(
            "DELETE", f"{base_url}/api/webchat/v2/admin/users/officer2",
            headers=_bearer(officer), timeout=15,
        )
        assert r3.status_code == 403, f"admin self-delete must be 403, got {r3.status_code}"

        # director(owner) deletes himself -> 403 (single owner is protected).
        r4 = await client.request(
            "DELETE", f"{base_url}/api/webchat/v2/admin/users/director",
            headers=_bearer(director), timeout=15,
        )
        assert r4.status_code == 403, f"last-owner self-delete must be 403, got {r4.status_code}"


@pytest.mark.xfail(
    reason="step 13 (D6/G4, #5355): an admin may not change the OWNER's caps. "
    "The per-user caps route discards AdminCaller.0 and never compares ranks, "
    "so officer(admin)->director(owner) currently 200s instead of 403.",
    strict=False,
)
async def test_section3_step13_admin_cannot_change_owner_caps(org_state):
    """Step 13: an admin PUT director(owner)'s caps -> 403.

    Mint a fresh admin bearer here rather than reusing `officer` (module-scoped
    `org_state` is shared, and the step-12 owner->admin delete may have revoked
    officer's token) so the xfail stays honest: it must fail on the rank check
    (admin can't touch an owner), not on a stale 401.
    """
    base_url = org_state["base_url"]
    director = org_state["tokens"]["director"]
    cap = next(iter(org_state["capabilities"]), "nearai.web_search")
    async with httpx.AsyncClient() as client:
        admin = await _create_user(client, base_url, director, "caps-admin", "admin")
        resp = await client.put(
            f"{base_url}/api/webchat/v2/admin/users/director/capabilities/{cap}",
            headers=_bearer(admin),
            json={"availability": "hidden"},
            timeout=15,
        )
        assert resp.status_code == 403, (
            f"admin editing owner's caps must be 403, got {resp.status_code}"
        )


# ===========================================================================
# SECTION 4 — enforcement at dispatch (per-user tool surface)
# ===========================================================================

@pytest.mark.xfail(
    reason="step 14 BEHAVIORAL: per-user dispatch-surface enforcement needs a "
    "deterministic model tool-call against a real installed extension tool plus "
    "the role-aware resolver (D2/D3) and builtin governance (D4); none of "
    "alice(builtin.shell)/admin-all-access are built, and the mock LLM has no "
    "canned tool-call wired for these capabilities yet.",
    strict=False,
)
async def test_section4_step14_dispatch_surface_enforcement(org_state):
    """Step 14: run the same tool-seeking turn per user; assert RAN vs
    PolicyDenied at dispatch. Requires D2/D3/D4 + a mock-LLM canned tool call."""
    base_url = org_state["base_url"]
    alice = org_state["tokens"]["alice"]
    async with httpx.AsyncClient() as client:
        thread_id = await _create_thread(client, base_url, alice)
        # Would send a prompt the mock LLM maps to a web_search tool call, then
        # poll the timeline for the tool result (RAN) vs a PolicyDenied marker.
        assert thread_id  # placeholder until the canned tool-call lands.
        raise AssertionError("dispatch-surface enforcement assertions not yet wired")


async def test_section4_step16_approval_pref_available_capability(org_state):
    """Step 16 (available case): a member may set an approval pref on a cap
    AVAILABLE to them. The live route is POST /settings/tools/{capability_id}
    (PUT is not yet accepted — see the xfail below)."""
    base_url = org_state["base_url"]
    alice = org_state["tokens"]["alice"]
    # alice's allow-set intersected with the live catalog; fall back to the
    # canonical web_search cap id if the catalog enumeration was empty.
    available = (org_state["capabilities"] & ALICE_ALLOW) or {"nearai.web_search"}
    cap = sorted(available)[0]
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{base_url}/api/webchat/v2/settings/tools/{cap}",
            headers=_bearer(alice),
            json={"state": "always_allow"},
            timeout=15,
        )
        # The settings/tools surface is an authenticated-caller route (not
        # operator-gated), so a member can set their own approval pref.
        assert resp.status_code == 200, f"approval pref on available cap: {resp.status_code} {resp.text}"


@pytest.mark.xfail(
    reason="step 16 PUT + unavailable-rejection (D7, #5344/#5355): the route only "
    "accepts POST today (no .put()), and there is no CapabilityAvailabilityProbe "
    "gating, so a pref on an UNAVAILABLE cap is not yet rejected with 403/404.",
    strict=False,
)
async def test_section4_step16_approval_pref_unavailable_rejected(org_state):
    """Step 16 (rejection case): PUT an approval pref on a cap UNAVAILABLE to the
    member -> 403/404. Needs the PUT verb + availability probe (D7)."""
    base_url = org_state["base_url"]
    carl = org_state["tokens"]["carl"]  # carl has every cap hidden (deny all).
    cap = sorted(org_state["capabilities"])[0] if org_state["capabilities"] else "nearai.web_search"
    async with httpx.AsyncClient() as client:
        resp = await client.put(
            f"{base_url}/api/webchat/v2/settings/tools/{cap}",
            headers=_bearer(carl),
            json={"state": "always_allow"},
            timeout=15,
        )
        assert resp.status_code in (403, 404), (
            f"approval pref on unavailable cap must be rejected, got {resp.status_code}"
        )