"""Bounded redaction for diagnostics persisted by the live QA harness."""

from __future__ import annotations

import json
import re
from collections.abc import Mapping, Sequence
from itertools import islice
from typing import Any


_SENSITIVE_KEY_NAMES = {
    "apikey",
    "authorization",
    "clientsecret",
    "code",
    "cookie",
    "idtoken",
    "oauthtoken",
    "password",
    "refreshtoken",
    "secret",
    "state",
    "token",
    "accesstoken",
    "privatecredentials",
    "privatekey",
    "signingcredentials",
    "signingkey",
}
_SENSITIVE_KEY_SUFFIXES = {
    "cookie",
    "credential",
    "credentials",
    "key",
    "password",
    "secret",
    "token",
}
_SENSITIVE_KEY_PATTERN = (
    r"(?:api[_-]?key|authorization|client[_-]?secret|code|cookie|id[_-]?token|"
    r"oauth[_-]?token|password|refresh[_-]?token|secret|state|token|access[_-]?token)"
)
_DOUBLE_QUOTED_SECRET = re.compile(
    rf'(?i)("{_SENSITIVE_KEY_PATTERN}"\s*:\s*")((?:\\.|[^"\\])*)(")'
)
_SINGLE_QUOTED_SECRET = re.compile(
    rf"(?i)('{_SENSITIVE_KEY_PATTERN}'\s*:\s*')((?:\\.|[^'\\])*)(')"
)
_ESCAPED_DOUBLE_QUOTED_SECRET = re.compile(
    rf'(?i)(\\"{_SENSITIVE_KEY_PATTERN}\\"\s*:\s*\\")(.+?)(\\"(?:[,}}]|$))'
)
_ASSIGNMENT_SECRET = re.compile(
    rf"(?i)(\b{_SENSITIVE_KEY_PATTERN}\s*=\s*)[^,\s)\]}}'\"]+"
)
_QUERY_SECRET = re.compile(
    rf"(?i)([?&]{_SENSITIVE_KEY_PATTERN}=)[^&#\s]+"
)
_AUTHORIZATION_HEADER = re.compile(
    r"(?i)(\bauthorization\s*:\s*)[^\r\n,;}]+"
)
_COOKIE_HEADER = re.compile(r"(?i)(\bcookie\s*:\s*)[^\r\n]+")
_BEARER_OR_BASIC = re.compile(r"(?i)\b(?:bearer|basic)\s+[^\s,;}]+")
_SLACK_TOKEN = re.compile(r"xox[baprs]-[A-Za-z0-9-]{10,}")
_GOOGLE_TOKEN = re.compile(r"ya29\.[A-Za-z0-9._-]{20,}")
_ANTHROPIC_KEY = re.compile(r"sk-ant-[A-Za-z0-9_-]{10,}")
_OPENAI_KEY = re.compile(r"sk-[A-Za-z0-9_-]{20,}")
# Slack entity IDs begin with one of these documented families and a digit.
# Requiring the second character to be numeric avoids matching normal prose.
_SLACK_ENTITY_ID = re.compile(r"\b[TCGDUWBASEF][0-9][A-Z0-9]{7,}\b")


def _text(value: object) -> str:
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    try:
        return str(value)
    except Exception as exc:  # diagnostics must never mask the original error
        return f"<UNPRINTABLE:{type(exc).__name__}>"


def _normalized_key(value: object) -> str:
    return re.sub(r"[^a-z0-9]", "", _text(value).lower())


def _is_sensitive_key(value: object) -> bool:
    raw = _text(value)
    normalized = _normalized_key(raw)
    if normalized in _SENSITIVE_KEY_NAMES:
        return True
    separated = [part for part in re.split(r"[^A-Za-z0-9]+", raw) if part]
    if len(separated) > 1 and separated[-1].lower() in _SENSITIVE_KEY_SUFFIXES:
        return True
    return re.search(
        r"(?:Secret|Token|Key|Cookie|Password|Credential|Credentials)$",
        raw,
    ) is not None


