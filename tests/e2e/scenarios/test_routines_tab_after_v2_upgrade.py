"""Regression for #2982: Routines tab visibility after engine v1 → v2 upgrade.

The bug: when ENGINE_V2 is on, `applyEngineModeToTabs()` and
`applyEngineModeUi()` unconditionally hide the v1-only Routines tab.
Users who upgraded from a v1 install (e.g. 0.24.0 → 0.26.0) lost the UI
affordance to view or manage their existing routines, even though the
routines were still in the DB and the API still served them.

The fix carries a `userHasLegacyRoutines` flag — when the flag is set,
the Routines tab stays visible even with engine v2 enabled. These tests
drive the JS helpers directly via `page.evaluate()` because the e2e
harness ships with `ROUTINES_ENABLED=false`, so we cannot create a real
routine in fixture setup. See `tests/e2e/CLAUDE.md` → "Environment
passed to ironclaw in tests".
"""


async def test_routines_tab_visible_when_user_has_legacy_routines(page):
    """v2 enabled + legacy routines → Routines tab stays visible."""

    visible = await page.evaluate(
        """
        () => {
            engineV2Enabled = true;
            userHasLegacyRoutines = true;
            applyEngineModeToTabs();
            applyEngineModeUi();
            const tab = document.querySelector('.tab-bar [data-tab-role="routines"]');
            return tab && tab.style.display !== 'none';
        }
        """
    )
    assert visible, "Routines tab must stay visible when legacy routines exist"


async def test_routines_tab_hidden_in_v2_with_no_legacy_routines(page):
    """v2 enabled + no legacy routines → Routines tab hidden (existing v2 behavior)."""

    hidden = await page.evaluate(
        """
        () => {
            engineV2Enabled = true;
            userHasLegacyRoutines = false;
            applyEngineModeToTabs();
            applyEngineModeUi();
            const tab = document.querySelector('.tab-bar [data-tab-role="routines"]');
            return tab && tab.style.display === 'none';
        }
        """
    )
    assert hidden, "Routines tab must be hidden when v2 is on and user has no routines"


async def test_routines_tab_visible_in_v1(page):
    """Engine v1 → Routines tab visible regardless of routine count."""

    visible = await page.evaluate(
        """
        () => {
            engineV2Enabled = false;
            userHasLegacyRoutines = false;
            applyEngineModeToTabs();
            applyEngineModeUi();
            const tab = document.querySelector('.tab-bar [data-tab-role="routines"]');
            return tab && tab.style.display !== 'none';
        }
        """
    )
    assert visible, "Routines tab must always be visible in engine v1 mode"


async def test_routines_hash_route_routes_to_routines_when_legacy_exists(page):
    """`#/routines/<id>` → opens routine detail when legacy routines exist (#2982)."""

    routes_to_routines = await page.evaluate(
        """
        () => {
            engineV2Enabled = true;
            userHasLegacyRoutines = true;
            return shouldHideRoutinesTab() === false
                && normalizeTabForEngineMode('routines') === 'routines';
        }
        """
    )
    assert routes_to_routines, (
        "`routines` hash must resolve to the Routines tab when legacy routines exist"
    )


async def test_routines_hash_route_falls_back_to_missions_in_pure_v2(page):
    """`#/routines` → redirected to Missions when no legacy data (existing v2 behavior)."""

    redirects = await page.evaluate(
        """
        () => {
            engineV2Enabled = true;
            userHasLegacyRoutines = false;
            return shouldHideRoutinesTab() === true
                && normalizeTabForEngineMode('routines') === 'missions';
        }
        """
    )
    assert redirects, "Routines hash must redirect to Missions in pure v2 mode"
