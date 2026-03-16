"""CLI entry point — mirrors src/main.rs."""

from __future__ import annotations

import asyncio
import logging
import sys
from pathlib import Path

import typer
from rich.console import Console
from rich.logging import RichHandler

from ironclaw.app import IronclawApp
from ironclaw.config import Config

app = typer.Typer(
    name="ironclaw",
    help="IronClaw — secure personal AI assistant (LangGraph edition)",
    no_args_is_help=False,
)
console = Console()


def _setup_logging(debug: bool = False) -> None:
    level = logging.DEBUG if debug else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(message)s",
        handlers=[RichHandler(rich_tracebacks=True, show_path=False)],
    )
    # Quiet noisy libraries
    logging.getLogger("httpx").setLevel(logging.WARNING)
    logging.getLogger("httpcore").setLevel(logging.WARNING)


@app.command()
def run(
    debug: bool = typer.Option(False, "--debug", "-d", help="Enable debug logging"),
    env_file: Path | None = typer.Option(None, "--env-file", help="Path to .env file"),
):
    """Start the interactive REPL (default command)."""
    _setup_logging(debug)

    if env_file:
        from dotenv import load_dotenv
        load_dotenv(env_file)

    config = Config.load()
    application = IronclawApp(config)

    try:
        asyncio.run(application.run())
    except KeyboardInterrupt:
        console.print("\n[dim]Interrupted.[/dim]")
        sys.exit(0)


@app.command()
def version():
    """Print the current version."""
    from ironclaw import __version__
    console.print(f"ironclaw {__version__} (LangGraph)")


@app.command()
def config_show():
    """Show the current configuration."""
    cfg = Config.load()
    console.print_json(cfg.model_dump_json(indent=2))


if __name__ == "__main__":
    app()
