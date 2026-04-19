#!/usr/bin/env python3
"""Dedicated example for an IronClaw agent running inside the Compose network."""

from ironclaw_agent_example import run_agent_cli


def main() -> None:
    run_agent_cli(
        name="internal_ironclaw_agent_example",
        default_ws_url="ws://ironclaw-worker:9090/ws/agent",
    )


if __name__ == "__main__":
    main()
