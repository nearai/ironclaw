import assert from "node:assert/strict";
import test from "node:test";

import { primaryRoutes, routeForId, SETTINGS_SUB_ROUTES } from "./routes.js";

test("Settings sidebar exposes the tools permissions tab", () => {
  assert.ok(
    SETTINGS_SUB_ROUTES.some((route) => route.id === "tools"),
    "Tools permissions must be reachable from the Settings sidebar"
  );
});

test("hidden workflow routes stay registered while suppressed from primary navigation", () => {
  for (const id of ["jobs", "routines", "missions", "admin"]) {
    const route = routeForId(id);
    assert.equal(route.id, id);
    assert.equal(route.hidden, true, `${id} should stay hidden from primary nav`);
    assert.ok(route.path.startsWith("/"), `${id} should keep a direct client route`);
  }

  assert.equal(routeForId("not-a-route").id, primaryRoutes[0].id);
});
