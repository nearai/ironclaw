"""IronClaw WebUI v2 live QA canary package."""

from __future__ import annotations

import os


# Keep existing local and repository-level live-QA configuration working while
# making the IronClaw-prefixed namespace authoritative inside the harness.
for _name, _value in tuple(os.environ.items()):
    if _name.startswith("REBORN_WEBUI_V2_"):
        os.environ.setdefault(
            f"IRONCLAW_WEBUI_V2_{_name.removeprefix('REBORN_WEBUI_V2_')}",
            _value,
        )
    elif _name.startswith("IRONCLAW_REBORN_"):
        os.environ.setdefault(
            f"IRONCLAW_{_name.removeprefix('IRONCLAW_REBORN_')}",
            _value,
        )
