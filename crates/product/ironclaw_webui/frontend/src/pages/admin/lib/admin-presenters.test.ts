// @ts-nocheck
import assert from "node:assert/strict";
import { test, vi } from "vitest";
import {
  adminUserActionErrorMessage,
  formatRelativeTime,
  formatUserRole,
  formatUserStatus,
} from "./admin-presenters";

function keyedT(key, params = {}) {
  return params.count == null ? key : `${key}:${params.count}`;
}

test("admin action errors localize the stable last-admin marker", () => {
  assert.equal(
    adminUserActionErrorMessage(
      { message: "Conflict (last_admin)", payload: { field: "last_admin" } },
      keyedT,
    ),
    "admin.users.lastAdminRequired",
  );
  assert.equal(
    adminUserActionErrorMessage({ message: "Service unavailable" }, (key, params) =>
      params?.message ? `${key}:${params.message}` : key),
    "admin.users.actionFailed:Service unavailable",
  );
});

test("user role and status labels use i18n keys", () => {
  assert.equal(formatUserRole("admin", keyedT), "admin.users.admin");
  assert.equal(formatUserRole("member", keyedT), "admin.users.member");
  assert.equal(formatUserStatus("active", keyedT), "admin.users.status.active");
  assert.equal(formatUserStatus("suspended", keyedT), "admin.users.status.suspended");
});

test("relative admin timestamps use i18n keys", () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-07-09T12:00:00Z"));
  try {
    assert.equal(formatRelativeTime(null, keyedT), "admin.relative.never");
    assert.equal(formatRelativeTime("not-a-date", keyedT), "admin.relative.never");
    assert.equal(formatRelativeTime("2026-07-09T11:59:30Z", keyedT), "admin.relative.justNow");
    assert.equal(formatRelativeTime("2026-07-09T11:45:00Z", keyedT), "admin.relative.minutesAgo:15");
    assert.equal(formatRelativeTime("2026-07-09T09:00:00Z", keyedT), "admin.relative.hoursAgo:3");
    assert.equal(formatRelativeTime("2026-07-07T12:00:00Z", keyedT), "admin.relative.daysAgo:2");
  } finally {
    vi.useRealTimers();
  }
});
