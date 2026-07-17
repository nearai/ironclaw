import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.live_canary import common  # noqa: E402


def test_cargo_build_and_gateway_launch_target_the_same_legacy_binary(
    monkeypatch,
) -> None:
    build_commands: list[list[str]] = []
    monkeypatch.setattr(
        common,
        "run",
        lambda command, **_kwargs: build_commands.append(command),
    )

    common.cargo_build()

    build_command = build_commands[0]
    built_binary = build_command[build_command.index("--bin") + 1]
    launched_binary = Path(common.legacy_gateway_command()[0]).name
    assert built_binary == launched_binary == "ironclaw-v1"
    assert common.legacy_gateway_command()[1:] == ["--no-onboard"]
