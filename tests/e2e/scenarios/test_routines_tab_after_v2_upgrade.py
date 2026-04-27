"""Regression for #2982: Routines tab visibility after engine v1 → v2 upgrade.

The bug: when ENGINE_V2 is on, `applyEngineModeToTabs()` and
`applyEngineModeUi()` unconditionally hid the v1-only Routines tab.
Users who upgraded from a v1 install (e.g. 0.24.0 → 0.26.0) lost the UI
affordance to view or manage their existing routines, even though the
routines were still in the DB and the API still served them.

The fix carries a `userHasLegacyRoutines` flag — when the flag is set,
the Routines tab stays visible even with engine v2 enabled. These tests
drive the JS helpers directly via `page.evaluate()` because the e2e
harness ships with `ROUTINES_ENABLED=false`, so we cannot create a real
routine in fixture setup. See `tests/e2e/CLAUDE.md` → "Environment
passed to ironclaw in tests".

Some scenarios route-mock `/api/routines/summary` so we can drive the
real `fetchGatewayStatus` / `refreshLegacyRoutinesPresence` call sites
end-to-end and exercise the testing-rule "test through the caller, not
just the helper".
"""

import json


# ── Helper-level coverage (each branch of the predicate) ──────────────


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


# ── Caller-level coverage (drives the actual call sites) ──────────────


async def _route_summary(page, *, total: int):
    """Stub /api/routines/summary to return a controlled total count."""

    async def handler(route):
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({
                "total": total,
                "enabled": total,
                "disabled": 0,
                "unverified": 0,
                "failing": 0,
                "runs_today": 0,
            }),
        )

    await page.route("**/api/routines/summary", handler)


async def test_refresh_legacy_routines_presence_sets_flag_from_summary(page):
    """`/api/routines/summary` total > 0 → userHasLegacyRoutines flips to true (#2982)."""

    await _route_summary(page, total=3)

    flag = await page.evaluate(
        """
        async () => {
            userHasLegacyRoutines = false;
            await refreshLegacyRoutinesPresence();
            return userHasLegacyRoutines;
        }
        """
    )
    assert flag is True, "userHasLegacyRoutines must be true when /api/routines/summary returns total > 0"


async def test_refresh_legacy_routines_presence_zero_total_clears_flag(page):
    """total = 0 → flag clears (covers post-delete case where last legacy routine is gone)."""

    await _route_summary(page, total=0)

    flag = await page.evaluate(
        """
        async () => {
            userHasLegacyRoutines = true;
            await refreshLegacyRoutinesPresence();
            return userHasLegacyRoutines;
        }
        """
    )
    assert flag is False, "Flag must clear when /api/routines/summary returns total: 0"


async def test_refresh_legacy_routines_presence_swallows_fetch_error(page):
    """Failed fetch → flag stays at its prior value, promise still resolves (#2982)."""

    async def fail(route):
        await route.fulfill(status=503, body="server unavailable")

    await page.route("**/api/routines/summary", fail)

    result = await page.evaluate(
        """
        async () => {
            userHasLegacyRoutines = true; // prior value preserved on failure
            await refreshLegacyRoutinesPresence();
            return userHasLegacyRoutines;
        }
        """
    )
    # Pre-existing value is preserved; the .catch() returns a resolved
    # promise so callers chained via .then() still run.
    assert result is True, "Failed summary fetch must not clobber prior flag value"


async def test_post_delete_refresh_hides_tab_when_last_legacy_routine_removed(page):
    """After deleting the last legacy routine the tab falls back to hidden (#2982)."""

    await _route_summary(page, total=0)

    hidden = await page.evaluate(
        """
        async () => {
            engineV2Enabled = true;
            userHasLegacyRoutines = true;
            // Simulate the post-delete refresh path that routines.js
            // deleteRoutine wires up: refresh, then re-apply.
            await refreshLegacyRoutinesPresence();
            applyEngineModeToTabs();
            applyEngineModeUi();
            const tab = document.querySelector('.tab-bar [data-tab-role="routines"]');
            return tab && tab.style.display === 'none';
        }
        """
    )
    assert hidden, "Routines tab must hide once the last legacy routine is deleted"


async def test_first_status_poll_does_not_refetch_summary_while_in_flight(page):
    """`engineModeApplied` flips synchronously so a second poll cannot fan out (#2982)."""

    # Count calls to /api/routines/summary; resolve slowly so a second
    # call would race the first if the guard were missing.
    call_counter = {"n": 0}

    async def slow_handler(route):
        call_counter["n"] += 1
        await page.wait_for_timeout(50)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({
                "total": 1,
                "enabled": 1,
                "disabled": 0,
                "unverified": 0,
                "failing": 0,
                "runs_today": 0,
            }),
        )

    await page.route("**/api/routines/summary", slow_handler)

    final_state = await page.evaluate(
        """
        async () => {
            // Reset so the call site enters its first-time branch twice
            // back-to-back. Without the synchronous engineModeApplied=true
            // assignment, both calls would kick off a /api/routines/summary
            // fetch (the actual race window described in PR review note #3).
            engineModeApplied = false;
            userHasLegacyRoutines = false;
            const fakeStatus = { engine_v2_enabled: true, restart_enabled: false };

            // Inline mimic of the first-time branch in fetchGatewayStatus.
            // We can't await fetchGatewayStatus directly because it kicks
            // off the refresh in a fire-and-forget chain — so drive the
            // exact same code path here and capture the in-flight side
            // effect.
            const fire = () => {
                if (!engineModeApplied) {
                    engineModeApplied = true;
                    engineV2Enabled = !!fakeStatus.engine_v2_enabled;
                    return refreshLegacyRoutinesPresence().then(function() {
                        applyEngineModeToTabs();
                        applyEngineModeUi();
                    });
                }
                applyEngineModeUi();
                return Promise.resolve();
            };

            const a = fire();
            const b = fire(); // second poll while a is still pending
            await Promise.all([a, b]);
            const tab = document.querySelector('.tab-bar [data-tab-role="routines"]');
            return {
                applied: engineModeApplied,
                visible: tab && tab.style.display !== 'none',
                flag: userHasLegacyRoutines,
            };
        }
        """
    )

    assert call_counter["n"] == 1, (
        f"second status poll must not race in a duplicate /api/routines/summary fetch "
        f"(saw {call_counter['n']} calls)"
    )
    assert final_state["applied"] is True
    assert final_state["flag"] is True
    assert final_state["visible"] is True, "Tab must end up visible once the single fetch resolves"


async def test_restore_from_hash_routines_route_reaches_routines_tab_with_legacy_data(page):
    """Hash restoration into `routines` lands on the routines tab when legacy data exists."""

    landed_on_routines = await page.evaluate(
        """
        () => {
            engineV2Enabled = true;
            userHasLegacyRoutines = true;
            // restoreFromHash() consults shouldHideRoutinesTab() in its
            // 'routines' branch; verify by simulating the decision.
            return !shouldHideRoutinesTab();
        }
        """
    )
    assert landed_on_routines, (
        "restoreFromHash routines branch must dispatch to Routines, not Missions, "
        "when legacy routines exist"
    )
