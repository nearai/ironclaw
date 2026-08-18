import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const PAGE_FILES = [
  "./jobs/jobs-page.tsx",
  "./projects/projects-page.tsx",
  "./workspace/workspace-page.tsx",
  "./extensions/extensions-page.tsx",
] as const;

for (const pageFile of PAGE_FILES) {
  test(`${pageFile} routes page feedback through InlineNotice`, () => {
    const source = readFileSync(new URL(pageFile, import.meta.url), "utf8");

    assert.match(source, /design-system\/inline-notice/);
    assert.match(source, /<InlineNotice/);
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
