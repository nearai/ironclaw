import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { ConfirmDialog } from "./confirm-dialog";

test("ConfirmDialog renders an accessible destructive confirmation surface", () => {
  const html = renderToStaticMarkup(
    <ConfirmDialog
      open
      title="Delete chat"
      description="Delete this chat?"
      confirmLabel="Delete"
      cancelLabel="Cancel"
      isConfirming
      onConfirm={() => {}}
      onCancel={() => {}}
    />,
  );

  assert.match(html, /role="dialog"/);
  assert.match(html, /aria-modal="true"/);
  assert.match(html, /aria-label="Delete chat"/);
  assert.match(html, /Delete this chat\?/);
  assert.match(html, /data-testid="confirm-dialog-cancel"/);
  assert.match(html, /data-testid="confirm-dialog-confirm"/);
  assert.match(html, /aria-busy="true"/);
  assert.equal((html.match(/disabled=""/g) || []).length, 2);
  assert.doesNotMatch(html, /aria-label="Cancel"/);
});

test("ConfirmDialog renders nothing while closed", () => {
  const html = renderToStaticMarkup(
    <ConfirmDialog
      open={false}
      title="Delete chat"
      confirmLabel="Delete"
      onConfirm={() => {}}
      onCancel={() => {}}
    />,
  );

  assert.equal(html, "");
});
