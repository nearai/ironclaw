"""Environment lookup helpers for Reborn WebUI v2 live QA."""

from __future__ import annotations

import os
import re

from scripts.live_canary.common import env_secret


def _env_value(name: str, extra_env: dict[str, str] | None = None) -> str | None:
    if extra_env and extra_env.get(name):
        return extra_env[name]
    return env_secret(name)


def _env_present(name: str, extra_env: dict[str, str] | None = None) -> bool:
    return bool(_env_value(name, extra_env))


def _first_env_value(
    names: list[str],
    extra_env: dict[str, str] | None = None,
) -> tuple[str, str] | None:
    for name in names:
        value = _env_value(name, extra_env)
        if value:
            return name, value
    return None


def _section_env_name(config_text: str, key: str, default: str) -> str:
    match = re.search(
        rf"^\s*{key}\s*=\s*\"([A-Za-z_][A-Za-z0-9_]*)\"",
        config_text,
        re.MULTILINE,
    )
    return match.group(1) if match else default


def _non_empty_env(name: str, default: str) -> str:
    value = os.environ.get(name)
    if value and value.strip():
        return value.strip()
    return default
