import assert from "node:assert/strict";
import { test } from "vitest";

import { visibleSidebarSubRoutes } from "./sidebar-nav";

test("member sidebar exposes inference but keeps users admin-only", () => {
  const routes = visibleSidebarSubRoutes("settings", false);
  const ids = routes.map((route) => route.id);

  assert.ok(ids.includes("inference"));
  assert.ok(!ids.includes("users"));
});
