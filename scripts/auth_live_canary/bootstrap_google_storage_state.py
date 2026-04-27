#!/usr/bin/env python3
"""Interactive bootstrap for the auth-browser-consent Google storage state.

The ``auth-browser-consent`` lane drives the real Google OAuth consent UI in
Playwright Chromium. When Google's risk engine sees a never-before-used
browser fingerprint it interrupts the flow with a "Verify it's you" /
"Is this really you trying to sign in?" challenge, which ``handle_google_popup``
in ``run_live_canary.py`` cannot solve.

Side-stepping that: log in once interactively in Playwright Chromium, save the
resulting cookies + localStorage to a ``storage_state.json`` file, then point
``AUTH_BROWSER_GOOGLE_STORAGE_STATE_PATH`` at it. Subsequent canary runs
spawn contexts with that storage state already loaded, so the OAuth popup
arrives at the consent screen with no password / verification step in the way.

Usage:

    python3 scripts/auth_live_canary/bootstrap_google_storage_state.py
    # follow the prompt, log in once, then press Enter

    export AUTH_BROWSER_GOOGLE_STORAGE_STATE_PATH=~/.ironclaw/auth-canary/google_storage_state.json
    LANE=auth-browser-consent PROVIDER=browser ./scripts/live-canary/run.sh

Re-run this when canary failures hint at an expired session — Google sessions
typically last weeks of active use but decay sooner if the storage state sits
unused.
"""

from __future__ import annotations

import argparse
import asyncio
from pathlib import Path

from playwright.async_api import async_playwright

DEFAULT_OUTPUT = Path.home() / ".ironclaw" / "auth-canary" / "google_storage_state.json"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Interactively log into Google in Playwright Chromium and capture "
            "the resulting storage state for the auth-browser-consent canary."
        )
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help=f"Where to write the storage state JSON (default: {DEFAULT_OUTPUT}).",
    )
    parser.add_argument(
        "--start-url",
        default="https://accounts.google.com/",
        help="URL to land on before manual login (default: accounts.google.com).",
    )
    return parser.parse_args()


async def capture_storage_state(output: Path, start_url: str) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=False)
        context = await browser.new_context(viewport={"width": 1280, "height": 720})
        page = await context.new_page()
        await page.goto(start_url)
        print(
            "\n👉 Log into the dedicated test Google account in the Playwright window."
        )
        print("   Solve any 'Verify it's you' challenges Google shows.")
        print(
            "   When you reach the Google account home page (or any page that "
            "confirms you're signed in), come back here.\n"
        )
        try:
            input("Press Enter once you're logged in (Ctrl+C to abort)... ")
        except (KeyboardInterrupt, EOFError):
            print("\nAborted; no file written.")
            await browser.close()
            return
        await context.storage_state(path=str(output))
        await browser.close()
    print(f"\n✅ Saved storage state to {output}")
    print(f"   export AUTH_BROWSER_GOOGLE_STORAGE_STATE_PATH={output}")


def main() -> int:
    args = parse_args()
    asyncio.run(capture_storage_state(args.output, args.start_url))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
