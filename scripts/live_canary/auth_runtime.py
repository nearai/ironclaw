from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
import time
from typing import Any
from urllib.parse import parse_qs, quote, urlencode, urlparse

from scripts.live_canary.common import CanaryError, ProbeResult, api_request

ROOT = Path(__file__).resolve().parents[2]


def _extension_path(package_id: str, suffix: str = "") -> str:
    encoded = quote(package_id, safe="")
    return f"/api/webchat/v2/extensions/{encoded}{suffix}"


async def list_extensions(base_url: str, token: str) -> list[dict[str, Any]]:
    response = await api_request(
        "GET",
        base_url,
        "/api/webchat/v2/extensions",
        token=token,
        timeout=30,
    )
    response.raise_for_status()
    return response.json().get("extensions", [])


async def get_extension(
    base_url: str,
    token: str,
    package_id: str,
) -> dict[str, Any] | None:
    for extension in await list_extensions(base_url, token):
        package_ref = extension.get("package_ref") or {}
        if package_ref.get("id") == package_id:
            return extension
    return None


async def wait_for_extension_lifecycle(
    base_url: str,
    token: str,
    package_id: str,
    *,
    state: str | None = None,
    required_tools: tuple[str, ...] = (),
    timeout: float = 60.0,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last_observed: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        extension = await get_extension(base_url, token, package_id)
        if extension is not None:
            last_observed = extension
            if state is not None and extension.get("installation_state") != state:
                await _sleep()
                continue
            tools = set(extension.get("tools") or [])
            if not set(required_tools).issubset(tools):
                await _sleep()
                continue
            return extension
        await _sleep()
    if last_observed is None:
        observed = "extension never appeared in the installed-extension projection"
    else:
        observed = (
            f"last state={last_observed.get('installation_state')!r}, "
            f"tools={last_observed.get('tools') or []!r}"
        )
    raise CanaryError(
        f"Timed out waiting for extension {package_id!r}: expected "
        f"installation_state={state!r}, tools={list(required_tools)!r}; {observed}"
    )


async def install_extension(
    base_url: str,
    token: str,
    *,
    package_id: str,
    idempotency_key: str,
    timeout: float = 60.0,
) -> dict[str, Any]:
    response = await api_request(
        "POST",
        base_url,
        "/api/webchat/v2/extensions/install",
        token=token,
        json_body={
            "package_ref": {
                "kind": "extension",
                "id": package_id,
            },
            "idempotency_key": idempotency_key,
        },
        timeout=180,
    )
    if not 200 <= response.status_code < 300:
        raise CanaryError(
            f"Install failed for {package_id}: {response.status_code} {response.text}"
        )
    action = response.json()
    if action.get("success") is not True:
        raise CanaryError(
            f"Install action did not succeed for {package_id}: {action!r}"
        )
    return await wait_for_extension_lifecycle(
        base_url,
        token,
        package_id,
        timeout=timeout,
    )


def _validated_setup_projection(
    package_id: str,
    body: dict[str, Any],
) -> dict[str, Any]:
    package_ref = body.get("package_ref") or {}
    if package_ref.get("id") != package_id:
        raise CanaryError(
            f"Setup projection for {package_id} returned the wrong package: "
            f"{package_ref!r}"
        )
    phase = body.get("phase")
    if phase not in {"uninstalled", "setup_needed", "active"}:
        raise CanaryError(
            f"Setup projection for {package_id} returned an invalid phase: "
            f"{phase!r}"
        )
    if not isinstance(body.get("secrets", []), list):
        raise CanaryError(
            f"Setup projection for {package_id} returned invalid secrets: "
            f"{body.get('secrets')!r}"
        )
    return body


async def get_extension_setup(
    base_url: str,
    token: str,
    *,
    package_id: str,
) -> dict[str, Any]:
    response = await api_request(
        "GET",
        base_url,
        _extension_path(package_id, "/setup"),
        token=token,
        timeout=30,
    )
    if not 200 <= response.status_code < 300:
        raise CanaryError(
            f"Setup discovery failed for {package_id}: "
            f"{response.status_code} {response.text}"
        )
    return _validated_setup_projection(package_id, response.json())


def setup_requirements(
    setup: dict[str, Any],
    *,
    kind: str | None = None,
) -> list[dict[str, Any]]:
    requirements = list(setup.get("secrets") or [])
    if kind is None:
        return requirements
    return [
        requirement
        for requirement in requirements
        if (requirement.get("setup") or {}).get("kind") == kind
    ]


def _single_setup_requirement(
    setup: dict[str, Any],
    *,
    package_id: str,
    kind: str,
) -> dict[str, Any]:
    requirements = setup_requirements(setup, kind=kind)
    if len(requirements) != 1:
        raise CanaryError(
            f"Expected exactly one manifest-declared {kind!r} requirement "
            f"for {package_id}; got {requirements!r}"
        )
    return requirements[0]


async def complete_manual_token_setup(
    base_url: str,
    token: str,
    *,
    package_id: str,
    value: str,
    required_tools: tuple[str, ...] = (),
    timeout: float = 90.0,
) -> dict[str, Any]:
    setup = await get_extension_setup(base_url, token, package_id=package_id)
    requirement = _single_setup_requirement(
        setup,
        package_id=package_id,
        kind="manual_token",
    )
    requirement_name = requirement.get("name")
    if not isinstance(requirement_name, str) or not requirement_name:
        raise CanaryError(
            f"Manual-token requirement for {package_id} has no opaque name"
        )
    response = await api_request(
        "POST",
        base_url,
        _extension_path(package_id, "/setup"),
        token=token,
        json_body={
            "action": "submit",
            "payload": {
                "secrets": {requirement_name: value},
                "fields": {},
            },
        },
        timeout=30,
    )
    if not 200 <= response.status_code < 300:
        raise CanaryError(
            f"Manual-token setup failed for {package_id}: "
            f"{response.status_code} {response.text}"
        )
    returned = _validated_setup_projection(package_id, response.json())
    returned_requirement = next(
        (
            item
            for item in returned.get("secrets", [])
            if item.get("name") == requirement_name
        ),
        None,
    )
    if returned_requirement is None or returned_requirement.get("provided") is not True:
        raise CanaryError(
            f"Manual-token setup did not mark {requirement_name!r} provided "
            f"for {package_id}: {returned!r}"
        )
    return await wait_for_extension_lifecycle(
        base_url,
        token,
        package_id,
        state="active",
        required_tools=required_tools,
        timeout=timeout,
    )


async def start_oauth_setup(
    base_url: str,
    token: str,
    *,
    package_id: str,
    expires_at: str | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    setup = await get_extension_setup(base_url, token, package_id=package_id)
    requirement = _single_setup_requirement(
        setup,
        package_id=package_id,
        kind="oauth",
    )
    requirement_name = requirement.get("name")
    provider = requirement.get("provider")
    if not isinstance(requirement_name, str) or not requirement_name:
        raise CanaryError(f"OAuth requirement for {package_id} has no opaque name")
    if not isinstance(provider, str) or not provider:
        raise CanaryError(f"OAuth requirement for {package_id} has no provider")
    setup_descriptor = requirement.get("setup") or {}
    invocation_id = setup_descriptor.get("invocation_id")
    if not isinstance(invocation_id, str) or not invocation_id:
        raise CanaryError(
            f"OAuth requirement for {package_id} has no invocation scope"
        )
    expires_at = expires_at or (
        datetime.now(UTC) + timedelta(minutes=5)
    ).isoformat()
    response = await api_request(
        "POST",
        base_url,
        _extension_path(package_id, "/setup/oauth/start"),
        token=token,
        json_body={
            "requirement": requirement_name,
            "expires_at": expires_at,
            "invocation_id": invocation_id,
        },
        timeout=30,
    )
    if not 200 <= response.status_code < 300:
        raise CanaryError(
            f"OAuth start failed for {package_id}: "
            f"{response.status_code} {response.text}"
        )
    body = response.json()
    for field in ("authorization_url", "flow_id", "callback_scope"):
        if not body.get(field):
            raise CanaryError(
                f"OAuth start for {package_id} omitted {field}: {body!r}"
            )
    if body.get("status") != "awaiting_user":
        raise CanaryError(
            f"OAuth start for {package_id} returned an invalid status: "
            f"{body.get('status')!r}"
        )
    if body.get("provider") != provider:
        raise CanaryError(
            f"OAuth start for {package_id} returned provider "
            f"{body.get('provider')!r}, expected {provider!r}"
        )
    if (body.get("callback_scope") or {}).get("invocation_id") != invocation_id:
        raise CanaryError(
            f"OAuth start for {package_id} returned the wrong callback scope: "
            f"{body.get('callback_scope')!r}"
        )
    return requirement, body


async def complete_oauth_flow(
    base_url: str,
    token: str,
    *,
    package_id: str,
    code: str = "mock_auth_code",
    callback_params: dict[str, str] | None = None,
    required_tools: tuple[str, ...] = (),
    timeout: float = 90.0,
) -> dict[str, Any]:
    """Complete a manifest-declared OAuth recipe through the Reborn flow."""
    requirement, started = await start_oauth_setup(
        base_url,
        token,
        package_id=package_id,
    )
    auth_url = started["authorization_url"]
    state = parse_qs(urlparse(auth_url).query).get("state", [None])[0]
    if not state:
        raise CanaryError(f"auth_url missing state parameter: {auth_url}")
    provider = requirement["provider"]
    callback_query = {
        "state": state,
        "code": code,
        **(callback_params or {}),
    }
    callback_response = await api_request(
        "GET",
        base_url,
        (
            f"/api/reborn/product-auth/oauth/{quote(provider, safe='')}/callback?"
            f"{urlencode(callback_query)}"
        ),
        token=token,
        timeout=30,
    )
    if not 200 <= callback_response.status_code < 300:
        raise CanaryError(
            f"OAuth callback failed for {package_id}: "
            f"{callback_response.status_code} {callback_response.text[:500]}"
        )
    callback = callback_response.json()
    if (
        str(callback.get("flow_id")) != str(started["flow_id"])
        or callback.get("status") != "completed"
    ):
        raise CanaryError(
            f"OAuth callback did not complete the expected flow for "
            f"{package_id}: {callback!r}"
        )
    callback_scope = started["callback_scope"]
    invocation_id = callback_scope.get("invocation_id")
    if not invocation_id:
        raise CanaryError(
            f"OAuth start for {package_id} omitted callback invocation scope"
        )
    await wait_for_oauth_flow_completed(
        base_url,
        token,
        package_id=package_id,
        flow_id=str(started["flow_id"]),
        invocation_id=invocation_id,
        timeout=timeout,
    )
    await select_single_oauth_account(
        base_url,
        token,
        package_id=package_id,
        provider=provider,
        invocation_id=invocation_id,
    )
    return await wait_for_extension_lifecycle(
        base_url,
        token,
        package_id,
        state="active",
        required_tools=required_tools,
        timeout=timeout,
    )


async def wait_for_oauth_flow_completed(
    base_url: str,
    token: str,
    *,
    package_id: str,
    flow_id: str,
    invocation_id: str,
    timeout: float = 60.0,
) -> None:
    deadline = time.monotonic() + timeout
    last_body: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        flow_response = await api_request(
            "GET",
            base_url,
            (
                f"/api/reborn/product-auth/oauth/flow/"
                f"{quote(flow_id, safe='')}/status?"
                f"{urlencode({'invocation_id': invocation_id})}"
            ),
            token=token,
            timeout=30,
        )
        if not 200 <= flow_response.status_code < 300:
            raise CanaryError(
                f"OAuth flow status failed for {package_id}: "
                f"{flow_response.status_code} {flow_response.text}"
            )
        last_body = flow_response.json()
        status = last_body.get("status")
        if status == "completed":
            return
        if status in {"failed", "expired", "canceled"}:
            raise CanaryError(
                f"OAuth flow for {package_id} reached terminal status "
                f"{status!r}: {last_body!r}"
            )
        await _sleep()
    raise CanaryError(
        f"Timed out waiting for OAuth flow completion for {package_id}; "
        f"last status={last_body!r}"
    )


async def select_single_oauth_account(
    base_url: str,
    token: str,
    *,
    package_id: str,
    provider: str,
    invocation_id: str,
) -> None:
    account_context = {
        "provider": provider,
        "requester_extension": package_id,
        "invocation_id": invocation_id,
    }
    accounts_response = await api_request(
        "POST",
        base_url,
        "/api/reborn/product-auth/accounts/list",
        token=token,
        json_body=account_context,
        timeout=30,
    )
    if not 200 <= accounts_response.status_code < 300:
        raise CanaryError(
            f"OAuth account discovery failed for {package_id}: "
            f"{accounts_response.status_code} {accounts_response.text}"
        )
    accounts = accounts_response.json().get("accounts") or []
    if len(accounts) > 1:
        raise CanaryError(
            f"OAuth for {package_id} produced multiple selectable accounts; "
            "the canary cannot choose an identity implicitly"
        )
    if accounts:
        account = accounts[0]
        if account.get("provider") != provider:
            raise CanaryError(
                f"OAuth account for {package_id} returned provider "
                f"{account.get('provider')!r}, expected {provider!r}"
            )
        if account.get("status") != "configured":
            raise CanaryError(
                f"OAuth account for {package_id} was not configured: "
                f"{account!r}"
            )
        account_id = account.get("id")
        if not isinstance(account_id, str) or not account_id:
            raise CanaryError(
                f"OAuth account for {package_id} omitted its opaque id"
            )
        selected_response = await api_request(
            "POST",
            base_url,
            "/api/reborn/product-auth/accounts/select",
            token=token,
            json_body={
                **account_context,
                "account_id": account_id,
            },
            timeout=30,
        )
        if not 200 <= selected_response.status_code < 300:
            raise CanaryError(
                f"OAuth account selection failed for {package_id}: "
                f"{selected_response.status_code} {selected_response.text}"
            )
        selected = selected_response.json()
        if (
            selected.get("id") != account_id
            or selected.get("provider") != provider
            or selected.get("status") != "configured"
        ):
            raise CanaryError(
                f"OAuth account selection for {package_id} returned an "
                f"unexpected projection: {selected!r}"
            )


async def configure_admin_group(
    base_url: str,
    token: str,
    *,
    group_id: str,
    values: dict[str, str],
    idempotency_key: str,
) -> dict[str, Any]:
    listed = await api_request(
        "GET",
        base_url,
        "/api/webchat/v2/operator/extension-configuration",
        token=token,
        timeout=30,
    )
    if not 200 <= listed.status_code < 300:
        raise CanaryError(
            "Admin configuration discovery failed: "
            f"{listed.status_code} {listed.text}"
        )
    group = next(
        (
            item
            for item in listed.json().get("groups", [])
            if item.get("group_id") == group_id
        ),
        None,
    )
    if group is None:
        raise CanaryError(
            f"Admin configuration group {group_id!r} was not declared"
        )
    declared_handles = {
        field.get("handle")
        for field in group.get("fields", [])
        if isinstance(field.get("handle"), str)
    }
    unknown_handles = set(values) - declared_handles
    if unknown_handles:
        raise CanaryError(
            f"Admin configuration values for {group_id} contain undeclared "
            f"handles: {sorted(unknown_handles)!r}"
        )
    expected_revision = group.get("revision", 0)
    response = await api_request(
        "PUT",
        base_url,
        (
            "/api/webchat/v2/operator/extension-configuration/"
            f"{quote(group_id, safe='')}"
        ),
        token=token,
        json_body={
            "values": [
                {"handle": handle, "value": value}
                for handle, value in values.items()
            ],
            "expected_revision": expected_revision,
            "idempotency_key": idempotency_key,
        },
        timeout=30,
    )
    if not 200 <= response.status_code < 300:
        raise CanaryError(
            f"Admin configuration failed for {group_id}: "
            f"{response.status_code} {response.text}"
        )
    updated = response.json()
    if updated.get("group_id") != group_id:
        raise CanaryError(
            f"Admin configuration for {group_id} returned the wrong group: "
            f"{updated!r}"
        )
    revision = updated.get("revision")
    if not isinstance(revision, int) or revision < expected_revision:
        raise CanaryError(
            f"Admin configuration for {group_id} returned an invalid revision: "
            f"{revision!r}"
        )
    if updated.get("complete") is not True:
        raise CanaryError(
            f"Admin configuration for {group_id} remained incomplete: "
            f"{updated!r}"
        )
    provided_by_handle = {
        field.get("handle"): field.get("provided")
        for field in updated.get("fields", [])
        if isinstance(field, dict)
    }
    unprovided = [
        handle
        for handle in values
        if provided_by_handle.get(handle) is not True
    ]
    if unprovided:
        raise CanaryError(
            f"Admin configuration for {group_id} did not persist submitted "
            f"handles: {unprovided!r}"
        )
    return updated


async def create_responses_probe(
    *,
    base_url: str,
    token: str,
    provider: str,
    prompt: str,
    expected_tool_name: str,
    expected_text: str,
) -> ProbeResult:
    started = time.perf_counter()
    response = await api_request(
        "POST",
        base_url,
        "/v1/responses",
        token=token,
        json_body={"model": "default", "input": prompt},
        timeout=180,
    )
    latency_ms = int((time.perf_counter() - started) * 1000)
    if response.status_code != 200:
        return ProbeResult(
            provider=provider,
            mode="responses_api",
            success=False,
            latency_ms=latency_ms,
            details={"status_code": response.status_code, "body": response.text[:1000]},
        )

    body = response.json()
    tool_names = [item.get("name") for item in body.get("output", []) if item.get("type") == "function_call"]
    tool_outputs = [
        item.get("output", "")
        for item in body.get("output", [])
        if item.get("type") == "function_call_output"
    ]
    texts: list[str] = []
    for item in body.get("output", []):
        if item.get("type") != "message":
            continue
        for content in item.get("content", []):
            if content.get("type") == "output_text":
                texts.append(content.get("text", ""))
    response_text = "\n".join(texts)
    success = (
        body.get("status") == "completed"
        and expected_tool_name in tool_names
        and bool(tool_outputs)
        and expected_text in response_text
    )
    return ProbeResult(
        provider=provider,
        mode="responses_api",
        success=success,
        latency_ms=latency_ms,
        details={
            "status": body.get("status"),
            "tool_names": tool_names,
            "tool_outputs": tool_outputs,
            "response_text": response_text,
            "error": body.get("error"),
        },
    )


async def _sleep() -> None:
    import asyncio

    await asyncio.sleep(0.5)
