"""Capture before/after screenshots of every major UI surface.

Opt-in — runs only when ``CAPTURE_SCREENSHOTS=1`` is set. Writes PNGs
to ``SCREENSHOT_DIR`` (default ``/tmp/ironclaw-screenshots``).

Used to produce visual diffs for design-system PRs (Phase A, B, C, …).
The typical workflow:

    # 1. Baseline (before design changes)
    CAPTURE_SCREENSHOTS=1 SCREENSHOT_DIR=/tmp/before \
        pytest tests/e2e/scenarios/capture_design_surfaces.py

    # 2. Apply design changes on the same branch

    # 3. After
    CAPTURE_SCREENSHOTS=1 SCREENSHOT_DIR=/tmp/after \
        pytest tests/e2e/scenarios/capture_design_surfaces.py

Outputs two sets of 1280×800 PNGs per tab + theme; reference them in
the PR body for visual review.
"""

import os
import pathlib

import pytest

from helpers import AUTH_TOKEN, SEL

# --- Surface inventory ----------------------------------------------------
#
# (tab_id, wait_for_selector) tuples. The wait_for selector is a best-effort
# anchor — a missing selector is downgraded to a fixed wait so empty states
# are still captured.

DARK_SURFACES = [
    ("chat",     "#chat-input"),
    ("memory",   "#memory-tab-container, #memory-tree, .empty-state"),
    ("jobs",     ".jobs-container, .empty-state"),
    ("missions", ".missions-container, .empty-state"),
    ("routines", ".routines-container, .empty-state"),
    ("settings", ".settings-layout, .settings-subtab"),
    ("logs",     "#tab-logs"),
]

# Capture the light-mode palette only for the two surfaces the user lands
# on first — enough to verify the color-token shift without ballooning the
# image count.
LIGHT_SURFACES = [
    ("chat",     "#chat-input"),
    ("settings", ".settings-layout, .settings-subtab"),
]


def _should_run() -> bool:
    return os.environ.get("CAPTURE_SCREENSHOTS", "").strip() in ("1", "true")


def _out_dir() -> pathlib.Path:
    raw = os.environ.get("SCREENSHOT_DIR", "/tmp/ironclaw-screenshots")
    p = pathlib.Path(raw)
    p.mkdir(parents=True, exist_ok=True)
    return p


async def _set_theme(page, theme: str) -> None:
    """Flip the <html> theme attributes the app reads on load."""
    await page.evaluate(
        f"""
        document.documentElement.setAttribute('data-theme', '{theme}');
        document.documentElement.setAttribute('data-theme-mode', '{theme}');
        """
    )
    # Give the CSS variable cascade a frame to settle.
    await page.wait_for_timeout(300)


async def _capture_tab(page, tab: str, wait_for: str, out: pathlib.Path, theme: str) -> None:
    btn = SEL["tab_button"].format(tab=tab)
    try:
        await page.locator(btn).click(timeout=3000)
    except Exception:
        # Some tabs (missions on v1, projects on v1) are hidden. Force-navigate
        # by setting the URL hash so the router promotes them.
        await page.evaluate(f"window.location.hash = '#/{tab}'")

    # Best-effort wait for the tab to populate.
    try:
        await page.wait_for_selector(wait_for, state="visible", timeout=3000)
    except Exception:
        pass  # Empty states are valid screenshots too.

    await page.wait_for_timeout(400)  # Let skeletons/loaders dissolve.
    await page.screenshot(path=str(out / f"{theme}-{tab}.png"))


async def _seed_chat_conversation(page) -> None:
    """Inject a one-turn conversation so chat screenshots show real bubbles.

    Uses the same DOM helpers the app calls from its SSE handlers — no
    new rendering path.
    """
    await page.evaluate(
        """
        () => {
            if (typeof addMessage !== 'function') return;
            addMessage('user', 'What is the current state of IronClaw?');
            addMessage('assistant',
                'IronClaw is a secure personal AI assistant built around defense in ' +
                'depth, self-expanding tools, and multi-channel access. The engine v2 ' +
                'rollout is currently landing per-project sandboxing and cost tracking.');
        }
        """
    )
    await page.wait_for_timeout(200)


async def test_capture_design_surfaces(page):
    """Walk every main tab, capture dark + (subset) light screenshots."""
    if not _should_run():
        pytest.skip("Set CAPTURE_SCREENSHOTS=1 to run")

    out = _out_dir()

    # Make projects/missions available in the v2-only guard, so we capture
    # them regardless of the baseline engine flag.
    await page.evaluate(
        """
        () => {
            if (typeof engineV2Enabled !== 'undefined') { engineV2Enabled = true; }
            if (typeof applyEngineModeToTabs === 'function') { applyEngineModeToTabs(); }
        }
        """
    )
    await page.wait_for_timeout(200)

    # Seed chat so the bubble styling is visible in both themes.
    await _seed_chat_conversation(page)

    # --- Dark mode ---
    await _set_theme(page, "dark")
    for tab, wait_for in DARK_SURFACES:
        await _capture_tab(page, tab, wait_for, out, "dark")

    # --- Light mode subset ---
    await _set_theme(page, "light")
    # Re-seed chat in case the tab switch cleared it.
    await page.locator(SEL["tab_button"].format(tab="chat")).click(timeout=3000)
    await _seed_chat_conversation(page)
    for tab, wait_for in LIGHT_SURFACES:
        await _capture_tab(page, tab, wait_for, out, "light")


async def test_capture_auth_screen(ironclaw_server, browser):
    """Capture the unauthenticated landing screen in both themes."""
    if not _should_run():
        pytest.skip("Set CAPTURE_SCREENSHOTS=1 to run")

    out = _out_dir()
    context = await browser.new_context(viewport={"width": 1280, "height": 800})
    pg = await context.new_page()
    try:
        await pg.goto(f"{ironclaw_server}/")  # no `?token=…` so auth screen stays
        await pg.wait_for_selector("#auth-screen", state="visible", timeout=5000)
        for theme in ("dark", "light"):
            await _set_theme(pg, theme)
            await pg.screenshot(path=str(out / f"{theme}-auth.png"))
    finally:
        await context.close()
