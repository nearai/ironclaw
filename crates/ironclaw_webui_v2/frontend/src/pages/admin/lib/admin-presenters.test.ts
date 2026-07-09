// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import {
  formatRelativeTime,
  formatUserRole,
  formatUserStatus,
} from "./admin-presenters";

function keyedT(key, params = {}) {
  return params.count == null ? key : `${key}:${params.count}`;
}

test("user role and status labels use i18n keys", () => {
  assert.equal(formatUserRole("admin", keyedT), "admin.users.admin");
  assert.equal(formatUserRole("member", keyedT), "admin.users.member");
  assert.equal(formatUserStatus("active", keyedT), "admin.users.filter.active");
  assert.equal(formatUserStatus("suspended", keyedT), "admin.users.filter.suspended");
});

test("relative admin timestamps use i18n keys", () => {
  const originalNow = Date.now;
  Date.now = () => new Date("2026-07-09T12:00:00Z").getTime();
  try {
    assert.equal(formatRelativeTime(null, keyedT), "admin.relative.never");
    assert.equal(formatRelativeTime("2026-07-09T11:59:30Z", keyedT), "admin.relative.justNow");
    assert.equal(formatRelativeTime("2026-07-09T11:45:00Z", keyedT), "admin.relative.minutesAgo:15");
    assert.equal(formatRelativeTime("2026-07-09T09:00:00Z", keyedT), "admin.relative.hoursAgo:3");
    assert.equal(formatRelativeTime("2026-07-07T12:00:00Z", keyedT), "admin.relative.daysAgo:2");
  } finally {
    Date.now = originalNow;
  }
});
