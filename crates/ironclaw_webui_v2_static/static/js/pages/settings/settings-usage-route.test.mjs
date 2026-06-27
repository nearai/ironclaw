import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { SETTINGS_SUB_ROUTES } from "../../app/routes.js";
import { SETTINGS_TABS } from "./lib/settings-schema.js";

test("settings usage route is named usage, not budget", () => {
  assert(SETTINGS_SUB_ROUTES.some((route) => route.id === "usage"));
  assert(!SETTINGS_SUB_ROUTES.some((route) => route.id === "budget"));
  assert(SETTINGS_TABS.some((tab) => tab.id === "usage"));
  assert(!SETTINGS_TABS.some((tab) => tab.id === "budget"));
});

test("settings usage API uses usage path", () => {
  const source = readFileSync(new URL("./lib/settings-api.js", import.meta.url), "utf8");

  assert(source.includes("/api/webchat/v2/settings/usage"));
  assert(!source.includes("/api/webchat/v2/settings/budget"));
});
