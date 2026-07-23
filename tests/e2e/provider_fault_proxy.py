"""Resettable fault-injection proxy for hermetic provider tests.

Emulate remains the source of provider state and response semantics. This
proxy adds transport and response failures that Emulate intentionally does not
model, while recording whether a request reached the provider.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
from collections.abc import Mapping
from dataclasses import asdict, dataclass, field
from typing import Literal

from aiohttp import ClientSession, ClientTimeout, web

FaultAction = Literal[
    "respond",
    "disconnect_before_forward",
    "disconnect_after_forward",
    "delay_before_forward",
]


@dataclass(frozen=True)
class ProviderFaultProfile:
    """One reusable provider failure, independent of a specific operation."""

    name: str
    action: FaultAction
    status: int = 500
    body: str = ""
    content_type: str = "application/json"
    headers: dict[str, str] = field(default_factory=dict)
    delay_seconds: float = 0.0

    def control_payload(
        self,
        *,
        method: str,
        path: str,
        count: int = 1,
    ) -> dict:
        return {
            **asdict(self),
            "method": method.upper(),
            "path": path,
            "count": count,
        }


def _json_error(status: int, message: str) -> str:
    return json.dumps({"message": message, "status": status})


PROVIDER_FAULT_PROFILES = {
    "http_400": ProviderFaultProfile(
        name="http_400",
        action="respond",
        status=400,
        body=_json_error(400, "Bad Request"),
    ),
    "http_401": ProviderFaultProfile(
        name="http_401",
        action="respond",
        status=401,
        body=_json_error(401, "Bad credentials"),
    ),
    "http_403": ProviderFaultProfile(
        name="http_403",
        action="respond",
        status=403,
        body=_json_error(403, "Resource not accessible"),
    ),
    "http_404": ProviderFaultProfile(
        name="http_404",
        action="respond",
        status=404,
        body=_json_error(404, "Not Found"),
    ),
    "http_409": ProviderFaultProfile(
        name="http_409",
        action="respond",
        status=409,
        body=_json_error(409, "Conflict"),
    ),
    "http_429": ProviderFaultProfile(
        name="http_429",
        action="respond",
        status=429,
        body=_json_error(429, "Rate limit exceeded"),
        headers={"Retry-After": "1"},
    ),
    "http_500": ProviderFaultProfile(
        name="http_500",
        action="respond",
        status=500,
        body=_json_error(500, "Internal Server Error"),
    ),
    "http_503": ProviderFaultProfile(
        name="http_503",
        action="respond",
        status=503,
        body=_json_error(503, "Service Unavailable"),
        headers={"Retry-After": "1"},
    ),
    "timeout": ProviderFaultProfile(
        name="timeout",
        action="delay_before_forward",
        delay_seconds=30.0,
    ),
    "connection_reset": ProviderFaultProfile(
        name="connection_reset",
        action="disconnect_before_forward",
    ),
    "malformed_json": ProviderFaultProfile(
        name="malformed_json",
        action="respond",
        status=200,
        body='{"incomplete":',
    ),
    "truncated_response": ProviderFaultProfile(
        name="truncated_response",
        action="respond",
        status=200,
        body='{"id":"provider-object"',
    ),
    "missing_field": ProviderFaultProfile(
        name="missing_field",
        action="respond",
        status=200,
        body="{}",
    ),
    "lost_acknowledgement": ProviderFaultProfile(
        name="lost_acknowledgement",
        action="disconnect_after_forward",
    ),
}


class ProviderFaultProxy:
    """Transparent provider proxy with resettable FIFO fault rules."""

    def __init__(self, upstream_url: str, *, service: str = "provider") -> None:
        self.upstream_url = upstream_url.rstrip("/")
        self.service = service
        self.url = ""
        self._runner: web.AppRunner | None = None
        self._session: ClientSession | None = None
        self._rules: list[dict] = []
        self._requests: list[dict] = []

    async def start(self) -> None:
        self._session = ClientSession(
            timeout=ClientTimeout(total=None),
            auto_decompress=False,
        )
        try:
            app = web.Application()
            app.router.add_post("/__mock/provider_faults", self._arm_http)
            app.router.add_get("/__mock/provider_faults", self._state_http)
            app.router.add_post("/__mock/provider_faults/reset", self._reset_http)
            app.router.add_route("*", "/{path:.*}", self._proxy)
            self._runner = web.AppRunner(app)
            await self._runner.setup()
            site = web.TCPSite(self._runner, "127.0.0.1", 0)
            await site.start()
            server = site._server
            if server is None or not server.sockets:
                raise RuntimeError("provider fault proxy did not expose a socket")
            self.url = f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"
        except BaseException:
            await self.close()
            raise

    async def close(self) -> None:
        if self._runner is not None:
            await self._runner.cleanup()
            self._runner = None
        if self._session is not None:
            await self._session.close()
            self._session = None

    def arm(
        self,
        profile: ProviderFaultProfile,
        *,
        method: str,
        path: str,
        count: int = 1,
    ) -> None:
        if count < 1:
            raise ValueError("provider fault count must be at least one")
        if not path.startswith("/"):
            raise ValueError("provider fault path must start with '/'")
        self._rules.append(
            profile.control_payload(method=method, path=path, count=count)
        )

    def reset(self) -> None:
        self._rules.clear()
        self._requests.clear()

    @property
    def state(self) -> dict:
        return {
            "rules": [dict(rule) for rule in self._rules],
            "requests": [dict(request) for request in self._requests],
        }

    async def _arm_http(self, request: web.Request) -> web.Response:
        payload = await request.json()
        profile_name = payload.pop("profile", None)
        if not isinstance(profile_name, str):
            raise web.HTTPBadRequest(text="provider fault profile is required")
        try:
            profile = PROVIDER_FAULT_PROFILES[profile_name]
        except KeyError as exc:
            raise web.HTTPBadRequest(
                text=f"unknown provider fault profile: {profile_name}"
            ) from exc
        try:
            self.arm(profile, **payload)
        except (TypeError, ValueError) as exc:
            raise web.HTTPBadRequest(text=str(exc)) from exc
        return web.json_response(self.state)

    async def _state_http(self, _request: web.Request) -> web.Response:
        return web.json_response(self.state)

    async def _reset_http(self, _request: web.Request) -> web.Response:
        self.reset()
        return web.json_response({"ok": True})

    def _take_rule(self, method: str, path: str) -> dict | None:
        for index, rule in enumerate(self._rules):
            if rule["method"] != method or path != rule["path"]:
                continue
            selected = dict(rule)
            rule["count"] -= 1
            if rule["count"] == 0:
                self._rules.pop(index)
            return selected
        return None

    @staticmethod
    def _credential_fingerprint(headers: Mapping[str, str]) -> str | None:
        authorization = headers.get("Authorization")
        if authorization is None:
            return None
        return hashlib.sha256(authorization.encode()).hexdigest()[:12]

    async def _proxy(self, request: web.Request) -> web.StreamResponse:
        body = await request.read()
        rule = self._take_rule(request.method, request.path)
        entry = {
            "service": self.service,
            "method": request.method,
            "path": request.path,
            "query": request.query_string,
            "credential_fingerprint": self._credential_fingerprint(request.headers),
            "body_sha256": hashlib.sha256(body).hexdigest(),
            "fault": None if rule is None else rule["name"],
            "forwarded": False,
            "upstream_status": None,
            "responded": False,
        }
        self._requests.append(entry)

        if rule is not None:
            action = rule["action"]
            if action == "delay_before_forward":
                await asyncio.sleep(float(rule["delay_seconds"]))
            elif action == "disconnect_before_forward":
                request.transport.abort()
                raise asyncio.CancelledError
            elif action == "respond":
                entry["responded"] = True
                return web.Response(
                    status=int(rule["status"]),
                    text=rule["body"],
                    content_type=rule["content_type"],
                    headers=rule["headers"],
                )

        upstream = await self._forward(request, body)
        entry["forwarded"] = True
        entry["upstream_status"] = upstream.status

        if rule is not None and rule["action"] == "disconnect_after_forward":
            request.transport.abort()
            raise asyncio.CancelledError

        entry["responded"] = True
        return upstream

    async def _forward(self, request: web.Request, body: bytes) -> web.Response:
        if self._session is None:
            raise RuntimeError("provider fault proxy is not started")
        headers = {
            name: value
            for name, value in request.headers.items()
            if name.lower() not in {"host", "content-length", "transfer-encoding"}
        }
        target = f"{self.upstream_url}{request.rel_url}"
        async with self._session.request(
            request.method,
            target,
            headers=headers,
            data=body,
            allow_redirects=False,
        ) as response:
            response_body = await response.read()
            response_headers = {
                name: value
                for name, value in response.headers.items()
                if name.lower()
                not in {
                    "content-length",
                    "content-encoding",
                    "transfer-encoding",
                    "connection",
                }
            }
            return web.Response(
                status=response.status,
                body=response_body,
                headers=response_headers,
            )


class ProviderFaultProxyWorld:
    """One independently resettable proxy per Emulate provider."""

    def __init__(self, upstream_urls: dict[str, str]) -> None:
        self.proxies = {
            service: ProviderFaultProxy(upstream_url, service=service)
            for service, upstream_url in upstream_urls.items()
        }

    @property
    def servers(self) -> dict[str, dict[str, str]]:
        return {
            service: {"url": proxy.url}
            for service, proxy in self.proxies.items()
        }

    async def start(self) -> None:
        started = []
        try:
            for proxy in self.proxies.values():
                await proxy.start()
                started.append(proxy)
        except BaseException:
            for proxy in reversed(started):
                await proxy.close()
            raise

    async def close(self) -> None:
        for proxy in reversed(tuple(self.proxies.values())):
            await proxy.close()

    def reset(self) -> None:
        for proxy in self.proxies.values():
            proxy.reset()
