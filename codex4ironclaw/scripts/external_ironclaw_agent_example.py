#!/usr/bin/env python3
"""Dedicated example for an IronClaw agent connecting from outside Docker."""

from ironclaw_agent_example import run_agent_cli


def main() -> None:
    run_agent_cli(
        name="external_ironclaw_agent_example",
        default_ws_url="ws://127.0.0.1:9090/ws/agent",
    )


if __name__ == "__main__":
    main()
