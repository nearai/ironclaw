"""Committed, deterministic PTY end-to-end test for the Reborn TUI.

The terminal analog of the Playwright WebChat v2 smoke scenario
(``test_reborn_webui_v2_smoke.py``): boots the real ``ironclaw-reborn
serve`` binary against the local, regex-matched mock LLM (``mock_llm.py`` —
no real network, no live provider), then drives the ``ironclaw-reborn tui``
ratatui client through a real pseudo-terminal with ``pexpect``. Runs in the
PR-blocking ``webui-v2-smoke`` job (``.github/workflows/reborn-e2e.yml``).

Determinism / no-flake contract:
  * The mock LLM's canned replies are exact regex-matched strings, not live
    model output — the assistant's reply text is byte-identical every run
    (see ``mock_llm.py``'s ``CANNED_RESPONSES`` "hello|hi|hey" entry).
  * No fixed ``sleep``s: every wait is either a bounded HTTP polling loop or
    a bounded poll of the reconstructed terminal screen (see
    ``_TuiScreen`` below).
  * Fixed 120x40 pty dimensions so ratatui's status-bar fold-by-width logic
    (``crates/ironclaw_reborn_tui/src/ui/status_bar.rs``) always keeps the
    full keybinding hint on one line.
  * No Postgres, no Docker, no real network: ``local-dev`` profile,
    file-backed libSQL under a throwaway ``IRONCLAW_REBORN_HOME``.

Screen reconstruction: ratatui's crossterm backend only rewrites terminal
cells that changed between frames — unchanged interior cells (often plain
spaces between words) are skipped via cursor-jump escapes rather than
rewritten. That means a rendered sentence is *not* reliably one contiguous
run of bytes in the raw pty stream, so naively ``pexpect.expect_exact``-ing
the reply text against the raw stream flakes/fails even on a correct
render. ``_TuiScreen`` feeds the raw stream through ``pyte`` (a small VT100
screen emulator) to reconstruct the actual on-screen 2D character grid, so
an exact-substring assertion against the reconstructed screen holds.
"""

import asyncio
import os
import time

import httpx
import pytest

pexpect = pytest.importorskip("pexpect")
pyte = pytest.importorskip("pyte")

import reborn_webui_harness as harness
from reborn_webui_harness import REBORN_V2_AUTH_TOKEN, reborn_bearer_headers

# Bounds the test body (not fixture setup, which reuses the session-scoped
# binary/mock-LLM fixtures): generous but finite, so a genuine hang fails
# the PR-blocking job instead of stalling it.
pytestmark = pytest.mark.timeout(120)

TERM_COLS = 120
TERM_ROWS = 40


class _TuiScreen:
    """Reconstructs the TUI's on-screen text from a pexpect pty stream.

    Wraps a ``pyte.Screen``/``pyte.Stream`` pair sized to match the pty
    dimensions the child was spawned with, so ``wait_for`` polls the same
    2D grid a real terminal would show — not the raw ANSI byte stream,
    which (per the module docstring) does not reliably contain rendered
    sentences as contiguous byte runs.
    """

    def __init__(self, child: "pexpect.spawn", *, cols: int, rows: int) -> None:
        self._child = child
        self._screen = pyte.Screen(cols, rows)
        self._stream = pyte.Stream(self._screen)

    def _pump(self, timeout: float) -> None:
        try:
            chunk = self._child.read_nonblocking(size=65536, timeout=timeout)
        except pexpect.TIMEOUT:
            return
        if chunk:
            self._stream.feed(chunk)

    def text(self) -> str:
        return "\n".join(self._screen.display)

    def wait_for(self, needle: str, *, timeout: float, poll_interval: float = 0.2) -> None:
        """Poll the reconstructed screen until `needle` renders, or fail.

        Bounded by `timeout`; each underlying read itself has a short
        `poll_interval` timeout so this never blocks past the deadline.
        No fixed sleep — the loop exits the instant the text appears.
        """
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self._pump(poll_interval)
            if needle in self.text():
                return
        pytest.fail(
            f"expected rendered text {needle!r} not found within {timeout}s.\n"
            f"last rendered screen:\n{self.text()}"
        )


@pytest.fixture(scope="module")
async def reborn_tui_server(ironclaw_reborn_binary, mock_llm_server, tmp_path_factory):
    """Boots ``ironclaw-reborn serve`` against the mock LLM for the TUI scenario.

    A dedicated fixture rather than the shared ``reborn_v2_server``: the
    ``tui`` subprocess needs the exact ``reborn_home`` directory ``serve``
    was started with so its config lookup (``webui_token::resolve_webui_token``,
    same precedence `serve` uses) is unambiguous. Module-scoped so the one
    scenario in this file gets one throwaway server, not a shared one.
    """
    home_dir = tmp_path_factory.mktemp("ironclaw-reborn-tui-home")
    proc, base_url = await harness.start_reborn_webui_v2_server(
        ironclaw_reborn_binary=ironclaw_reborn_binary,
        mock_llm_server=mock_llm_server,
        home_dir=home_dir,
        profile=harness.DEFAULT_PROFILE,
        log_prefix="reborn-tui",
    )
    try:
        yield {
            "base_url": base_url,
            "home_dir": home_dir,
            "reborn_home": home_dir / "reborn-home",
        }
    finally:
        await harness.close_reborn_server(proc)


