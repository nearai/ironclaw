import assert from "node:assert/strict";
import { test } from "vitest";
import {
  isValidWorkspaceFilePath,
  workspaceFileHrefFromPath,
  workspaceFilePathFromHref,
  workspaceViewerRouteFromFilePath,
} from "./workspace-file-links";

test("isValidWorkspaceFilePath rejects paths outside the scoped root", () => {
  assert.equal(isValidWorkspaceFilePath("/workspace/reports/final.csv"), true);
  assert.equal(isValidWorkspaceFilePath("/workspace/reports/../secret.txt"), false);
  assert.equal(isValidWorkspaceFilePath("/project/reports/final.csv"), false);
  assert.equal(
    isValidWorkspaceFilePath(`/workspace/${"a".repeat(4_097)}.txt`),
    false,
  );
  assert.equal(
    isValidWorkspaceFilePath(
      `/workspace/${Array.from({ length: 65 }, () => "dir").join("/")}/file.txt`,
    ),
    false,
  );
});

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
    workspaceFilePathFromHref("/workspace/100%25-ready.txt"),
    "/workspace/100%-ready.txt",
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
    "/workspace/%252e%252e/secret.txt",
    "/workspace/reports%252Fsecret.txt",
    "/workspace/reports%2Fsecret.txt",
    "/workspace/reports%5Csecret.txt",
    "/workspace/reports//final.csv",
    "/workspace/bad%00name.txt",
    "/workspace/report.csv?download=1",
    "/workspace/report.csv#preview",
    "/workspace/bad%encoding.txt",
    `/workspace/${"a".repeat(8_193)}.txt`,
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
  assert.equal(
    workspaceFileHrefFromPath(`/workspace/${"报告".repeat(1_500)}.md`),
    null,
  );
  const percentPath = "/workspace/100%-ready.txt";
  const percentHref = workspaceFileHrefFromPath(percentPath);
  assert.equal(percentHref, "/workspace/100%25-ready.txt");
  assert.equal(workspaceFilePathFromHref(percentHref), percentPath);
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
