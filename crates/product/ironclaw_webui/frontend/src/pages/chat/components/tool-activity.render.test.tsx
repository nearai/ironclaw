// @vitest-environment happy-dom
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, test } from "vitest";
import { ToolRun } from "./tool-activity";

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) root.unmount();
  document.body.innerHTML = "";
});

async function renderExpandedRun(tool) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);
  await act(async () => {
    root.render(<ToolRun tools={[tool]} />);
  });
  // The run header collapses by default; the card toggle sits inside it.
  for (const selector of [
    "button[aria-expanded='false']",
    "[data-testid='tool-activity-toggle'][aria-expanded='false']",
  ]) {
    const toggle = container.querySelector<HTMLButtonElement>(selector);
    if (toggle) {
      await act(async () => {
        toggle.click();
      });
    }
  }
  const panel = container.querySelector("[data-testid='tool-activity-detail']");
  assert.ok(panel, "the expanded card renders its detail panel");
  return Array.from(panel.querySelectorAll("button"), (button) =>
    button.textContent?.trim(),
  );
}

const completedTool = {
  id: "call-1",
  callId: "call-1",
  toolName: "http.get",
  toolStatus: "success",
  toolDetail: "GET https://example.test",
  toolParameters: '{"url":"https://example.test"}',
  toolResultPreview: '{"status":200,"body":"<a very large payload>"}',
  toolError: null,
  toolDurationMs: 12,
};

test("an expanded tool card offers the tool's details and inputs, never its output", async () => {
  const tabs = await renderExpandedRun(completedTool);
  assert.deepEqual(tabs, ["tool.tabDetails", "tool.tabParameters"]);
});

test("a failed tool card still surfaces its error beside the inputs", async () => {
  const tabs = await renderExpandedRun({
    ...completedTool,
    toolStatus: "error",
    toolError: "Invalid input: url — missing required field",
  });
  assert.deepEqual(tabs, ["tool.tabError", "tool.tabDetails", "tool.tabParameters"]);
});
