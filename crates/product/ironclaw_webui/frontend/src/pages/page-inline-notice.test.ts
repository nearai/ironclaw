import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const PAGE_FILES = [
  "./jobs/jobs-page.tsx",
  "./projects/projects-page.tsx",
  "./workspace/workspace-page.tsx",
  "./extensions/extensions-page.tsx",
] as const;

const SETTINGS_ADMIN_NOTICE_CONSUMERS = [
  ["./settings/settings-page.tsx", 1],
  ["./settings/components/settings-toolbar.tsx", 1],
  ["./settings/components/restart-banner.tsx", 3],
  ["./settings/components/skills-tab.tsx", 2],
  ["./settings/components/provider-management.tsx", 2],
  ["./settings/components/tools-tab.tsx", 2],
  ["./settings/components/trace-commons-tab.tsx", 3],
  ["./admin/components/configuration-tab.tsx", 3],
  ["./admin/components/users-tab.tsx", 2],
  ["./admin/components/user-detail.tsx", 2],
] as const;

for (const pageFile of PAGE_FILES) {
  test(`${pageFile} routes page feedback through InlineNotice`, () => {
    const source = readFileSync(new URL(pageFile, import.meta.url), "utf8");

    assert.match(source, /design-system\/inline-notice/);
    assert.match(source, /<InlineNotice/);
    assert.match(source, /role=(?:"(?:alert|status)"|\{)/);
  });
}

for (const [consumerFile, minimumNoticeCount] of SETTINGS_ADMIN_NOTICE_CONSUMERS) {
  test(`${consumerFile} routes page feedback through InlineNotice`, () => {
    const source = readFileSync(new URL(consumerFile, import.meta.url), "utf8");
    const noticeCount = source.match(/<InlineNotice\b/g)?.length ?? 0;

    assert.match(source, /design-system\/inline-notice/);
    assert.ok(
      noticeCount >= minimumNoticeCount,
      `expected at least ${minimumNoticeCount} InlineNotice consumers, found ${noticeCount}`,
    );
    assert.match(source, /role=(?:"(?:alert|status)"|\{)/);
  });
}

test("legacy page feedback components are retired", () => {
  for (const legacyFile of [
    "./projects/components/feedback-banner.tsx",
    "./extensions/components/action-toast.tsx",
  ]) {
    assert.throws(
      () => readFileSync(new URL(legacyFile, import.meta.url), "utf8"),
      /ENOENT/,
    );
  }
});
