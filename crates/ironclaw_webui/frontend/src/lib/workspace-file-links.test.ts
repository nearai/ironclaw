import assert from "node:assert/strict";
import { test } from "vitest";
import { workspaceFilePathFromHref } from "./workspace-file-links";

test("workspaceFilePathFromHref recognizes scoped and sandbox workspace files", () => {
  assert.equal(
    workspaceFilePathFromHref("/workspace/reports/final.csv"),
    "/workspace/reports/final.csv",
  );
  assert.equal(
    workspaceFilePathFromHref("sandbox:/workspace/reports/final.csv"),
    "/workspace/reports/final.csv",
  );
});

test("workspaceFilePathFromHref rejects non-file and unsafe link targets", () => {
  for (const href of [
    "https://example.com/workspace/report.csv",
    "file:///workspace/report.csv",
    "/workspace",
    "/workspace/report",
    "/workspace/../secret.txt",
    "/workspace/reports//final.csv",
    "/project/report.csv",
    "",
    null,
  ]) {
    assert.equal(workspaceFilePathFromHref(href), null, String(href));
  }
});
