import assert from "node:assert/strict";
import { test } from "vitest";

import { visibleSettingsTabs } from "./settings-tabs";

test("non-admin settings include inference but exclude user administration", () => {
  const ids = visibleSettingsTabs(false).map((tab) => tab.id);

  assert.ok(ids.includes("inference"));
  assert.ok(!ids.includes("users"));
});
