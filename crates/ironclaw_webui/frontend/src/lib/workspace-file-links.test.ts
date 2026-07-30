import assert from "node:assert/strict";
import { test } from "vitest";
import {
  workspaceFileHrefFromPath,
  workspaceFilePathFromHref,
  workspaceViewerRouteFromFilePath,
} from "./workspace-file-links";

test("workspaceFilePathFromHref recognizes scoped and sandbox workspace files", () => {
  assert.equal(
    workspaceFilePathFromHref("/workspace/reports/final.csv"),
    "/workspace/reports/final.csv",
  );
  assert.equal(
    workspaceFilePathFromHref("sandbox:/workspace/reports/final.csv"),
    "/workspace/reports/final.csv",
  );
  assert.equal(
    workspaceFilePathFromHref("/workspace/my%20report.md"),
    "/workspace/my report.md",
  );
  assert.equal(
    workspaceFilePathFromHref("/workspace/%E6%8A%A5%E5%91%8A.md"),
    "/workspace/报告.md",
  );
  assert.equal(
    workspaceFilePathFromHref("/workspace/Makefile"),
    "/workspace/Makefile",
  );
});

test("workspaceFilePathFromHref rejects non-file and unsafe link targets", () => {
  for (const href of [
    "https://example.com/workspace/report.csv",
    "file:///workspace/report.csv",
    "/workspace",
    "/workspace/../secret.txt",
    "/workspace/%2e%2e/secret.txt",
    "/workspace/reports%2Fsecret.txt",
    "/workspace/reports%5Csecret.txt",
    "/workspace/reports//final.csv",
    "/workspace/bad%00name.txt",
    "/workspace/report.csv?download=1",
    "/workspace/report.csv#preview",
    "/workspace/bad%encoding.txt",
    "/project/report.csv",
    "",
    null,
  ]) {
    assert.equal(workspaceFilePathFromHref(href), null, String(href));
  }
});

test("workspaceFileHrefFromPath encodes each validated path segment", () => {
  assert.equal(
    workspaceFileHrefFromPath("/workspace/报告/my report.md"),
    "/workspace/%E6%8A%A5%E5%91%8A/my%20report.md",
  );
  assert.equal(workspaceFileHrefFromPath("/workspace/../secret.txt"), null);
});

test("workspaceViewerRouteFromFilePath builds a selected-file SPA route", () => {
  assert.equal(
    workspaceViewerRouteFromFilePath(
      "/workspace/attachments/报告 final.md",
    ),
    "/workspace/workspace/attachments/%E6%8A%A5%E5%91%8A%20final.md",
  );
  assert.equal(
    workspaceViewerRouteFromFilePath("/workspace/../secret.txt"),
    null,
  );
});
