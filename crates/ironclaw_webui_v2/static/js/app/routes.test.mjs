import assert from "node:assert/strict";
import test from "node:test";

import { SETTINGS_SUB_ROUTES } from "./routes.js";

test("Settings sidebar exposes the tools permissions tab", () => {
  assert.ok(
    SETTINGS_SUB_ROUTES.some((route) => route.id === "tools"),
    "Tools permissions must be reachable from the Settings sidebar"
  );
});