def _sanitize_text(value: str, literal_secrets: Sequence[str]) -> str:
    text = value
    for secret in literal_secrets:
        if secret:
            text = text.replace(secret, "<REDACTED>")

    # A diagnostic may itself be a JSON object encoded as a string. Parsing it
    # first handles escaped quotes correctly; malformed strings fall through to
    # the conservative regex passes below.
    stripped = text.strip()
    if stripped.startswith(("{", "[")):
        try:
            parsed = json.loads(stripped)
        except (json.JSONDecodeError, TypeError):
            parsed = None
        if isinstance(parsed, (dict, list)):
            return json.dumps(
                sanitize_diagnostic(parsed, literal_secrets=literal_secrets),
                sort_keys=True,
            )

    text = _DOUBLE_QUOTED_SECRET.sub(r"\1<REDACTED>\3", text)
    text = _SINGLE_QUOTED_SECRET.sub(r"\1<REDACTED>\3", text)
    text = _ESCAPED_DOUBLE_QUOTED_SECRET.sub(r"\1<REDACTED>\3", text)
    text = _AUTHORIZATION_HEADER.sub(r"\1<REDACTED>", text)
    text = _COOKIE_HEADER.sub(r"\1<REDACTED>", text)
    text = _QUERY_SECRET.sub(r"\1<REDACTED>", text)
    text = _ASSIGNMENT_SECRET.sub(r"\1<REDACTED>", text)
    text = _BEARER_OR_BASIC.sub("<REDACTED_AUTHORIZATION>", text)
    text = _SLACK_TOKEN.sub("<REDACTED_SLACK_TOKEN>", text)
    text = _GOOGLE_TOKEN.sub("<REDACTED_GOOGLE_TOKEN>", text)
    text = _ANTHROPIC_KEY.sub("<REDACTED_ANTHROPIC_KEY>", text)
    text = _OPENAI_KEY.sub("<REDACTED_OPENAI_KEY>", text)
    return _SLACK_ENTITY_ID.sub("<REDACTED_SLACK_ID>", text)


def sanitize_diagnostic(
    value: object,
    *,
    max_depth: int = 6,
    max_items: int = 100,
    max_string_chars: int = 4_000,
    literal_secrets: Sequence[str] = (),
) -> object:
    """Return a JSON-safe, cycle-aware, bounded and redacted diagnostic.

    Collection limits are applied before converting their members to strings,
    so an omitted value cannot execute an expensive or unsafe ``__str__``.
    """

    active: set[int] = set()

    def walk(item: object, depth: int) -> Any:
        if item is None or isinstance(item, (bool, int, float)):
            return item
        if depth > max_depth:
            return "<MAX_DEPTH>"
        if isinstance(item, (str, bytes)):
            sanitized = _sanitize_text(_text(item), literal_secrets)
            if len(sanitized) > max_string_chars:
                return sanitized[:max_string_chars] + "<TRUNCATED>"
            return sanitized

        if isinstance(item, Mapping):
            identity = id(item)
            if identity in active:
                return "<CYCLE>"
            active.add(identity)
            try:
                result: dict[str, object] = {}
                entries = list(islice(item.items(), max_items))
                for raw_key, raw_value in entries:
                    key = _sanitize_text(_text(raw_key), literal_secrets)
                    if _is_sensitive_key(raw_key):
                        result[key] = "<REDACTED>"
                    else:
                        result[key] = walk(raw_value, depth + 1)
                omitted = max(0, len(item) - len(entries))
                if omitted:
                    result["<TRUNCATED>"] = f"{omitted} item(s) omitted"
                return result
            finally:
                active.remove(identity)

        if isinstance(item, Sequence):
            identity = id(item)
            if identity in active:
                return "<CYCLE>"
            active.add(identity)
            try:
                length = len(item)
                values = [walk(item[index], depth + 1) for index in range(min(length, max_items))]
                if length > max_items:
                    values.append(f"<{length - max_items} item(s) omitted>")
                return values
            finally:
                active.remove(identity)

        return walk(_text(item), depth)

    return walk(value, 0)
