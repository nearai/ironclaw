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
import contextlib
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

# Standard xterm CSI escapes crossterm's unix parser recognizes (verified
# against the vendored crossterm 0.29 source,
# `event/sys/unix/parse.rs::parse_csi`'s numbered-tilde and bare-letter
# arms): unlike Ctrl+<letter> (a single raw control byte), these keys need
# the full multi-byte CSI sequence a real terminal would send.
PAGE_UP = "\x1b[5~"
PAGE_DOWN = "\x1b[6~"
END_KEY = "\x1b[F"


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

    def wait_for_absence(
        self, needle: str, *, timeout: float, poll_interval: float = 0.2
    ) -> None:
        """Poll the reconstructed screen until `needle` no longer renders.

        The mirror image of `wait_for`: used to confirm a modal actually
        closed (its bordered popup content disappears) rather than merely
        that the chat screen's always-present chrome (composer/status bar)
        is still on screen — that chrome renders every frame regardless of
        modal state, so it can't distinguish open from closed.
        """
        deadline = time.monotonic() + timeout
        last = self.text()
        while time.monotonic() < deadline:
            self._pump(poll_interval)
            last = self.text()
            if needle not in last:
                return
        pytest.fail(
            f"expected rendered text {needle!r} to disappear within {timeout}s.\n"
            f"last rendered screen:\n{last}"
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


@contextlib.contextmanager
def _spawn_tui(ironclaw_reborn_binary: str, reborn_tui_server: dict):
    """Spawns a fresh TUI child against the shared ``reborn_tui_server``,
    wired with the same env/pty setup ``test_reborn_tui_happy_path`` builds
    inline below, and force-terminates it on exit.

    Every scenario in this module drives its own pty/screen pair against
    the *same* module-scoped server — and so the same account/thread,
    exactly as a real user's ``ironclaw-reborn tui`` would reconnect and
    pick up where a previous session left off. Scenarios below that reason
    about item counts (the scroll test) are written to hold regardless of
    how much history earlier scenarios already left on that shared thread —
    see that test's own docstring.
    """
    base_url = reborn_tui_server["base_url"]
    reborn_home = reborn_tui_server["reborn_home"]
    home_dir = reborn_tui_server["home_dir"]

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
        # Same readiness gate as the happy-path test below: the status
        # bar's global keybinding hint only draws once startup has
        # completed and the event loop is drawing frames.
        screen.wait_for("quit", timeout=30)
        yield child, screen
    finally:
        if child.isalive():
            child.terminate(force=True)


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


@pytest.mark.timeout(60)
async def test_reborn_tui_multiline_reply_renders_separate_rows(
    ironclaw_reborn_binary, reborn_tui_server
):
    """Real-terminal guard for the multi-line render fix (defect D): an
    assistant reply with embedded newlines must render as separate rows on
    the reconstructed screen, not squish onto one line
    (`ui/transcript.rs::split_lines`/`split_lines_with_prefix`).
    """
    await _wait_for_webchat_session(reborn_tui_server["base_url"])
    with _spawn_tui(ironclaw_reborn_binary, reborn_tui_server) as (child, screen):
        # mock_llm.py CANNED_RESPONSES: r"reborn tui multiline reply" ->
        # "line one\nline two\nline three".
        child.send("reborn tui multiline reply")
        child.send("\r")
        screen.wait_for("assistant: line one", timeout=30)
        screen.wait_for("line two", timeout=5)
        screen.wait_for("line three", timeout=5)

        content = screen.text()
        assert "line oneline two" not in content, (
            f"embedded newlines must not collapse onto one squished line:\n{content}"
        )
        assert "line twoline three" not in content, (
            f"embedded newlines must not collapse onto one squished line:\n{content}"
        )

        rows = content.splitlines()
        row_one = next(i for i, r in enumerate(rows) if "line one" in r)
        row_two = next(i for i, r in enumerate(rows) if "line two" in r)
        row_three = next(i for i, r in enumerate(rows) if "line three" in r)
        assert row_one < row_two < row_three, (
            "each newline-separated segment must render on its own, "
            f"strictly later row: rows={rows}"
        )


SCROLL_TURN_COUNT = 20
# Mirrors `crates/ironclaw_reborn_tui/src/app/mod.rs::TRANSCRIPT_PAGE_SIZE`.
TRANSCRIPT_PAGE_SIZE = 10


@pytest.mark.timeout(180)
async def test_reborn_tui_scroll_auto_follow_and_page_navigation(
    ironclaw_reborn_binary, reborn_tui_server
):
    """Real-terminal guard for the scroll fix: the transcript pane
    auto-follows the latest reply once content overflows the fixed 40-row
    pane, `PageUp` reveals earlier content, and `End` resumes following
    the tail.

    Sends `SCROLL_TURN_COUNT` (20) distinct turns — 2 transcript items
    (user + assistant) each, 40 total — comfortably overflowing the pane's
    ~34-row visible item budget (`ui/transcript.rs::visible_window`, fixed
    120x40 pty). Each turn's user text is unique ("hello scroll NNN") so a
    specific turn's visibility can be asserted. The mock returns a unique
    `Scroll turn NNN complete.` reply for each input, so every iteration can
    wait for that exact assistant completion before starting the next turn.

    Order independence: this module's server/thread is shared across every
    scenario in the file (see `_spawn_tui`'s docstring), so this test may
    run with some prior history already on the thread. The assertions below
    are written to hold regardless of that prior count `P`: `app/mod.rs`'s
    `scroll_transcript_page_up` anchors from the transcript's *total*
    length on the first press and from its own previous pin thereafter,
    stepping back one `TRANSCRIPT_PAGE_SIZE` (10) at a time. After exactly
    4 presses from follow, the pin lands `4 * TRANSCRIPT_PAGE_SIZE` (40)
    items back from the tail — i.e. exactly at this test's own first turn
    (index `P`, since this test's 40 items occupy `[P, P+40)`) — for any
    `P >= 0`, as long the shared thread's total history stays within the
    server's 50-message timeline-refetch page
    (`SETTLED_RUN_TIMELINE_REFETCH_LIMIT` in `lib.rs`), which holds here
    since only the happy-path and multi-line scenarios run before this one
    and add at most 4 prior items.
    """
    await _wait_for_webchat_session(reborn_tui_server["base_url"])
    with _spawn_tui(ironclaw_reborn_binary, reborn_tui_server) as (child, screen):
        for i in range(1, SCROLL_TURN_COUNT + 1):
            marker = f"hello scroll {i:03d}"
            child.send(marker)
            child.send("\r")
            # Waiting for the user marker is insufficient: it appears in the
            # composer before Enter is processed and in the local transcript
            # before the model run settles. The mock's reply is unique per
            # turn, and the status-bar absence then confirms the run left its
            # working state before the next message is entered.
            screen.wait_for(f"assistant: Scroll turn {i:03d} complete.", timeout=30)
            screen.wait_for_absence("working…", timeout=15)

        first_marker = "hello scroll 001"
        last_marker = f"hello scroll {SCROLL_TURN_COUNT:03d}"

        # Auto-follow: the latest turn is visible by default; this test's
        # own first turn is scrolled off the top.
        follow_view = screen.text()
        assert last_marker in follow_view, (
            f"latest reply must be visible by default (auto-follow): {follow_view}"
        )
        assert first_marker not in follow_view, (
            "the pane shows only the tail under follow; turn 1 must be "
            f"scrolled off the top: {follow_view}"
        )

        for _ in range(4):
            child.send(PAGE_UP)
        screen.wait_for(first_marker, timeout=15)
        scrolled_view = screen.text()
        assert last_marker not in scrolled_view, (
            "scrolling up must move the tail (this test's latest turn) "
            f"off screen: {scrolled_view}"
        )

        child.send(END_KEY)
        screen.wait_for(last_marker, timeout=15)


@pytest.mark.timeout(45)
async def test_reborn_tui_provider_panel_marks_the_active_provider(
    ironclaw_reborn_binary, reborn_tui_server
):
    """Real-terminal guard for the provider-marker fix: `Ctrl+P` opens the
    provider modal, the provider list renders, and the configured active
    provider carries the active marker glyph
    (`ui/modals.rs::render_provider`'s `"● "` prefix). `Esc` closes back to
    chat.

    `reborn_webui_harness.py`'s `write_config_toml` points this server's
    `[llm.default]` at the mock LLM with `provider_id = "openai"`, so
    `openai` is the row that must carry the marker.
    """
    await _wait_for_webchat_session(reborn_tui_server["base_url"])
    with _spawn_tui(ironclaw_reborn_binary, reborn_tui_server) as (child, screen):
        child.send("\x10")  # Ctrl+P
        screen.wait_for("● openai", timeout=15)

        child.send("\x1b")  # Esc: Providers-level closes the modal in one press.
        screen.wait_for_absence("┌providers", timeout=15)


@pytest.mark.timeout(45)
async def test_reborn_tui_automations_panel_round_trip(
    ironclaw_reborn_binary, reborn_tui_server
):
    """`Ctrl+A` opens the automations modal and it renders
    (`ui/modals.rs::render_automations`'s bordered "automations" block,
    empty-state since no automation is seeded), then `Esc` closes it back
    to chat.

    Seeding a real automation isn't feasible in this mock harness — that
    round trip (create/list/hold-badge rendering) is covered at the
    integration tier. This scenario only proves the panel opens and closes
    through a real terminal.
    """
    await _wait_for_webchat_session(reborn_tui_server["base_url"])
    with _spawn_tui(ironclaw_reborn_binary, reborn_tui_server) as (child, screen):
        child.send("\x01")  # Ctrl+A
        screen.wait_for("┌automations", timeout=15)

        child.send("\x1b")  # Esc closes the modal.
        screen.wait_for_absence("┌automations", timeout=15)


@pytest.mark.skip(
    reason=(
        "Approval-kind gates are not reachable through this harness's plain "
        "`local-dev` `ironclaw-reborn serve` spawn. Empirically verified: "
        "sending the existing 'reborn write approval file <label>' canned "
        "trigger (mock_llm.py) — the exact trigger "
        "test_reborn_webui_v2_legacy_tool_permissions.py's "
        "test_reborn_legacy_always_approve_survives_reborn_restart asserts "
        "produces `allow_always is True` for `builtin.write_file` — through "
        "this real TUI/pty session ran the write to completion "
        "(CapabilityDisplayPreview shows status 'completed') with no "
        "Gate/AuthRequired event ever emitted; the composer never blocked. "
        "tests/integration/tui_gate_seam.rs's own approve/deny coverage "
        "only reaches this gate via `RebornIntegrationGroup::live_approvals()` "
        "— a test-only harness that force-configures live approval "
        "interactions the plain `serve` binary this e2e spawns does not "
        "wire up on its own (matches the local-dev "
        "interactive_default-vs-planned_default divergence: real chat/CLI "
        "submissions resolve a more permissive effective policy than the "
        "interactive-only path some other harnesses configure). Gate "
        "approve/deny end-to-end is covered at the integration tier in "
        "tui_gate_seam.rs; do not force a fake trigger here."
    )
)
@pytest.mark.timeout(60)
async def test_reborn_tui_gate_approve_resolves_the_turn(
    ironclaw_reborn_binary, reborn_tui_server
):
    """Skeleton for a real-terminal approve-a-pending-gate guard — see the
    `skip` reason above for why this isn't reachable through this harness
    today. Left in place (rather than deleted) so a future change that
    makes the plain `local-dev` serve spawn actually gate a builtin write
    has an obvious place to un-skip and fill in the body: press `a` on
    the gate zone's options line and assert the post-approval reply
    renders.
    """
    await _wait_for_webchat_session(reborn_tui_server["base_url"])
    with _spawn_tui(ironclaw_reborn_binary, reborn_tui_server) as (child, screen):
        child.send("reborn write approval file tuiprobe")
        child.send("\r")
        screen.wait_for("[a] allow  [A] allow always", timeout=30)

        child.send("a")
        screen.wait_for(
            "assistant: Done - saved the approval test file.", timeout=30
        )
