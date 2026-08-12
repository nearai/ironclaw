// @vitest-environment jsdom
import assert from "node:assert/strict";
import { test } from "vitest";
import { renderMarkdown } from "./markdown";

test("renderMarkdown only preserves workspace links for preview-enabled renderers", () => {
  const content =
    "[plain](/workspace/plain.txt) " +
    "[sandbox](sandbox:/workspace/sandbox.txt) " +
    "[encoded](sandbox:/workspace/%E6%8A%A5%E5%91%8A%20final.md)";
  const defaultOut = renderMarkdown(content);
  const previewOut = renderMarkdown(content, { workspaceFileLinks: true });
  const afterPreviewOut = renderMarkdown(
    "[sandbox](sandbox:/workspace/after.txt)",
  );

  assert.doesNotMatch(defaultOut, /data-workspace-path=/);
  assert.doesNotMatch(defaultOut, /href="\/workspace\/sandbox\.txt"/);
  assert.doesNotMatch(afterPreviewOut, /data-workspace-path=/);
  assert.doesNotMatch(afterPreviewOut, /href="\/workspace\/after\.txt"/);
  assert.match(
    previewOut,
    /<a href="\/workspace\/plain\.txt" data-workspace-path="\/workspace\/plain\.txt" target="_blank" rel="noopener noreferrer">plain<\/a>/,
  );
  assert.match(
    previewOut,
    /<a href="\/workspace\/sandbox\.txt" data-workspace-path="\/workspace\/sandbox\.txt" target="_blank" rel="noopener noreferrer">sandbox<\/a>/,
  );
  assert.match(
    previewOut,
    /href="\/workspace\/%E6%8A%A5%E5%91%8A%20final\.md" data-workspace-path="\/workspace\/报告 final\.md"/,
  );

  const forgedOut = renderMarkdown(
    '<a href="/workspace/approved.txt" data-workspace-path="/workspace/secret.txt">file</a> ' +
      '<a href="https://example.com" data-workspace-path="/workspace/secret.txt">external</a>',
    { workspaceFileLinks: true },
  );
  assert.match(
    forgedOut,
    /href="\/workspace\/approved\.txt" data-workspace-path="\/workspace\/approved\.txt"/,
  );
  assert.doesNotMatch(
    forgedOut,
    /data-workspace-path="\/workspace\/secret\.txt"/,
  );
});
