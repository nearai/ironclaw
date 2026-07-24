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
    "delay_before_disconnect",
    "truncate_response",
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
        action="delay_before_disconnect",
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
        action="truncate_response",
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
        self._delayed_requests: set[asyncio.Task] = set()

    async def start(self) -> None:
        self._session = ClientSession(
            timeout=ClientTimeout(total=None),
            auto_decompress=False,
        )
        try:
            app = web.Application()
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
        delayed_requests = tuple(self._delayed_requests)
        for task in delayed_requests:
            task.cancel()
        if delayed_requests:
            await asyncio.gather(*delayed_requests, return_exceptions=True)
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
        for task in tuple(self._delayed_requests):
            task.cancel()
        self._rules.clear()
        self._requests.clear()

    @property
    def state(self) -> dict:
        return {
            "rules": [dict(rule) for rule in self._rules],
            "requests": [dict(request) for request in self._requests],
        }

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

    @staticmethod
    def _abort_transport(request: web.Request) -> None:
        transport = request.transport
        if transport is not None:
            transport.abort()

    async def _delay_then_disconnect(
        self,
        request: web.Request,
        delay_seconds: float,
    ) -> None:
        task = asyncio.current_task()
        if task is not None:
            self._delayed_requests.add(task)
        try:
            await asyncio.sleep(delay_seconds)
            self._abort_transport(request)
            raise asyncio.CancelledError
        finally:
            if task is not None:
                self._delayed_requests.discard(task)

    async def _truncate_response(
        self,
        request: web.Request,
        rule: dict,
    ) -> None:
        body = rule["body"].encode()
        response = web.StreamResponse(
            status=int(rule["status"]),
            headers={
                **rule["headers"],
                "Content-Type": rule["content_type"],
                "Content-Length": str(len(body) + 1),
            },
        )
        await response.prepare(request)
        await response.write(body)
        self._abort_transport(request)
        raise asyncio.CancelledError

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
            if action == "delay_before_disconnect":
                await self._delay_then_disconnect(
                    request,
                    float(rule["delay_seconds"]),
                )
            elif action == "disconnect_before_forward":
                self._abort_transport(request)
                raise asyncio.CancelledError
            elif action == "truncate_response":
                await self._truncate_response(request, rule)
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
            self._abort_transport(request)
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
