import assert from "node:assert/strict";
import { test } from "vitest";

import { SETTINGS_SUB_ROUTES } from "./routes";

test("Settings sidebar exposes the tools permissions tab", () => {
  assert.ok(
    SETTINGS_SUB_ROUTES.some((route) => route.id === "tools"),
    "Tools permissions must be reachable from the Settings sidebar"
  );
});

test("Settings sidebar exposes the appearance preferences tab", () => {
  assert.ok(
    SETTINGS_SUB_ROUTES.some((route) => route.id === "appearance"),
    "Appearance preferences must be reachable from the Settings sidebar"
  );
});
