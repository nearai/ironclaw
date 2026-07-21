// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  formatMessageRole,
  formatMetricValue,
  formatMissionCadence,
  formatMissionStatus,
  formatProjectDate,
  formatProjectHealth,
  formatProjectRelativeTime,
  formatThreadState,
  formatThreadType,
  projectCount,
  threadPresentation,
} from "./projects-presenters";

const t = (key, params = {}) => {
  const translations = {
    "projects.count.steps": "Steps: {count}",
    "projects.health.green": "Localized healthy",
    "projects.metric.notSet": "Localized not set",
    "projects.missions.manual": "Localized manual",
    "projects.relative.hoursAgo": "{count} localized hours ago",
    "projects.relative.inMinutes": "in {count} localized minutes",
    "projects.relative.noActivity": "Localized no activity",
    "projects.date.notAvailable": "Localized date unavailable",
    "projects.role.assistant": "Localized assistant",
    "projects.status.active": "Localized active",
    "projects.thread.type.missionRun": "Localized mission run",
    "projects.threadState.done": "Localized done",
  };
  if (translations[key]) {
    return translations[key].replace(/\{(\w+)\}/g, (match, name) =>
      params[name] !== undefined ? params[name] : match
    );
  }
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

test("date and relative-time formatters use localized fallback and count params", () => {
  const originalNow = Date.now;
  Date.now = () => new Date("2026-07-07T12:00:00Z").getTime();
  try {
    assert.equal(formatProjectDate(null, t), "Localized date unavailable");
    assert.equal(formatProjectRelativeTime(null, t), "Localized no activity");
    assert.equal(
      formatProjectRelativeTime("2026-07-07T10:00:00Z", t),
      "2 localized hours ago"
    );
    assert.equal(
      formatProjectRelativeTime("2026-07-07T12:05:00Z", t),
      "in 5 localized minutes"
    );
  } finally {
    Date.now = originalNow;
  }
});

test("count and metric formatters cover interpolation and unset metric labels", () => {
  assert.equal(projectCount(t, "steps", 7), "Steps: 7");
  assert.equal(projectCount(t, "steps", undefined), "Steps: 0");
  assert.equal(formatMetricValue(null, t), "Localized not set");
  assert.equal(formatMetricValue({ current: 3, target: 5, unit: "runs" }, t), "3 runs / 5 runs");
  assert.equal(formatMetricValue({ target: 5 }, t), "Localized not set / 5");
});