async def _wait_for_webchat_session(base_url: str, *, timeout: float = 30.0) -> None:
    """Bounded poll of ``/api/webchat/v2/session`` — the exact readiness
    probe ``ironclaw_reborn_tui::spawn::ensure_serve`` itself uses before
    the TUI takes over the terminal (see ``client/session.rs``). Gating on
    it here, before the pty is even spawned, separates "is serve up" from
    "did the TUI render" so a failure points at the right layer.
    """
    deadline = time.monotonic() + timeout
    last_status: int | None = None
    async with httpx.AsyncClient(headers=reborn_bearer_headers()) as client:
        while time.monotonic() < deadline:
            try:
                response = await client.get(
                    f"{base_url}/api/webchat/v2/session", timeout=5
                )
                last_status = response.status_code
                if response.status_code == 200:
                    return
            except httpx.HTTPError:
                pass
            await asyncio.sleep(0.25)
    pytest.fail(
        f"/api/webchat/v2/session never returned 200 within {timeout}s "
        f"(last status: {last_status})"
    )


async def test_reborn_tui_happy_path(ironclaw_reborn_binary, reborn_tui_server):
    """Boot, send a message, get the exact canned reply, nav round trip, quit."""
    base_url = reborn_tui_server["base_url"]
    reborn_home = reborn_tui_server["reborn_home"]
    home_dir = reborn_tui_server["home_dir"]

    await _wait_for_webchat_session(base_url)

    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home_dir),
        "IRONCLAW_REBORN_HOME": str(reborn_home),
        "IRONCLAW_REBORN_PROFILE": harness.DEFAULT_PROFILE,
        "IRONCLAW_REBORN_WEBUI_TOKEN": REBORN_V2_AUTH_TOKEN,
        "TERM": "xterm-256color",
        "RUST_LOG": "ironclaw=warn,ironclaw_runner=warn",
        "RUST_BACKTRACE": "1",
        "NO_PROXY": "127.0.0.1,localhost,::1",
        "no_proxy": "127.0.0.1,localhost,::1",
    }

    child = pexpect.spawn(
        ironclaw_reborn_binary,
        ["tui", "--base-url", base_url],
        env=env,
        cwd=str(home_dir),
        dimensions=(TERM_ROWS, TERM_COLS),  # (rows, cols)
        timeout=20,
        encoding="utf-8",
        codec_errors="replace",
        echo=False,
    )
    screen = _TuiScreen(child, cols=TERM_COLS, rows=TERM_ROWS)
    try:
        # Readiness gate 2: the TUI's own first render. The status bar's
        # global keybinding hint (`ui/status_bar.rs::hint_text`) only draws
        # once startup's `list_threads` (+ create-if-empty) and initial
        # timeline fetch have completed and the event loop is drawing
        # frames — "quit" is unique to that idle hint row.
        screen.wait_for("quit", timeout=30)

        # Happy path: a message the mock deterministically replies to
        # (mock_llm.py CANNED_RESPONSES: r"\bhello\b|\bhi\b|\bhey\b").
        child.send("hello")
        child.send("\r")
        screen.wait_for("assistant: Hello! How can I help you today?", timeout=30)

        # Nav round trip: Ctrl+X opens the threads modal. Its pinned
        # "+ new" row (`ui/modals.rs::render_threads`) never renders
        # anywhere else, so matching it proves the modal actually opened.
        child.send("\x18")  # Ctrl+X
        screen.wait_for("+ new", timeout=15)
        child.send("\x1b")  # Esc closes the modal

        # Quit via `/exit`. This doubles as the "Esc returned focus to the
        # composer" assertion: if the modal were still open, `/exit` +
        # Enter would be swallowed as an unrecognized modal keypress (the
        # threads modal only handles Up/Down/Enter/'d'/Esc — see
        # `app/threads_modal.rs::dispatch_key`) and the process would never
        # exit, failing the EOF expect below instead of silently passing.
        child.send("/exit")
        child.send("\r")

        child.expect(pexpect.EOF, timeout=20)
        child.close()
        assert child.exitstatus == 0, (
            f"expected a clean exit, got exitstatus={child.exitstatus} "
            f"signalstatus={child.signalstatus}"
        )
    finally:
        if child.isalive():
            child.terminate(force=True)
