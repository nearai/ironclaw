import assert from "node:assert/strict";
import test from "node:test";

import {
  formatMessageRole,
  formatMissionCadence,
  formatMissionStatus,
  formatProjectHealth,
  formatThreadState,
  formatThreadType,
  threadPresentation,
} from "./projects-presenters.js";

const t = (key) => {
  const translations = {
    "projects.health.green": "Localized healthy",
    "projects.missions.manual": "Localized manual",
    "projects.role.assistant": "Localized assistant",
    "projects.status.active": "Localized active",
    "projects.thread.type.missionRun": "Localized mission run",
    "projects.threadState.done": "Localized done",
  };
  if (translations[key]) return translations[key];
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

test("project enum formatters localize known values and preserve unknowns", () => {
  assert.equal(formatProjectHealth("green", t), "Localized healthy");
  assert.equal(formatProjectHealth("blue", t), "blue");
  assert.equal(formatMissionStatus("Active", t), "Localized active");
  assert.equal(formatThreadState("Done", t), "Localized done");
  assert.equal(formatThreadType("mission_run", t), "Localized mission run");
  assert.equal(formatThreadType("ad_hoc", t), "ad hoc");
  assert.equal(formatMessageRole("assistant", t), "Localized assistant");
});

test("threadPresentation interpolates fallback title without a translator", () => {
  assert.equal(threadPresentation({ id: "abcdefghij" }, null).title, "Thread abcdefgh");
});
