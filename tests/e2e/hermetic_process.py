"""Process-boundary controls shared by the deterministic E2E harnesses."""

from __future__ import annotations

import os
import sys
from collections.abc import Mapping, MutableMapping

_HERMETIC_PROCESS_ENV_KEYS = (
    "IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY",
    "IRONCLAW_HERMETIC_NETWORK_VIOLATIONS",
    "LD_PRELOAD",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_FORCE_FLAT_NAMESPACE",
)


def forward_hermetic_process_env(
    env: MutableMapping[str, str],
    source: Mapping[str, str] | None = None,
) -> None:
    """Forward only the syscall guard controls into a minimal child env."""
    source_env = os.environ if source is None else source
    for key in _HERMETIC_PROCESS_ENV_KEYS:
        value = source_env.get(key)
        if value is not None:
            env[key] = value

    guard_library = source_env.get("IRONCLAW_HERMETIC_NETWORK_GUARD_LIBRARY")
    if guard_library is None:
        return
    if sys.platform == "darwin":
        env.setdefault("DYLD_INSERT_LIBRARIES", guard_library)
        env.setdefault("DYLD_FORCE_FLAT_NAMESPACE", "1")
    elif sys.platform.startswith("linux"):
        env.setdefault("LD_PRELOAD", guard_library)
