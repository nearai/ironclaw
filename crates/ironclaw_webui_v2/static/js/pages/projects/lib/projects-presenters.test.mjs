import assert from "node:assert/strict";
import test from "node:test";

import { formatMissionCadence } from "./projects-presenters.js";

const t = (key) => {
  if (key === "projects.missions.manual") return "Localized manual";
  return key;
};

test("formatMissionCadence localizes backend manual cadence type", () => {
  assert.equal(formatMissionCadence({ cadence_type: "manual" }, t), "Localized manual");
  assert.equal(formatMissionCadence({ cadence_type: "Manual" }, t), "Localized manual");
});

test("formatMissionCadence preserves descriptions and custom cadence types", () => {
  assert.equal(
    formatMissionCadence({ cadence_description: "Every weekday", cadence_type: "manual" }, t),
    "Every weekday"
  );
  assert.equal(formatMissionCadence({ cadence_type: "cron" }, t), "cron");
});

test("formatMissionCadence falls back to localized manual label", () => {
  assert.equal(formatMissionCadence({}, t), "Localized manual");
  assert.equal(formatMissionCadence(null, t), "Localized manual");
});
